//! LSP session management and supervision
//!
//! Provides a robust session supervisor that manages LSP server lifecycle,
//! health monitoring, and automatic restarts. Uses RwLock to allow concurrent
//! read-only operations (definition lookups, hover) while serializing lifecycle
//! operations (start, stop, restart).

use crate::infrastructure::config::LanguageConfig;
use crate::infrastructure::lsp::column_utils::utf16_column_for_file;
use crate::infrastructure::lsp::def_trace::{self, DefOutcome};
use crate::infrastructure::lsp::errors::LspError;
use crate::infrastructure::lsp::notification_sink::{
    lsp_verbose, IndexingProgress, LspNotificationSink,
};
use crate::infrastructure::lsp::protocol::WorkspaceFolder;
use crate::infrastructure::lsp::simple_client::SimpleLspClient;
use serde_json::Value;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, warn};
use url::Url;

/// LSP session state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Idle = 0,
    Starting = 1,
    Ready = 2,
    Degraded = 3,
    Failed = 4,
}

impl SessionState {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(SessionState::Idle),
            1 => Some(SessionState::Starting),
            2 => Some(SessionState::Ready),
            3 => Some(SessionState::Degraded),
            4 => Some(SessionState::Failed),
            _ => None,
        }
    }

    pub fn is_functional(self) -> bool {
        matches!(self, SessionState::Ready | SessionState::Degraded)
    }

    pub fn is_ready(self) -> bool {
        self == SessionState::Ready
    }

    pub fn can_accept_requests(self) -> bool {
        self == SessionState::Ready
    }
}

/// LSP session supervisor that manages server lifecycle.
///
/// Uses `RwLock` so concurrent definition/hover lookups share a read lock
/// while lifecycle operations (init, shutdown) take an exclusive write lock.
pub struct SessionSupervisor {
    state: Arc<AtomicU8>,
    lsp_client: Arc<RwLock<Option<SimpleLspClient>>>,
    retry_budget: Arc<AtomicU32>,
    health_interval: Duration,
    config: Arc<LanguageConfig>,
    workspace_root: Option<Url>,
    /// F.2: explicit workspace folders to advertise at `initialize`
    /// time. Populated from SFDX `packageDirectories` for Apex so
    /// jorje indexes every declared source root (not just the
    /// default one). Empty for languages where the single-root LSP
    /// semantics are sufficient.
    workspace_folders: Vec<WorkspaceFolder>,
    /// F.2: how this language signals "indexing complete". Default
    /// is `Immediate` for backwards compatibility with every
    /// currently-working language; Apex uses
    /// `ProgressAndProbe` to wait on jorje's `$/progress` stream
    /// and a `documentSymbol` canary probe.
    readiness: ReadinessStrategy,
    security_config: SecurityConfig,
    metrics: Arc<tokio::sync::Mutex<SessionMetrics>>,
    /// Observability sink shared with the `SimpleLspClient`. Counts
    /// and the latest indexing-progress snapshot live here so F.1 can
    /// route notifications and stderr through a structured path instead
    /// of dropping them in `process_messages`.
    observability: Arc<NotificationState>,
}

/// How the supervisor decides the server is ready for work.
///
/// * `Immediate` — flip to `Ready` as soon as the `initialize`
///   response arrives. Correct for rust-analyzer, jdtls, pyright,
///   gopls, ts-server, etc. — they all respond to requests before
///   the full workspace is indexed, and returning "not found" from
///   an early request is an acceptable failure mode in those
///   ecosystems.
/// * `ProgressAndProbe { canary_file, deadline }` — wait for either
///   (a) a `$/progress` `end` frame, (b) a successful `documentSymbol`
///   probe on the canary file, or (c) a "quiet period" (no new
///   notifications for several seconds after at least one signal).
///   Used for Apex (jorje), which accepts `textDocument/definition`
///   before indexing completes but always returns `null` in that
///   window — so we must not flip to `Ready` prematurely or the
///   whole scan falls through to heuristic resolution.
#[derive(Debug, Clone, Default)]
pub enum ReadinessStrategy {
    #[default]
    Immediate,
    ProgressAndProbe {
        /// `file://` URI of a real Apex class we'll `documentSymbol`
        /// against as a liveness probe. Chosen by the supervisor
        /// owner (typically the orchestrator picks the first `.cls`
        /// under the primary package dir).
        canary_file: Option<String>,
        /// How long we'll wait in total before giving up and
        /// flipping to `Degraded` (not `Failed`) so heuristic
        /// resolution still runs.
        deadline: Duration,
        /// How long the server has to be silent after we've seen at
        /// least one signal before we decide indexing is "probably
        /// done". Prevents a single `$/progress report` (no `end`)
        /// from stalling the scan indefinitely.
        quiet_period: Duration,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadinessOutcome {
    /// One of the strategy's exit conditions fired and the server
    /// should be treated as ready.
    Ready,
    /// The deadline elapsed without any exit condition firing. The
    /// supervisor still publishes the session as `Ready` (so the
    /// scan proceeds) but records a warning so the degraded path is
    /// observable in logs and telemetry.
    DegradedTimeout,
}

use crate::infrastructure::lsp::security::SecurityConfig;

/// Runtime statistics for LSP session lifecycle.
///
/// Sprint F.1 extends this with three observability counters and one
/// opaque state snapshot:
///
/// * `notifications_received` — total count of JSON-RPC notifications
///   observed across all restarts. Useful to prove jorje is speaking
///   at all; a `successful_starts > 0` paired with
///   `notifications_received == 0` is the classic "LSP handshake
///   completed but server went silent" failure mode.
/// * `stderr_lines_observed` — total stderr line count. Jorje logs
///   init + errors to stderr; if this stays at 0 after start, the
///   child's stderr pipe wasn't wired up correctly.
/// * `indexing_messages_seen` — subset of notifications classified
///   as progress/indexing signals (`$/progress`, `window/logMessage`
///   with indexing content, jorje-specific `apex/*`). Used by F.2's
///   readiness barrier as a "server has begun real work" proxy.
/// * `last_indexing_progress` — the most recent `$/progress` snapshot
///   seen, if any. Cloned out of the notification state at
///   `metrics()` time so callers don't reach into supervisor
///   internals.
#[derive(Debug, Clone, Default)]
pub struct SessionMetrics {
    pub start_attempts: u64,
    pub successful_starts: u64,
    pub failed_starts: u64,
    pub last_error: Option<String>,
    pub notifications_received: u64,
    pub stderr_lines_observed: u64,
    pub indexing_messages_seen: u64,
    pub last_indexing_progress: Option<IndexingProgress>,
}

/// Cross-task observability state shared between the supervisor (who
/// owns the metrics mutex and readiness logic) and the LSP client
/// reader task (which fires `record_notification` /
/// `record_stderr_line` inline).
///
/// This struct is deliberately synchronous and lock-light: the client
/// reader is on a tokio task, but we don't want to take an `async`
/// mutex for every stderr line. `Mutex<_>` is fine here — all
/// critical sections are O(1) writes.
struct NotificationState {
    last_notification_at: Mutex<Option<Instant>>,
    indexing_progress: Mutex<Option<IndexingProgress>>,
    notifications_received: std::sync::atomic::AtomicU64,
    stderr_lines_observed: std::sync::atomic::AtomicU64,
    indexing_messages_seen: std::sync::atomic::AtomicU64,
}

impl NotificationState {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            last_notification_at: Mutex::new(None),
            indexing_progress: Mutex::new(None),
            notifications_received: std::sync::atomic::AtomicU64::new(0),
            stderr_lines_observed: std::sync::atomic::AtomicU64::new(0),
            indexing_messages_seen: std::sync::atomic::AtomicU64::new(0),
        })
    }

    fn snapshot_indexing(&self) -> Option<IndexingProgress> {
        self.indexing_progress.lock().ok().and_then(|g| g.clone())
    }

    /// Copy counters into the provided `SessionMetrics` so
    /// `metrics()` returns a single, consistent view to callers.
    fn fill_metrics(&self, metrics: &mut SessionMetrics) {
        use std::sync::atomic::Ordering::Relaxed;
        metrics.notifications_received = self.notifications_received.load(Relaxed);
        metrics.stderr_lines_observed = self.stderr_lines_observed.load(Relaxed);
        metrics.indexing_messages_seen = self.indexing_messages_seen.load(Relaxed);
        metrics.last_indexing_progress = self.snapshot_indexing();
    }
}

impl LspNotificationSink for NotificationState {
    fn record_notification(&self, method: &str, params: Option<&Value>) {
        use std::sync::atomic::Ordering::Relaxed;
        self.notifications_received.fetch_add(1, Relaxed);
        if let Ok(mut last) = self.last_notification_at.lock() {
            *last = Some(Instant::now());
        }

        // Classify indexing-flavoured frames so the readiness barrier
        // can distinguish "server is thinking" from "server is
        // chattering". Everything that looks like progress or a
        // window log about indexing counts.
        let mentions_indexing = params
            .and_then(|p| p.get("message").and_then(|m| m.as_str()))
            .map(|m| m.to_ascii_lowercase().contains("index"))
            .unwrap_or(false);

        let is_indexing_frame = method == "$/progress"
            || method.starts_with("apex/")
            || (method == "window/logMessage" && mentions_indexing);

        if is_indexing_frame {
            self.indexing_messages_seen.fetch_add(1, Relaxed);
        }

        if method == "$/progress" {
            if let Some(params) = params {
                if let Some(progress) = parse_progress(params) {
                    if let Ok(mut cur) = self.indexing_progress.lock() {
                        *cur = Some(progress.clone());
                    }
                    if lsp_verbose() {
                        info!(
                            token = %progress.token,
                            pct = ?progress.percentage,
                            message = ?progress.message,
                            finished = progress.finished,
                            "LSP $/progress"
                        );
                    } else {
                        debug!(
                            token = %progress.token,
                            finished = progress.finished,
                            "LSP $/progress"
                        );
                    }
                    return;
                }
            }
        }

        if lsp_verbose() {
            info!(method, "LSP notification");
        } else {
            debug!(method, "LSP notification");
        }
    }

    fn record_stderr_line(&self, line: &str) {
        use std::sync::atomic::Ordering::Relaxed;
        self.stderr_lines_observed.fetch_add(1, Relaxed);

        if lsp_verbose() {
            info!(target: "graphengine::lsp::stderr", "{}", line);
        } else {
            debug!(target: "graphengine::lsp::stderr", "{}", line);
        }
    }
}

/// Extract an `IndexingProgress` from a `$/progress` frame. Jorje
/// uses the standard `WorkDoneProgress` shape: params is
/// `{ token, value: { kind, title?, message?, percentage?, … } }`.
fn parse_progress(params: &Value) -> Option<IndexingProgress> {
    let token = params.get("token").map(|t| match t {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    })?;
    let value = params.get("value")?;
    let kind = value
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let message = value
        .get("message")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let percentage = value
        .get("percentage")
        .and_then(|v| v.as_u64())
        .and_then(|n| u8::try_from(n.min(100)).ok());
    let finished = kind == "end";
    Some(IndexingProgress {
        token,
        percentage,
        message,
        finished,
    })
}

impl SessionSupervisor {
    pub fn new(config: LanguageConfig, workspace_root: Option<Url>) -> Self {
        Self {
            state: Arc::new(AtomicU8::new(SessionState::Idle as u8)),
            lsp_client: Arc::new(RwLock::new(None)),
            retry_budget: Arc::new(AtomicU32::new(3)),
            health_interval: Duration::from_secs(30),
            config: Arc::new(config),
            workspace_root,
            workspace_folders: Vec::new(),
            readiness: ReadinessStrategy::default(),
            security_config: SecurityConfig::default(),
            metrics: Arc::new(tokio::sync::Mutex::new(SessionMetrics::default())),
            observability: NotificationState::new(),
        }
    }

    pub fn with_security(
        config: LanguageConfig,
        workspace_root: Option<Url>,
        security_config: SecurityConfig,
    ) -> Self {
        Self {
            state: Arc::new(AtomicU8::new(SessionState::Idle as u8)),
            lsp_client: Arc::new(RwLock::new(None)),
            retry_budget: Arc::new(AtomicU32::new(3)),
            health_interval: Duration::from_secs(30),
            config: Arc::new(config),
            workspace_root,
            workspace_folders: Vec::new(),
            readiness: ReadinessStrategy::default(),
            security_config,
            metrics: Arc::new(tokio::sync::Mutex::new(SessionMetrics::default())),
            observability: NotificationState::new(),
        }
    }

    /// Install workspace folders (e.g. SFDX `packageDirectories`).
    /// Call before [`initialize`](Self::initialize). Clears any
    /// previously-set folder list.
    pub fn set_workspace_folders(&mut self, folders: Vec<WorkspaceFolder>) {
        self.workspace_folders = folders;
    }

    /// Install a readiness strategy. `Immediate` is the default and
    /// matches pre-F.2 behaviour for every non-Apex language.
    pub fn set_readiness_strategy(&mut self, strategy: ReadinessStrategy) {
        self.readiness = strategy;
    }

    /// Inspect the current strategy (tests + orchestrator use this
    /// to log what readiness path is active).
    pub fn readiness_strategy(&self) -> &ReadinessStrategy {
        &self.readiness
    }

    /// Expose the observability sink so callers that construct a
    /// `SimpleLspClient` outside `initialize()` (tests, integration
    /// harnesses) can still route notifications back here. Returns a
    /// `dyn` handle so consumers don't depend on the concrete
    /// `NotificationState` type.
    pub fn notification_sink(&self) -> Arc<dyn LspNotificationSink> {
        Arc::clone(&self.observability) as Arc<dyn LspNotificationSink>
    }

    /// Most recent indexing progress snapshot, if the server has
    /// emitted a `$/progress` begin/report since start. Used by F.2's
    /// readiness wait.
    pub fn indexing_progress(&self) -> Option<IndexingProgress> {
        self.observability.snapshot_indexing()
    }

    /// `Instant` of the last notification (of any kind) received from
    /// the server. `None` until the server has said anything.
    pub fn last_notification_at(&self) -> Option<Instant> {
        self.observability
            .last_notification_at
            .lock()
            .ok()
            .and_then(|g| *g)
    }

    #[instrument(skip(self), level = "trace")]
    pub async fn acquire(&self) -> Result<(), LspError> {
        loop {
            let current_state = self.get_state();

            match current_state {
                SessionState::Ready => {
                    info!("LSP server is ready");
                    return Ok(());
                }
                SessionState::Failed => {
                    error!("LSP server is in failed state");
                    return Err(LspError::server_crashed("LSP server failed"));
                }
                SessionState::Idle => {
                    info!("Starting LSP server");
                    self.start().await?;
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    pub fn get_state(&self) -> SessionState {
        let state_value = self.state.load(Ordering::SeqCst);
        SessionState::from_u8(state_value).unwrap_or(SessionState::Failed)
    }

    fn set_state(&self, new_state: SessionState) {
        self.state.store(new_state.as_u8(), Ordering::SeqCst);
        debug!("LSP session state changed to: {:?}", new_state);
    }

    #[instrument(skip(self), level = "trace")]
    async fn start(&self) -> Result<(), LspError> {
        self.initialize().await
    }

    #[instrument(skip(self), level = "trace")]
    pub async fn is_healthy(&self) -> bool {
        if !self.get_state().is_functional() {
            return false;
        }

        let client_guard = self.lsp_client.read().await;
        client_guard.is_some()
    }

    #[instrument(skip(self), level = "trace")]
    pub async fn restart(&self) -> Result<(), LspError> {
        info!("Restarting LSP server");
        self.kill().await;
        self.set_state(SessionState::Idle);
        self.start().await
    }

    #[instrument(skip(self), level = "trace")]
    pub async fn kill(&self) {
        let client = {
            let mut client_guard = self.lsp_client.write().await;
            client_guard.take()
        };

        if let Some(mut client) = client {
            if let Err(e) = client.shutdown().await {
                warn!("Failed to shutdown LSP client: {:?}", e);
            }
        }
    }

    pub fn reset_retry_budget(&self) {
        self.retry_budget.store(3, Ordering::SeqCst);
    }

    /// Current number of restart attempts the supervisor still has
    /// budgeted for this scan. Exposed for diagnostics / tests and for
    /// Sprint D.2's `ResolutionDegraded` finding to correlate restarts
    /// against fallback rate.
    pub fn remaining_retry_budget(&self) -> u32 {
        self.retry_budget.load(Ordering::Acquire)
    }

    /// Probe the LSP client once and, if it's dead, try to recover.
    ///
    /// Returns the supervisor's state after the probe. This is the
    /// atomic unit that [`spawn_health_probe`](Self::spawn_health_probe)
    /// repeats on a timer; isolating a single tick as its own method
    /// keeps it unit-testable without spinning up `tokio::time`.
    ///
    /// Transitions (assuming a functional starting state):
    ///
    /// * client alive → state left untouched (stays `Ready` /
    ///   `Degraded` as the caller set it).
    /// * client dead + restart succeeds → state cycles `Ready →
    ///   Degraded → Ready` inside
    ///   [`try_recover_from_crash`](Self::try_recover_from_crash).
    /// * client dead + restart fails → state becomes `Failed`.
    /// * no client yet (pre-initialize) → state left untouched; the
    ///   probe is a no-op until `initialize` has run.
    ///
    /// This deliberately does **not** issue an LSP request as a
    /// heartbeat. The `SimpleLspClient::is_alive()` flag is driven by
    /// the reader task observing EOF / read errors on the child's
    /// stdout (see D.1). That signal is already the most honest
    /// liveness indicator available — sending an extra request would
    /// trade real liveness data for synthetic latency + a race window
    /// where the request races against the reader's EOF detection.
    pub async fn probe_once(&self) -> SessionState {
        let current = self.get_state();
        if !current.is_functional() {
            return current;
        }

        let client_alive = {
            let guard = self.lsp_client.read().await;
            match guard.as_ref() {
                Some(client) => client.is_alive(),
                None => {
                    // No client: supervisor may be mid-startup or
                    // post-shutdown. Neither state wants probe-driven
                    // interference.
                    return current;
                }
            }
        };

        if client_alive {
            return current;
        }

        warn!(
            state = ?current,
            "Health probe detected dead LSP client — triggering bounded restart"
        );
        let _ = self
            .try_recover_from_crash(&LspError::server_crashed(
                "health probe observed dead LSP client",
            ))
            .await;
        self.get_state()
    }

    /// Spawn a background task that runs [`probe_once`](Self::probe_once)
    /// every `health_interval` until the supervisor enters `Failed`
    /// (budget exhausted, unrecoverable) or `Idle` (explicit shutdown).
    ///
    /// The task holds only `Arc` clones of the supervisor's internal
    /// state, so when the outer `SessionSupervisor` is dropped the
    /// clones keep the probe alive long enough to observe the state
    /// flip to `Failed`/`Idle` via [`Drop`], after which the probe
    /// exits cleanly on its next tick.
    ///
    /// Returns the [`tokio::task::JoinHandle`] so callers (the
    /// orchestrator) can `abort()` it during controlled shutdown.
    /// Typical lifetime: one probe per scan, spawned immediately after
    /// `initialize()` completes.
    pub fn spawn_health_probe(&self) -> tokio::task::JoinHandle<()> {
        let supervisor = self.clone();
        let interval = self.health_interval;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // We don't care about catching up on missed ticks — if the
            // system was pausing the task, piling up a burst of probes
            // just after resumption would be pure noise.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // Drop the first (immediate) tick so we don't probe a
            // still-initializing session at t=0.
            ticker.tick().await;

            loop {
                ticker.tick().await;

                let state = supervisor.get_state();
                if matches!(state, SessionState::Failed | SessionState::Idle) {
                    debug!(
                        ?state,
                        "Health probe exiting — supervisor is no longer functional"
                    );
                    break;
                }

                let observed = supervisor.probe_once().await;
                debug!(?observed, "Health probe tick complete");
            }
        })
    }

    /// If `err` represents the LSP server dying (EOF on stdout, child
    /// exit, etc.), attempt a single bounded restart using
    /// `retry_budget`. Returns `true` iff the restart succeeded and
    /// the caller should retry the originating operation. On `false`
    /// the supervisor is in one of:
    ///   * `Failed` — budget exhausted or restart itself failed; the
    ///     original error should propagate and the scan should fall
    ///     back to the heuristic path.
    ///   * Whatever terminal state `err` implied — if the error was
    ///     not crash-shaped we return `false` without touching state
    ///     so normal error handling proceeds.
    ///
    /// This is deliberately conservative:
    ///   - Only `ServerCrashed` errors trigger recovery. Timeouts are
    ///     recoverable at the request level, not the session level,
    ///     and we don't want a single slow request to burn a restart.
    ///   - The budget is per-supervisor (per-scan), not per-request,
    ///     so a genuinely unhealthy server eventually exits to
    ///     `Failed` instead of flapping forever.
    ///   - During a restart the session goes `Degraded`; concurrent
    ///     requests on other tasks observe `Degraded` via
    ///     `get_state()` and can short-circuit to heuristic fallback
    ///     without queueing on the write lock.
    async fn try_recover_from_crash(&self, err: &LspError) -> bool {
        if !matches!(err, LspError::ServerCrashed(_)) {
            return false;
        }

        let claimed = self
            .retry_budget
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |b| {
                if b == 0 {
                    None
                } else {
                    Some(b - 1)
                }
            });

        let remaining_after = match claimed {
            Ok(prev) => prev.saturating_sub(1),
            Err(_) => {
                error!(
                    "LSP retry budget exhausted — cannot restart after crash: {}",
                    err
                );
                self.set_state(SessionState::Failed);
                let mut metrics = self.metrics.lock().await;
                metrics.last_error = Some(format!("retry budget exhausted: {err}"));
                return false;
            }
        };

        warn!(
            remaining_restarts = remaining_after,
            error = %err,
            "LSP server crashed — attempting bounded restart"
        );
        self.set_state(SessionState::Degraded);

        match self.restart().await {
            Ok(()) => {
                info!(
                    remaining_restarts = remaining_after,
                    "LSP server successfully restarted after crash"
                );
                true
            }
            Err(restart_err) => {
                error!(
                    remaining_restarts = remaining_after,
                    error = %restart_err,
                    "LSP restart failed after crash — marking Failed"
                );
                self.set_state(SessionState::Failed);
                let mut metrics = self.metrics.lock().await;
                metrics.last_error = Some(format!("restart after crash failed: {restart_err}"));
                false
            }
        }
    }
}

impl Clone for SessionSupervisor {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            lsp_client: Arc::clone(&self.lsp_client),
            retry_budget: Arc::clone(&self.retry_budget),
            health_interval: self.health_interval,
            config: Arc::clone(&self.config),
            workspace_root: self.workspace_root.clone(),
            workspace_folders: self.workspace_folders.clone(),
            readiness: self.readiness.clone(),
            security_config: self.security_config.clone(),
            metrics: Arc::clone(&self.metrics),
            observability: Arc::clone(&self.observability),
        }
    }
}

impl SessionSupervisor {
    pub async fn is_ready(&self) -> bool {
        let state = SessionState::from_u8(self.state.load(Ordering::Acquire))
            .unwrap_or(SessionState::Failed);
        state.is_ready()
    }

    pub async fn wait_until_ready(&self, timeout: Duration) -> Result<(), LspError> {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(100);

        loop {
            if self.is_ready().await {
                return Ok(());
            }

            if start.elapsed() >= timeout {
                return Err(LspError::ProtocolError(format!(
                    "LSP session did not become ready within {}ms",
                    timeout.as_millis()
                )));
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Initialize the LSP session (takes write lock for lifecycle mutation)
    pub async fn initialize(&self) -> Result<(), LspError> {
        let current_state = self.get_state();
        if current_state == SessionState::Ready {
            return Ok(());
        }

        if current_state == SessionState::Starting {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                match self.get_state() {
                    SessionState::Ready => return Ok(()),
                    SessionState::Starting => continue,
                    SessionState::Failed => break,
                    _ => break,
                }
            }
        }

        info!(
            "Initializing LSP session for language: {}",
            self.config.language
        );

        self.set_state(SessionState::Starting);
        {
            let mut metrics = self.metrics.lock().await;
            metrics.start_attempts += 1;
        }

        let mut client = SimpleLspClient::new((*self.config).clone());
        client.set_notification_sink(self.notification_sink());
        if !self.workspace_folders.is_empty() {
            client.set_workspace_folders(self.workspace_folders.clone());
        }
        let workspace_root = self.effective_workspace_root();
        match client.start_server(workspace_root.clone()).await {
            Ok(_) => {
                let mut metrics = self.metrics.lock().await;
                metrics.successful_starts += 1;
                metrics.last_error = None;
            }
            Err(err) => {
                self.set_state(SessionState::Failed);
                let mut metrics = self.metrics.lock().await;
                metrics.failed_starts += 1;
                metrics.last_error = Some(err.to_string());
                return Err(err);
            }
        }

        {
            let mut client_guard = self.lsp_client.write().await;
            *client_guard = Some(client);
        }

        // F.2: the readiness barrier runs *after* `initialize` but
        // *before* we flip to `Ready`. `Immediate` is an instant
        // pass-through (preserves the pre-F.2 behaviour for every
        // other language); `ProgressAndProbe` waits on jorje.
        match self.await_indexing_ready().await {
            ReadinessOutcome::Ready => {
                self.state
                    .store(SessionState::Ready.as_u8(), Ordering::Release);
                info!("LSP session initialized and ready");
            }
            ReadinessOutcome::DegradedTimeout => {
                // The server is alive (we got an `initialize` response)
                // but didn't signal completion within the deadline.
                // We still flip to `Ready` so callers can issue
                // requests — the worst case is that early requests
                // return `null` and fall through to heuristic, which
                // is the pre-F.2 behaviour anyway.
                warn!(
                    "LSP session readiness barrier timed out; \
                     marking Ready and continuing — early requests \
                     may fall back to heuristic resolution"
                );
                self.state
                    .store(SessionState::Ready.as_u8(), Ordering::Release);
            }
        }
        Ok(())
    }

    /// Readiness barrier. Returns `Ready` when either the active
    /// strategy signals indexing is complete, or `DegradedTimeout`
    /// when the deadline fires first.
    async fn await_indexing_ready(&self) -> ReadinessOutcome {
        let strategy = self.readiness.clone();
        match strategy {
            ReadinessStrategy::Immediate => ReadinessOutcome::Ready,
            ReadinessStrategy::ProgressAndProbe {
                canary_file,
                deadline,
                quiet_period,
            } => {
                self.await_progress_and_probe(canary_file, deadline, quiet_period)
                    .await
            }
        }
    }

    async fn await_progress_and_probe(
        &self,
        canary_file: Option<String>,
        deadline: Duration,
        quiet_period: Duration,
    ) -> ReadinessOutcome {
        let start = Instant::now();
        let poll = Duration::from_millis(200);
        // Probe interval: bound how often we send `documentSymbol`
        // while waiting — once a second is plenty, and more would
        // spam a busy jorje.
        let probe_interval = Duration::from_secs(1);
        let mut last_probe_at: Option<Instant> = None;

        info!(
            canary = ?canary_file,
            deadline_ms = deadline.as_millis() as u64,
            quiet_ms = quiet_period.as_millis() as u64,
            "LSP readiness barrier engaged (ProgressAndProbe)"
        );

        loop {
            // Exit 1: $/progress end was observed.
            if let Some(progress) = self.indexing_progress() {
                if progress.finished {
                    info!(
                        token = %progress.token,
                        elapsed_ms = start.elapsed().as_millis() as u64,
                        "LSP readiness: progress end frame observed"
                    );
                    return ReadinessOutcome::Ready;
                }
            }

            // Exit 2: canary probe returns non-empty.
            if let Some(uri) = canary_file.as_deref() {
                let due = last_probe_at
                    .map(|t| t.elapsed() >= probe_interval)
                    .unwrap_or(true);
                if due {
                    last_probe_at = Some(Instant::now());
                    if self.probe_document_symbol(uri).await {
                        info!(
                            elapsed_ms = start.elapsed().as_millis() as u64,
                            "LSP readiness: documentSymbol probe returned non-empty"
                        );
                        return ReadinessOutcome::Ready;
                    }
                }
            }

            // Exit 3: server has been talking and has now been quiet
            // for `quiet_period`. Requires that we've already seen
            // *some* notification — if the server has said nothing
            // we must not treat that silence as "done".
            if let Some(last) = self.last_notification_at() {
                let indexing_seen = self
                    .observability
                    .indexing_messages_seen
                    .load(std::sync::atomic::Ordering::Relaxed);
                if indexing_seen > 0 && last.elapsed() >= quiet_period {
                    info!(
                        indexing_seen,
                        elapsed_ms = start.elapsed().as_millis() as u64,
                        quiet_ms = last.elapsed().as_millis() as u64,
                        "LSP readiness: quiet period elapsed after indexing signals"
                    );
                    return ReadinessOutcome::Ready;
                }
            }

            if start.elapsed() >= deadline {
                return ReadinessOutcome::DegradedTimeout;
            }

            tokio::time::sleep(poll).await;
        }
    }

    /// Send a `textDocument/documentSymbol` request against `uri` and
    /// return `true` iff the response contains at least one symbol.
    /// All failure modes (client gone, protocol error, empty result,
    /// timeout) return `false` — this is a probe, not a hard check.
    async fn probe_document_symbol(&self, uri: &str) -> bool {
        // Short per-probe timeout: if jorje is truly wedged, we'd
        // rather bail and keep polling than pile up requests.
        let request_timeout = Duration::from_secs(2);
        let client_guard = self.lsp_client.read().await;
        let Some(client) = client_guard.as_ref() else {
            return false;
        };
        let result =
            tokio::time::timeout(request_timeout, client.document_symbol(uri.to_string())).await;
        match result {
            Ok(Ok(Some(symbols))) => !symbols.is_empty(),
            _ => false,
        }
    }

    pub async fn metrics(&self) -> SessionMetrics {
        let mut snap = self.metrics.lock().await.clone();
        self.observability.fill_metrics(&mut snap);
        snap
    }

    /// Find the definition of a symbol (takes read lock — concurrent-safe).
    ///
    /// If the underlying LSP call returns `ServerCrashed`, the supervisor
    /// will attempt a single bounded restart (see `try_recover_from_crash`)
    /// and retry the request once. After a second failure, or if the retry
    /// budget is exhausted, the error propagates so callers can fall back
    /// to the heuristic path.
    pub async fn find_definition(
        &self,
        symbol_name: &str,
        location: &crate::domain::Range,
    ) -> Result<Option<crate::domain::Range>, LspError> {
        let uri = match self.file_to_uri(&location.file) {
            Ok(uri) => uri,
            Err(e) => {
                warn!("Failed to convert file path to URI: {}", e);
                return Ok(None);
            }
        };

        let line = location.start_line.saturating_sub(1);
        // F.2: LSP positions are UTF-16 code units by default.
        // Tree-sitter gives us byte columns, so convert before
        // sending. For ASCII-only Apex this is a pass-through, but
        // the moment a comment or string contains a non-ASCII glyph
        // we'd otherwise target the wrong identifier.
        let byte_col = location.start_char;
        let mut character = utf16_column_for_file(
            Path::new(&location.file),
            location.start_line,
            location.start_char,
        );
        if let Some(last_segment) = symbol_name.rsplit("::").next() {
            if let Some(offset) = symbol_name.rfind(last_segment) {
                character = character.saturating_add(offset as u32);
            }
        }

        // Retry loop: at most one restart attempt per call site. A second
        // crash on the same call burns another budget unit only via the
        // supervisor's global counter, not via re-entry here.
        for attempt in 0u8..=1 {
            let client_guard = self.lsp_client.read().await;
            let Some(client) = client_guard.as_ref() else {
                warn!("LSP client not available for definition lookup");
                return Ok(None);
            };

            // Tier-3 P0 debug: record per-call-site trace when
            // `GRAPHENGINE_LSP_TRACE_DEFS` is set.  Zero cost when off.
            let t_req = Instant::now();
            let res = client.find_definition(uri.clone(), line, character).await;
            drop(client_guard);
            let elapsed = t_req.elapsed();

            if def_trace::enabled() {
                match &res {
                    Ok(Some(def_loc)) => {
                        let (hit_line, hit_col) = def_loc
                            .range
                            .as_ref()
                            .map(|r| (r.start_line, r.start_character))
                            .unwrap_or((0, 0));
                        def_trace::record(
                            &location.file,
                            line,
                            character,
                            byte_col,
                            symbol_name,
                            elapsed,
                            DefOutcome::Hit {
                                uri: &def_loc.uri,
                                line: hit_line,
                                col: hit_col,
                            },
                        );
                    }
                    Ok(None) => {
                        def_trace::record(
                            &location.file,
                            line,
                            character,
                            byte_col,
                            symbol_name,
                            elapsed,
                            DefOutcome::Null,
                        );
                    }
                    Err(e) => {
                        def_trace::record(
                            &location.file,
                            line,
                            character,
                            byte_col,
                            symbol_name,
                            elapsed,
                            DefOutcome::Err(e),
                        );
                    }
                }
            }

            match res {
                Ok(Some(def_loc)) => {
                    return if let Some(path) = self.uri_to_path(&def_loc.uri) {
                        let file_string = path.to_string_lossy().to_string();
                        let range = def_loc.range.map(|r| {
                            crate::domain::Range::with_file(
                                r.start_line + 1,
                                r.start_character,
                                r.end_line + 1,
                                r.end_character,
                                file_string.clone(),
                            )
                        });
                        if let Some(range) = range {
                            Ok(Some(range))
                        } else {
                            Ok(Some(crate::domain::Range::with_file(
                                location.start_line,
                                location.start_char,
                                location.start_line,
                                location.start_char,
                                file_string,
                            )))
                        }
                    } else {
                        warn!(
                            "LSP returned definition URI that could not be converted to path: {}",
                            def_loc.uri
                        );
                        Ok(None)
                    };
                }
                Ok(None) => return Ok(None),
                Err(err) => {
                    if attempt == 0 && self.try_recover_from_crash(&err).await {
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        unreachable!("find_definition retry loop always returns within two iterations")
    }

    /// Get hover information (takes read lock — concurrent-safe).
    ///
    /// Crash recovery semantics are identical to `find_definition`.
    pub async fn hover(&self, location: &crate::domain::Range) -> Result<Option<String>, LspError> {
        let uri = match self.file_to_uri(&location.file) {
            Ok(uri) => uri,
            Err(e) => {
                warn!("Failed to convert file path to URI: {}", e);
                return Ok(None);
            }
        };

        let line = location.start_line.saturating_sub(1);
        let character = utf16_column_for_file(
            Path::new(&location.file),
            location.start_line,
            location.start_char,
        );

        for attempt in 0u8..=1 {
            let client_guard = self.lsp_client.read().await;
            let Some(client) = client_guard.as_ref() else {
                warn!("LSP client not available for hover lookup");
                return Ok(None);
            };

            let res = client.hover(uri.clone(), line, character).await;
            drop(client_guard);

            match res {
                Ok(Some(hover_info)) => return Ok(Some(hover_info.contents)),
                Ok(None) => return Ok(None),
                Err(err) => {
                    if attempt == 0 && self.try_recover_from_crash(&err).await {
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        unreachable!("hover retry loop always returns within two iterations")
    }

    fn effective_workspace_root(&self) -> Option<Url> {
        if let Some(url) = &self.workspace_root {
            return Some(url.clone());
        }

        env::current_dir()
            .ok()
            .and_then(|path| Url::from_directory_path(path).ok())
    }

    fn file_to_uri(&self, file_path: &str) -> Result<String, LspError> {
        let path = Path::new(file_path);
        let mut resolved = if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(root_path) = self
            .effective_workspace_root()
            .and_then(|url| url.to_file_path().ok())
        {
            root_path.join(path)
        } else {
            match env::current_dir() {
                Ok(cwd) => cwd.join(path),
                Err(e) => {
                    return Err(LspError::ProtocolError(format!(
                        "Failed to resolve relative path '{}': {}",
                        file_path, e
                    )))
                }
            }
        };

        if let Ok(canonical) = resolved.canonicalize() {
            resolved = canonical;
        }

        Url::from_file_path(&resolved)
            .map_err(|_| {
                LspError::ProtocolError(format!(
                    "Failed to convert file path '{}' to URI",
                    resolved.display()
                ))
            })
            .map(|url| url.to_string())
    }

    /// Open a document (takes read lock — notification, concurrent-safe)
    pub async fn document_did_open(
        &self,
        file_path: &str,
        content: String,
    ) -> Result<(), LspError> {
        let client_guard = self.lsp_client.read().await;
        if let Some(client) = client_guard.as_ref() {
            let uri = self.file_to_uri(file_path)?;
            let language_id = self.config.language.clone();
            client.document_did_open(uri, language_id, content).await
        } else {
            Err(LspError::ProtocolError(
                "LSP client not available".to_string(),
            ))
        }
    }

    /// Close a document (takes read lock — notification, concurrent-safe)
    pub async fn document_did_close(&self, file_path: &str) -> Result<(), LspError> {
        let client_guard = self.lsp_client.read().await;
        if let Some(client) = client_guard.as_ref() {
            let uri = self.file_to_uri(file_path)?;
            client.document_did_close(uri).await
        } else {
            Err(LspError::ProtocolError(
                "LSP client not available".to_string(),
            ))
        }
    }

    fn uri_to_path(&self, uri: &str) -> Option<PathBuf> {
        if let Ok(parsed) = Url::parse(uri) {
            if parsed.scheme() == "file" {
                return parsed.to_file_path().ok();
            }
            return None;
        }

        let path = PathBuf::from(uri);
        if path.is_absolute() {
            Some(path)
        } else if let Some(root) = &self.workspace_root {
            root.to_file_path()
                .ok()
                .map(|root_path| root_path.join(&path))
        } else if let Ok(cwd) = env::current_dir() {
            Some(cwd.join(path))
        } else {
            None
        }
    }
}

impl Drop for SessionSupervisor {
    fn drop(&mut self) {
        let client = self
            .lsp_client
            .try_write()
            .ok()
            .and_then(|mut guard| guard.take());
        if let Some(mut client) = client {
            tokio::spawn(async move {
                if let Err(e) = client.shutdown().await {
                    warn!("Failed to shutdown LSP client on drop: {:?}", e);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::config::create_default_rust_config;
    use std::time::Duration;
    use url::Url;

    #[test]
    fn test_session_state() {
        assert_eq!(SessionState::Idle.as_u8(), 0);
        assert_eq!(SessionState::Starting.as_u8(), 1);
        assert_eq!(SessionState::Ready.as_u8(), 2);
        assert_eq!(SessionState::Degraded.as_u8(), 3);
        assert_eq!(SessionState::Failed.as_u8(), 4);

        assert_eq!(SessionState::from_u8(0), Some(SessionState::Idle));
        assert_eq!(SessionState::from_u8(5), None);

        assert!(SessionState::Ready.is_functional());
        assert!(SessionState::Degraded.is_functional());
        assert!(!SessionState::Failed.is_functional());

        assert!(SessionState::Ready.can_accept_requests());
        assert!(!SessionState::Degraded.can_accept_requests());
    }

    #[test]
    fn test_session_supervisor_creation() {
        let config = create_default_rust_config();
        let supervisor = SessionSupervisor::new(config, None);

        assert_eq!(supervisor.get_state(), SessionState::Idle);
    }

    #[test]
    fn test_effective_workspace_root_prefers_config() {
        let config = create_default_rust_config();
        let root =
            Url::from_directory_path(std::env::current_dir().unwrap().join("test-workspace")).ok();

        let supervisor = SessionSupervisor::new(config, root.clone());
        assert_eq!(supervisor.effective_workspace_root(), root);
    }

    #[test]
    fn test_effective_workspace_root_falls_back_to_cwd() {
        let config = create_default_rust_config();
        let supervisor = SessionSupervisor::new(config, None);
        let expected = Url::from_directory_path(std::env::current_dir().unwrap()).ok();
        assert_eq!(supervisor.effective_workspace_root(), expected);
    }

    #[tokio::test]
    async fn test_session_metrics_initial_state() {
        let config = create_default_rust_config();
        let supervisor = SessionSupervisor::new(config, None);
        let metrics = supervisor.metrics().await;
        assert_eq!(metrics.start_attempts, 0);
        assert_eq!(metrics.successful_starts, 0);
        assert_eq!(metrics.failed_starts, 0);
        assert!(metrics.last_error.is_none());
    }

    #[tokio::test]
    async fn wait_until_ready_succeeds_when_state_changes() {
        let session = SessionSupervisor::new(create_default_rust_config(), None);
        let clone = session.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            clone.set_state(SessionState::Ready);
        });

        session
            .wait_until_ready(Duration::from_secs(1))
            .await
            .expect("session should become ready");
    }

    #[tokio::test]
    async fn wait_until_ready_times_out() {
        let session = SessionSupervisor::new(create_default_rust_config(), None);
        let result = session.wait_until_ready(Duration::from_millis(10)).await;
        assert!(
            result.is_err(),
            "expected timeout when session never becomes ready"
        );
    }

    #[tokio::test]
    async fn try_recover_ignores_non_crash_errors() {
        let session = SessionSupervisor::new(create_default_rust_config(), None);
        let before = session.remaining_retry_budget();
        let recovered = session
            .try_recover_from_crash(&LspError::timeout(500))
            .await;
        assert!(
            !recovered,
            "timeout errors must not trigger a session-level restart"
        );
        assert_eq!(
            session.remaining_retry_budget(),
            before,
            "retry budget must only be consumed by crash-shaped errors"
        );
        assert_ne!(
            session.get_state(),
            SessionState::Failed,
            "non-crash errors must not transition the supervisor to Failed"
        );
    }

    #[tokio::test]
    async fn try_recover_exhausts_budget_and_marks_failed() {
        let session = SessionSupervisor::new(create_default_rust_config(), None);
        // Drain the budget to zero so the next recovery attempt can't
        // claim a slot. This simulates the "we've already restarted N
        // times and the server is still dying" case.
        session.retry_budget.store(0, Ordering::SeqCst);

        let recovered = session
            .try_recover_from_crash(&LspError::server_crashed("simulated crash"))
            .await;
        assert!(
            !recovered,
            "exhausted retry budget must refuse further recovery"
        );
        assert_eq!(
            session.get_state(),
            SessionState::Failed,
            "supervisor must transition to Failed when budget is exhausted"
        );
        let metrics = session.metrics().await;
        assert!(
            metrics
                .last_error
                .as_deref()
                .is_some_and(|e| e.contains("retry budget exhausted")),
            "metrics must record budget-exhaustion cause, got {:?}",
            metrics.last_error
        );
    }

    #[tokio::test]
    async fn probe_once_is_noop_when_client_is_uninitialized() {
        let session = SessionSupervisor::new(create_default_rust_config(), None);
        // Force a functional state without initialization so the probe
        // hits the `client_guard.is_none()` branch. This mirrors the
        // real race where the probe timer fires between `Ready`
        // transition and the write-lock release that publishes the
        // client.
        session.set_state(SessionState::Ready);

        let observed = session.probe_once().await;
        assert_eq!(
            observed,
            SessionState::Ready,
            "probe must not perturb state when the client isn't published yet"
        );
        assert_eq!(
            session.remaining_retry_budget(),
            3,
            "probe must not consume retry budget when there's nothing to recover"
        );
    }

    #[tokio::test]
    async fn probe_once_in_idle_state_is_noop() {
        let session = SessionSupervisor::new(create_default_rust_config(), None);
        // `Idle` is the pre-init state. A probe firing here (e.g.
        // because `spawn_health_probe` ran before `initialize`) must
        // do nothing — the initializer owns state transitions.
        assert_eq!(session.get_state(), SessionState::Idle);

        let observed = session.probe_once().await;
        assert_eq!(observed, SessionState::Idle);
        assert_eq!(session.remaining_retry_budget(), 3);
    }

    #[tokio::test]
    async fn probe_once_in_failed_state_is_noop() {
        let session = SessionSupervisor::new(create_default_rust_config(), None);
        session.set_state(SessionState::Failed);

        let observed = session.probe_once().await;
        assert_eq!(
            observed,
            SessionState::Failed,
            "Failed is terminal for the probe — no attempt to recover"
        );
        assert_eq!(
            session.remaining_retry_budget(),
            3,
            "probe must not attempt recovery once the supervisor is Failed"
        );
    }

    #[tokio::test]
    async fn spawn_health_probe_exits_when_state_flips_to_failed() {
        let config = create_default_rust_config();
        let session = SessionSupervisor {
            state: Arc::new(AtomicU8::new(SessionState::Ready as u8)),
            lsp_client: Arc::new(RwLock::new(None)),
            retry_budget: Arc::new(AtomicU32::new(3)),
            // Fast cadence so the test completes in well under a
            // second; real callers use 30s.
            health_interval: Duration::from_millis(20),
            config: Arc::new(config),
            workspace_root: None,
            workspace_folders: Vec::new(),
            readiness: ReadinessStrategy::default(),
            security_config: SecurityConfig::default(),
            metrics: Arc::new(tokio::sync::Mutex::new(SessionMetrics::default())),
            observability: NotificationState::new(),
        };

        let handle = session.spawn_health_probe();

        // Let the probe tick at least once while the state is Ready,
        // then flip to Failed. The probe's loop header must observe
        // the flip on its next tick and exit cleanly without needing
        // an explicit abort.
        tokio::time::sleep(Duration::from_millis(50)).await;
        session.set_state(SessionState::Failed);

        // Give the probe a few tick-intervals to notice. If it doesn't
        // exit on its own, the JoinHandle never completes and the
        // `tokio::time::timeout` wrapper below fails the test.
        let finished = tokio::time::timeout(Duration::from_millis(200), handle).await;
        assert!(
            finished.is_ok(),
            "health probe must exit when the supervisor transitions to Failed"
        );
    }

    #[tokio::test]
    async fn try_recover_consumes_one_budget_slot_per_attempt() {
        let session = SessionSupervisor::new(create_default_rust_config(), None);
        assert_eq!(session.remaining_retry_budget(), 3);

        // This will attempt a real restart, which will fail because no
        // LSP server is actually running. We only care that exactly one
        // budget slot was consumed per attempt — that's the contract
        // D.2 depends on when correlating restart count vs. fallback
        // rate.
        let _ = session
            .try_recover_from_crash(&LspError::server_crashed("simulated crash"))
            .await;
        assert_eq!(
            session.remaining_retry_budget(),
            2,
            "exactly one budget slot must be consumed per crash-triggered recovery attempt"
        );
    }

    // ──────────────────────────────────────────────────────────────
    // Sprint F.1: observability plumbing
    // ──────────────────────────────────────────────────────────────
    //
    // These tests exercise the `NotificationState` / `LspNotificationSink`
    // path that the `SimpleLspClient` reader task fires into. They do
    // not spin up a real LSP server — we push synthetic frames
    // directly through the sink. The integration side (real jorje
    // emitting `$/progress`) is covered by the binary LSP harness.

    #[tokio::test]
    async fn notifications_counter_increments_through_sink() {
        let supervisor = SessionSupervisor::new(create_default_rust_config(), None);
        let sink = supervisor.notification_sink();

        sink.record_notification("window/logMessage", None);
        sink.record_notification("textDocument/publishDiagnostics", None);

        let metrics = supervisor.metrics().await;
        assert_eq!(
            metrics.notifications_received, 2,
            "every notification frame must bump the counter"
        );
        assert_eq!(
            metrics.indexing_messages_seen, 0,
            "neither frame is indexing-flavoured"
        );
        assert!(
            supervisor.last_notification_at().is_some(),
            "last_notification_at must be set as soon as the first frame lands"
        );
    }

    #[tokio::test]
    async fn stderr_counter_increments_through_sink() {
        let supervisor = SessionSupervisor::new(create_default_rust_config(), None);
        let sink = supervisor.notification_sink();

        sink.record_stderr_line("INFO  Starting Apex language server");
        sink.record_stderr_line("DEBUG Loaded project");

        let metrics = supervisor.metrics().await;
        assert_eq!(metrics.stderr_lines_observed, 2);
    }

    #[tokio::test]
    async fn progress_frames_populate_indexing_progress() {
        let supervisor = SessionSupervisor::new(create_default_rust_config(), None);
        let sink = supervisor.notification_sink();

        let begin = serde_json::json!({
            "token": "apex-indexing-1",
            "value": { "kind": "begin", "title": "Indexing Apex classes", "percentage": 0 }
        });
        sink.record_notification("$/progress", Some(&begin));

        let report = serde_json::json!({
            "token": "apex-indexing-1",
            "value": { "kind": "report", "message": "force-app/main", "percentage": 42 }
        });
        sink.record_notification("$/progress", Some(&report));

        let end = serde_json::json!({
            "token": "apex-indexing-1",
            "value": { "kind": "end", "message": "done" }
        });
        sink.record_notification("$/progress", Some(&end));

        let metrics = supervisor.metrics().await;
        assert_eq!(metrics.notifications_received, 3);
        assert_eq!(
            metrics.indexing_messages_seen, 3,
            "every $/progress frame counts as an indexing signal"
        );
        let progress = metrics
            .last_indexing_progress
            .as_ref()
            .expect("last progress snapshot must be retained after end frame");
        assert_eq!(progress.token, "apex-indexing-1");
        assert!(
            progress.finished,
            "end frame must set finished=true so the readiness barrier can exit"
        );
    }

    #[tokio::test]
    async fn window_log_message_with_indexing_text_is_classified_as_indexing() {
        let supervisor = SessionSupervisor::new(create_default_rust_config(), None);
        let sink = supervisor.notification_sink();

        let params = serde_json::json!({
            "type": 3,
            "message": "Indexing workspace symbols"
        });
        sink.record_notification("window/logMessage", Some(&params));

        let params_plain = serde_json::json!({
            "type": 3,
            "message": "Parsed trigger"
        });
        sink.record_notification("window/logMessage", Some(&params_plain));

        let metrics = supervisor.metrics().await;
        assert_eq!(metrics.notifications_received, 2);
        assert_eq!(
            metrics.indexing_messages_seen, 1,
            "only the 'indexing' window/logMessage should count as an indexing signal"
        );
    }

    // ---- F.2: readiness barrier ----------------------------------------------

    #[tokio::test]
    async fn immediate_readiness_returns_ready_without_waiting() {
        let supervisor = SessionSupervisor::new(create_default_rust_config(), None);
        let start = Instant::now();
        let outcome = supervisor.await_indexing_ready().await;
        let elapsed = start.elapsed();
        assert_eq!(outcome, ReadinessOutcome::Ready);
        assert!(
            elapsed < Duration::from_millis(50),
            "Immediate strategy must not sleep; elapsed={elapsed:?}"
        );
    }

    #[tokio::test]
    async fn progress_and_probe_exits_on_end_frame() {
        let mut supervisor = SessionSupervisor::new(create_default_rust_config(), None);
        supervisor.set_readiness_strategy(ReadinessStrategy::ProgressAndProbe {
            canary_file: None,
            deadline: Duration::from_secs(2),
            quiet_period: Duration::from_secs(60), // would never fire alone
        });
        // Fire a `$/progress end` immediately so the barrier takes
        // the fast path.
        let sink = supervisor.notification_sink();
        let end = serde_json::json!({
            "token": "idx",
            "value": { "kind": "end" }
        });
        sink.record_notification("$/progress", Some(&end));

        let start = Instant::now();
        let outcome = supervisor.await_indexing_ready().await;
        assert_eq!(outcome, ReadinessOutcome::Ready);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "end frame must short-circuit the barrier"
        );
    }

    #[tokio::test]
    async fn progress_and_probe_exits_on_quiet_period_after_indexing_signal() {
        let mut supervisor = SessionSupervisor::new(create_default_rust_config(), None);
        supervisor.set_readiness_strategy(ReadinessStrategy::ProgressAndProbe {
            canary_file: None,
            deadline: Duration::from_secs(5),
            quiet_period: Duration::from_millis(200),
        });
        // At least one indexing signal, then stop talking. After
        // `quiet_period` of silence, barrier must exit Ready.
        let sink = supervisor.notification_sink();
        let report = serde_json::json!({
            "token": "idx",
            "value": { "kind": "report", "percentage": 42 }
        });
        sink.record_notification("$/progress", Some(&report));

        let outcome =
            tokio::time::timeout(Duration::from_secs(2), supervisor.await_indexing_ready())
                .await
                .expect("barrier must not outlive the 2s timeout");
        assert_eq!(outcome, ReadinessOutcome::Ready);
    }

    #[tokio::test]
    async fn progress_and_probe_times_out_cleanly_when_no_signal_arrives() {
        let mut supervisor = SessionSupervisor::new(create_default_rust_config(), None);
        supervisor.set_readiness_strategy(ReadinessStrategy::ProgressAndProbe {
            canary_file: None,
            // Very short deadline — no probe file, no progress, no
            // quiet-period trigger. Must fall through to
            // DegradedTimeout without panicking.
            deadline: Duration::from_millis(300),
            quiet_period: Duration::from_secs(60),
        });
        let outcome = supervisor.await_indexing_ready().await;
        assert_eq!(outcome, ReadinessOutcome::DegradedTimeout);
    }

    #[test]
    fn workspace_folders_setter_round_trips() {
        let mut supervisor = SessionSupervisor::new(create_default_rust_config(), None);
        supervisor.set_workspace_folders(vec![WorkspaceFolder {
            uri: "file:///tmp/a".into(),
            name: "a".into(),
        }]);
        assert_eq!(supervisor.workspace_folders.len(), 1);
        assert_eq!(supervisor.workspace_folders[0].name, "a");
    }

    #[test]
    fn parse_progress_handles_numeric_and_string_tokens() {
        let numeric = serde_json::json!({ "token": 7, "value": { "kind": "begin" } });
        let p = parse_progress(&numeric).expect("numeric token");
        assert_eq!(p.token, "7");
        assert!(!p.finished);

        let string_tok =
            serde_json::json!({ "token": "abc", "value": { "kind": "end", "message": "  " } });
        let p = parse_progress(&string_tok).expect("string token");
        assert_eq!(p.token, "abc");
        assert!(p.finished);
        assert!(
            p.message.is_none(),
            "whitespace-only message must be normalised to None"
        );
    }
}
