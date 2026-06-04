//! LSP Timing and Coordination System
//!
//! Provides a sophisticated timing system that coordinates LSP initialization,
//! parsing phases, and prevents race conditions through proper sequencing
//! and synchronization.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, Semaphore};
use tokio::time::{interval, sleep, timeout};
use tracing::{debug, info, instrument};

/// Phase of the parsing pipeline
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParsingPhase {
    /// Initial phase - not started
    Idle = 0,
    /// LSP server initialization
    LspInitialization = 1,
    /// Syntax extraction phase
    SyntaxExtraction = 2,
    /// LSP semantic resolution phase
    LspResolution = 3,
    /// Graph construction phase
    GraphConstruction = 4,
    /// Persistence phase
    Persistence = 5,
    /// Completed successfully
    Completed = 6,
    /// Failed with error
    Failed = 7,
}

impl ParsingPhase {
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(ParsingPhase::Idle),
            1 => Some(ParsingPhase::LspInitialization),
            2 => Some(ParsingPhase::SyntaxExtraction),
            3 => Some(ParsingPhase::LspResolution),
            4 => Some(ParsingPhase::GraphConstruction),
            5 => Some(ParsingPhase::Persistence),
            6 => Some(ParsingPhase::Completed),
            7 => Some(ParsingPhase::Failed),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, ParsingPhase::Completed | ParsingPhase::Failed)
    }

    pub fn can_proceed_to(self, next_phase: ParsingPhase) -> bool {
        match (self, next_phase) {
            (ParsingPhase::Idle, ParsingPhase::LspInitialization) => true,
            (ParsingPhase::LspInitialization, ParsingPhase::SyntaxExtraction) => true,
            (ParsingPhase::SyntaxExtraction, ParsingPhase::LspResolution) => true,
            (ParsingPhase::LspResolution, ParsingPhase::GraphConstruction) => true,
            (ParsingPhase::GraphConstruction, ParsingPhase::Persistence) => true,
            (ParsingPhase::Persistence, ParsingPhase::Completed) => true,
            (_, ParsingPhase::Failed) => true, // Can fail from any phase
            _ => false,
        }
    }
}

/// Timing configuration for different phases
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingConfig {
    /// LSP initialization timeout
    pub lsp_init_timeout: Duration,
    /// Syntax extraction timeout
    pub syntax_extraction_timeout: Duration,
    /// LSP resolution timeout
    pub lsp_resolution_timeout: Duration,
    /// Graph construction timeout
    pub graph_construction_timeout: Duration,
    /// Persistence timeout
    pub persistence_timeout: Duration,
    /// Overall pipeline timeout
    pub pipeline_timeout: Duration,
    /// Health check interval
    pub health_check_interval: Duration,
    /// Retry delay for failed operations
    pub retry_delay: Duration,
    /// Maximum retry attempts
    pub max_retries: u32,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            lsp_init_timeout: Duration::from_secs(30),
            syntax_extraction_timeout: Duration::from_secs(60),
            lsp_resolution_timeout: Duration::from_secs(120),
            graph_construction_timeout: Duration::from_secs(30),
            persistence_timeout: Duration::from_secs(30),
            pipeline_timeout: Duration::from_secs(300), // 5 minutes total
            health_check_interval: Duration::from_secs(10),
            retry_delay: Duration::from_millis(1000),
            max_retries: 3,
        }
    }
}

/// Phase timing information
#[derive(Debug, Clone)]
pub struct PhaseTiming {
    pub phase: ParsingPhase,
    pub start_time: Option<Instant>,
    pub end_time: Option<Instant>,
    pub duration: Option<Duration>,
    pub retry_count: u32,
    pub success: bool,
}

impl PhaseTiming {
    pub fn new(phase: ParsingPhase) -> Self {
        Self {
            phase,
            start_time: None,
            end_time: None,
            duration: None,
            retry_count: 0,
            success: false,
        }
    }

    pub fn start(&mut self) {
        self.start_time = Some(Instant::now());
    }

    pub fn complete(&mut self, success: bool) {
        self.end_time = Some(Instant::now());
        self.success = success;
        if let Some(start) = self.start_time {
            self.duration = Some(self.end_time.unwrap().duration_since(start));
        }
    }

    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
    }
}

/// Pipeline timing coordinator
#[derive(Debug)]
pub struct TimingCoordinator {
    /// Current phase
    current_phase: Arc<AtomicU32>,
    /// Timing configuration
    config: Arc<TimingConfig>,
    /// Phase timings
    phase_timings: Arc<RwLock<Vec<PhaseTiming>>>,
    /// Pipeline start time
    pipeline_start: Arc<Mutex<Option<Instant>>>,
    /// Phase completion notification
    phase_complete: Arc<Notify>,
    /// Health check notification
    health_check: Arc<Notify>,
    /// Concurrency control
    concurrency_semaphore: Arc<Semaphore>,
    /// Pipeline completion flag
    pipeline_complete: Arc<AtomicBool>,
    /// Pipeline failure flag
    pipeline_failed: Arc<AtomicBool>,
}

impl TimingCoordinator {
    /// Create a new timing coordinator
    pub fn new(config: TimingConfig, max_concurrency: usize) -> Self {
        Self {
            current_phase: Arc::new(AtomicU32::new(ParsingPhase::Idle as u32)),
            config: Arc::new(config),
            phase_timings: Arc::new(RwLock::new(Vec::new())),
            pipeline_start: Arc::new(Mutex::new(None)),
            phase_complete: Arc::new(Notify::new()),
            health_check: Arc::new(Notify::new()),
            concurrency_semaphore: Arc::new(Semaphore::new(max_concurrency)),
            pipeline_complete: Arc::new(AtomicBool::new(false)),
            pipeline_failed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start the pipeline
    #[instrument(skip(self))]
    pub async fn start_pipeline(&self) -> Result<(), String> {
        if self.get_current_phase() != ParsingPhase::Idle {
            return Err("Pipeline already started".to_string());
        }

        // Record pipeline start time
        {
            let mut start = self.pipeline_start.lock().unwrap();
            *start = Some(Instant::now());
        }

        // Initialize phase timings
        {
            let mut timings = self.phase_timings.write().unwrap();
            timings.clear();
            for phase in [
                ParsingPhase::LspInitialization,
                ParsingPhase::SyntaxExtraction,
                ParsingPhase::LspResolution,
                ParsingPhase::GraphConstruction,
                ParsingPhase::Persistence,
            ] {
                timings.push(PhaseTiming::new(phase));
            }
        }

        // Start health monitoring
        self.start_health_monitoring().await;

        info!("Pipeline started with timing coordinator");
        Ok(())
    }

    /// Transition to the next phase
    #[instrument(skip(self))]
    pub async fn transition_to_phase(&self, next_phase: ParsingPhase) -> Result<(), String> {
        let current_phase = self.get_current_phase();

        if !current_phase.can_proceed_to(next_phase) {
            return Err(format!(
                "Invalid phase transition from {:?} to {:?}",
                current_phase, next_phase
            ));
        }

        // Complete current phase timing
        if current_phase != ParsingPhase::Idle {
            self.complete_phase_timing(current_phase, true).await;
        }

        // Start new phase timing
        self.start_phase_timing(next_phase).await;

        // Update current phase
        self.current_phase
            .store(next_phase.as_u32(), Ordering::SeqCst);

        info!("Transitioned to phase: {:?}", next_phase);

        // Notify phase completion
        self.phase_complete.notify_waiters();

        Ok(())
    }

    /// Wait for a specific phase to complete
    #[instrument(skip(self))]
    pub async fn wait_for_phase(&self, target_phase: ParsingPhase) -> Result<(), String> {
        let timeout_duration = self.get_phase_timeout(target_phase);

        let start_time = Instant::now();
        loop {
            let current_phase = self.get_current_phase();

            if current_phase == target_phase {
                return Ok(());
            }

            if current_phase.is_terminal() {
                return Err(format!("Pipeline terminated in phase {:?}", current_phase));
            }

            if start_time.elapsed() > timeout_duration {
                return Err(format!("Timeout waiting for phase {:?}", target_phase));
            }

            // Wait for phase change notification
            self.phase_complete.notified().await;
        }
    }

    /// Wait for phase completion with timeout
    #[instrument(skip(self))]
    pub async fn wait_for_phase_completion(&self, phase: ParsingPhase) -> Result<(), String> {
        let timeout_duration = self.get_phase_timeout(phase);

        match timeout(timeout_duration, self.wait_for_phase(phase)).await {
            Ok(result) => result,
            Err(_) => Err(format!("Timeout waiting for phase {:?} completion", phase)),
        }
    }

    /// Execute a phase with proper timing and error handling
    #[instrument(skip(self, phase_fn))]
    pub async fn execute_phase<F, Fut, T>(
        &self,
        phase: ParsingPhase,
        mut phase_fn: F,
    ) -> Result<T, String>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, String>>,
    {
        // Acquire concurrency permit
        let _permit = self
            .concurrency_semaphore
            .acquire()
            .await
            .map_err(|e| format!("Failed to acquire concurrency permit: {}", e))?;

        // Transition to phase
        self.transition_to_phase(phase).await?;

        // Execute phase with retry logic
        let mut last_error = String::new();
        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                info!("Retrying phase {:?}, attempt {}", phase, attempt + 1);
                sleep(self.config.retry_delay).await;
                self.increment_phase_retry(phase).await;
            }

            match timeout(self.get_phase_timeout(phase), phase_fn()).await {
                Ok(Ok(result)) => {
                    self.complete_phase_timing(phase, true).await;
                    return Ok(result);
                }
                Ok(Err(e)) => {
                    last_error = e;
                    if attempt < self.config.max_retries {
                        continue;
                    }
                }
                Err(_) => {
                    last_error = format!("Phase {:?} timed out", phase);
                    if attempt < self.config.max_retries {
                        continue;
                    }
                }
            }
        }

        // Mark phase as failed
        self.complete_phase_timing(phase, false).await;
        self.transition_to_phase(ParsingPhase::Failed).await?;

        Err(format!(
            "Phase {:?} failed after {} attempts: {}",
            phase,
            self.config.max_retries + 1,
            last_error
        ))
    }

    /// Start timing for a phase
    async fn start_phase_timing(&self, phase: ParsingPhase) {
        let mut timings = self.phase_timings.write().unwrap();
        if let Some(timing) = timings.iter_mut().find(|t| t.phase == phase) {
            timing.start();
        }
    }

    /// Complete timing for a phase
    async fn complete_phase_timing(&self, phase: ParsingPhase, success: bool) {
        let mut timings = self.phase_timings.write().unwrap();
        if let Some(timing) = timings.iter_mut().find(|t| t.phase == phase) {
            timing.complete(success);
        }
    }

    /// Increment retry count for a phase
    async fn increment_phase_retry(&self, phase: ParsingPhase) {
        let mut timings = self.phase_timings.write().unwrap();
        if let Some(timing) = timings.iter_mut().find(|t| t.phase == phase) {
            timing.increment_retry();
        }
    }

    /// Get timeout duration for a phase
    fn get_phase_timeout(&self, phase: ParsingPhase) -> Duration {
        match phase {
            ParsingPhase::LspInitialization => self.config.lsp_init_timeout,
            ParsingPhase::SyntaxExtraction => self.config.syntax_extraction_timeout,
            ParsingPhase::LspResolution => self.config.lsp_resolution_timeout,
            ParsingPhase::GraphConstruction => self.config.graph_construction_timeout,
            ParsingPhase::Persistence => self.config.persistence_timeout,
            _ => Duration::from_secs(10), // Default timeout
        }
    }

    /// Get current phase
    pub fn get_current_phase(&self) -> ParsingPhase {
        let phase_value = self.current_phase.load(Ordering::SeqCst);
        ParsingPhase::from_u32(phase_value).unwrap_or(ParsingPhase::Failed)
    }

    /// Check if pipeline is complete
    pub fn is_pipeline_complete(&self) -> bool {
        self.pipeline_complete.load(Ordering::SeqCst)
    }

    /// Check if pipeline has failed
    pub fn is_pipeline_failed(&self) -> bool {
        self.pipeline_failed.load(Ordering::SeqCst)
    }

    /// Mark pipeline as complete
    pub async fn mark_pipeline_complete(&self) {
        self.pipeline_complete.store(true, Ordering::SeqCst);
        self.transition_to_phase(ParsingPhase::Completed)
            .await
            .unwrap_or_default();
    }

    /// Mark pipeline as failed
    pub async fn mark_pipeline_failed(&self) {
        self.pipeline_failed.store(true, Ordering::SeqCst);
        self.transition_to_phase(ParsingPhase::Failed)
            .await
            .unwrap_or_default();
    }

    /// Get phase timings
    pub fn get_phase_timings(&self) -> Vec<PhaseTiming> {
        self.phase_timings.read().unwrap().clone()
    }

    /// Get pipeline duration
    pub fn get_pipeline_duration(&self) -> Option<Duration> {
        (*self.pipeline_start.lock().unwrap()).map(|start| Instant::now().duration_since(start))
    }

    /// Start health monitoring
    async fn start_health_monitoring(&self) {
        let health_check = Arc::clone(&self.health_check);
        let health_interval = self.config.health_check_interval;
        let current_phase = Arc::clone(&self.current_phase);

        tokio::spawn(async move {
            let mut interval = interval(health_interval);

            loop {
                interval.tick().await;

                let phase = ParsingPhase::from_u32(current_phase.load(Ordering::SeqCst))
                    .unwrap_or(ParsingPhase::Failed);

                if phase.is_terminal() {
                    break;
                }

                // Perform health check
                debug!("Health check for phase: {:?}", phase);
                health_check.notify_waiters();
            }
        });
    }

    /// Wait for health check
    pub async fn wait_for_health_check(&self) {
        self.health_check.notified().await;
    }

    /// Get timing statistics
    pub fn get_timing_stats(&self) -> TimingStats {
        let timings = self.get_phase_timings();
        let total_duration = self.get_pipeline_duration();

        let mut total_phase_duration = Duration::ZERO;
        let mut successful_phases = 0;
        let mut failed_phases = 0;
        let mut total_retries = 0;

        for timing in &timings {
            if let Some(duration) = timing.duration {
                total_phase_duration += duration;
            }

            if timing.success {
                successful_phases += 1;
            } else {
                failed_phases += 1;
            }

            total_retries += timing.retry_count;
        }

        TimingStats {
            total_duration,
            total_phase_duration,
            successful_phases,
            failed_phases,
            total_retries,
            phase_timings: timings,
        }
    }
}

/// Timing statistics
#[derive(Debug, Clone)]
pub struct TimingStats {
    pub total_duration: Option<Duration>,
    pub total_phase_duration: Duration,
    pub successful_phases: usize,
    pub failed_phases: usize,
    pub total_retries: u32,
    pub phase_timings: Vec<PhaseTiming>,
}

impl TimingStats {
    /// Get success rate
    pub fn success_rate(&self) -> f64 {
        let total = self.successful_phases + self.failed_phases;
        if total == 0 {
            0.0
        } else {
            self.successful_phases as f64 / total as f64
        }
    }

    /// Get average phase duration
    pub fn average_phase_duration(&self) -> Duration {
        let total_phases = self.successful_phases + self.failed_phases;
        if total_phases == 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos(self.total_phase_duration.as_nanos() as u64 / total_phases as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_phase_transitions() {
        let config = TimingConfig::default();
        let coordinator = TimingCoordinator::new(config, 1);

        assert_eq!(coordinator.get_current_phase(), ParsingPhase::Idle);

        coordinator.start_pipeline().await.unwrap();
        coordinator
            .transition_to_phase(ParsingPhase::LspInitialization)
            .await
            .unwrap();
        assert_eq!(
            coordinator.get_current_phase(),
            ParsingPhase::LspInitialization
        );

        coordinator
            .transition_to_phase(ParsingPhase::SyntaxExtraction)
            .await
            .unwrap();
        assert_eq!(
            coordinator.get_current_phase(),
            ParsingPhase::SyntaxExtraction
        );
    }

    #[tokio::test]
    async fn test_phase_execution() {
        let config = TimingConfig::default();
        let coordinator = TimingCoordinator::new(config, 1);

        coordinator.start_pipeline().await.unwrap();

        let result = coordinator
            .execute_phase(ParsingPhase::LspInitialization, || async {
                Ok::<String, String>("success".to_string())
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }

    #[test]
    fn test_phase_validation() {
        assert!(ParsingPhase::Idle.can_proceed_to(ParsingPhase::LspInitialization));
        assert!(ParsingPhase::LspInitialization.can_proceed_to(ParsingPhase::SyntaxExtraction));
        assert!(!ParsingPhase::Idle.can_proceed_to(ParsingPhase::SyntaxExtraction));
        assert!(ParsingPhase::LspInitialization.can_proceed_to(ParsingPhase::Failed));
    }
}
