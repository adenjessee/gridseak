//! Mock LSP session supervisor for testing the parsing pipeline
//! without a live language-server process.
//!
//! Relocated from `graphengine-parsing/src/infrastructure/lsp/mock_session.rs`
//! by R2 (v0.1.0-rc1 follow-up). Type definitions and behaviour are
//! byte-for-byte identical to the original; only the import paths
//! changed to consume the parsing crate's public API instead of
//! `crate::…` internal paths.

use graphengine_parsing::infrastructure::config::LanguageConfig;
use graphengine_parsing::infrastructure::lsp::errors::LspError;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tracing::{debug, info, instrument};
use url::Url;

/// Mock LSP session state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockSessionState {
    /// Session is idle and not started
    Idle = 0,
    /// Session is starting up
    Starting = 1,
    /// Session is ready for requests
    Ready = 2,
    /// Session is degraded but functional
    Degraded = 3,
    /// Session has failed and needs restart
    Failed = 4,
}

impl MockSessionState {
    /// Get the numeric value of the state
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Create a state from a numeric value
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(MockSessionState::Idle),
            1 => Some(MockSessionState::Starting),
            2 => Some(MockSessionState::Ready),
            3 => Some(MockSessionState::Degraded),
            4 => Some(MockSessionState::Failed),
            _ => None,
        }
    }

    /// Check if the session is functional
    pub fn is_functional(self) -> bool {
        matches!(self, MockSessionState::Ready | MockSessionState::Degraded)
    }

    /// Check if the session can accept requests
    pub fn can_accept_requests(self) -> bool {
        self == MockSessionState::Ready
    }
}

/// Mock LSP session supervisor for testing
pub struct MockSessionSupervisor {
    /// Current session state
    state: Arc<AtomicU8>,
    /// Language configuration
    #[allow(dead_code)]
    config: Arc<LanguageConfig>,
    /// Workspace root URI
    #[allow(dead_code)]
    workspace_root: Option<Url>,
}

impl MockSessionSupervisor {
    /// Create a new mock session supervisor
    pub fn new(config: LanguageConfig, workspace_root: Option<Url>) -> Self {
        Self {
            state: Arc::new(AtomicU8::new(MockSessionState::Idle as u8)),
            config: Arc::new(config),
            workspace_root,
        }
    }

    /// Get the current session state
    pub fn get_state(&self) -> MockSessionState {
        let state_value = self.state.load(Ordering::SeqCst);
        MockSessionState::from_u8(state_value).unwrap_or(MockSessionState::Failed)
    }

    /// Set the session state
    pub fn set_state(&self, new_state: MockSessionState) {
        self.state.store(new_state.as_u8(), Ordering::SeqCst);
        debug!("Mock LSP session state changed to: {:?}", new_state);
    }

    /// Check if the LSP server is healthy (mock implementation)
    #[instrument(skip(self))]
    pub async fn is_healthy(&self) -> bool {
        self.get_state().is_functional()
    }

    /// Restart the LSP server (mock implementation)
    #[instrument(skip(self))]
    pub async fn restart(&self) -> Result<(), LspError> {
        info!("Mock LSP server restart requested");
        self.set_state(MockSessionState::Idle);
        Ok(())
    }

    /// Kill the LSP server process (mock implementation)
    #[instrument(skip(self))]
    pub async fn kill(&self) {
        info!("Mock LSP server kill requested");
        self.set_state(MockSessionState::Failed);
    }

    /// Reset retry budget (mock implementation)
    pub fn reset_retry_budget(&self) {
        // Mock implementation - no-op
    }
}

impl Clone for MockSessionSupervisor {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            config: Arc::clone(&self.config),
            workspace_root: self.workspace_root.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphengine_parsing::infrastructure::config::create_default_rust_config;

    #[test]
    fn test_mock_session_state() {
        assert_eq!(MockSessionState::Idle.as_u8(), 0);
        assert_eq!(MockSessionState::Starting.as_u8(), 1);
        assert_eq!(MockSessionState::Ready.as_u8(), 2);
        assert_eq!(MockSessionState::Degraded.as_u8(), 3);
        assert_eq!(MockSessionState::Failed.as_u8(), 4);

        assert_eq!(MockSessionState::from_u8(0), Some(MockSessionState::Idle));
        assert_eq!(MockSessionState::from_u8(5), None);

        assert!(MockSessionState::Ready.is_functional());
        assert!(MockSessionState::Degraded.is_functional());
        assert!(!MockSessionState::Failed.is_functional());

        assert!(MockSessionState::Ready.can_accept_requests());
        assert!(!MockSessionState::Degraded.can_accept_requests());
    }

    #[test]
    fn test_mock_session_supervisor_creation() {
        let config = create_default_rust_config();
        let supervisor = MockSessionSupervisor::new(config, None);
        assert_eq!(supervisor.get_state(), MockSessionState::Idle);
    }

    #[tokio::test]
    async fn test_mock_session_health() {
        let config = create_default_rust_config();
        let supervisor = MockSessionSupervisor::new(config, None);
        assert!(!supervisor.is_healthy().await);
        supervisor.set_state(MockSessionState::Ready);
        assert!(supervisor.is_healthy().await);
    }

    #[tokio::test]
    async fn test_mock_session_restart() {
        let config = create_default_rust_config();
        let supervisor = MockSessionSupervisor::new(config, None);
        supervisor.set_state(MockSessionState::Failed);
        assert_eq!(supervisor.get_state(), MockSessionState::Failed);
        supervisor.restart().await.unwrap();
        assert_eq!(supervisor.get_state(), MockSessionState::Idle);
    }
}
