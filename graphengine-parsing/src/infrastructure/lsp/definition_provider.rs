//! Abstractions for sourcing LSP-backed semantic information.
//!
//! The parsing pipeline should not depend directly on any concrete LSP session
//! or client implementation.  Instead, `DefinitionProvider` offers a small
//! interface for "find definition" lookups and readiness checks so we can swap
//! in real sessions, mocks, or alternative implementations per language.

use crate::application::ports::{CallSite, SessionMetricsSnapshot};
use crate::domain::Range;
use crate::infrastructure::config::LanguageConfig;
use crate::infrastructure::lsp::command_locator::resolve_lsp_command;
use crate::infrastructure::lsp::errors::LspError;
use crate::infrastructure::lsp::session::SessionSupervisor;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, warn};

/// Provides definition lookups for call-sites, typically backed by LSP.
#[async_trait]
pub trait DefinitionProvider: Send + Sync {
    /// Returns true when a definition backend is available (e.g. executable exists).
    async fn is_available(&self) -> bool;

    /// Ensure the backend is ready to accept requests (e.g. launch LSP session).
    async fn ensure_ready(&self) -> Result<(), LspError>;

    /// Resolve the definition location for the supplied call-site.
    async fn find_definition(&self, call_site: &CallSite) -> Result<Option<Range>, LspError>;

    /// Get hover information (type + docs) for a position
    async fn hover(&self, location: &Range) -> Result<Option<String>, LspError>;

    /// Synchronize an in-memory document with the backend (default no-op for mocks).
    async fn open_document(&self, _path: &str, _text: String) -> Result<(), LspError> {
        Ok(())
    }

    /// Close a synchronized document (default no-op for mocks).
    async fn close_document(&self, _path: &str) -> Result<(), LspError> {
        Ok(())
    }

    /// Wait until the backend is ready after opening documents (default no-op for mocks).
    async fn wait_until_ready(&self, _timeout: std::time::Duration) -> Result<(), LspError> {
        Ok(())
    }

    /// Snapshot the backend's session-lifecycle metrics. Default
    /// implementation returns `None`, which is the correct behavior
    /// for mock providers and any future non-LSP backend. LSP-backed
    /// providers override this to expose supervisor telemetry.
    async fn session_metrics(&self) -> Option<SessionMetricsSnapshot> {
        None
    }
}

/// Production implementation that wraps a [`SessionSupervisor`].
pub struct SessionDefinitionProvider {
    session: Arc<SessionSupervisor>,
    config: Arc<LanguageConfig>,
}

impl SessionDefinitionProvider {
    pub fn new(session: Arc<SessionSupervisor>, config: Arc<LanguageConfig>) -> Self {
        Self { session, config }
    }
}

#[async_trait]
impl DefinitionProvider for SessionDefinitionProvider {
    async fn is_available(&self) -> bool {
        // First, verify the configured command resolves. This is a quick check and
        // avoids spawning the server unnecessarily if the executable is missing.
        match resolve_lsp_command(&self.config) {
            Ok(command) => {
                debug!(
                    "Resolved LSP command '{}' for {}",
                    command.executable.display(),
                    self.config.language
                );
                true
            }
            Err(err) => {
                warn!(
                    "Unable to resolve LSP command for {}: {}",
                    self.config.language, err
                );
                false
            }
        }
    }

    async fn ensure_ready(&self) -> Result<(), LspError> {
        // Delegate to the session supervisor which manages retries and metrics.
        self.session.initialize().await
    }

    async fn find_definition(&self, call_site: &CallSite) -> Result<Option<Range>, LspError> {
        self.session
            .find_definition(&call_site.function_name, &call_site.location)
            .await
    }

    async fn hover(&self, location: &Range) -> Result<Option<String>, LspError> {
        self.session.hover(location).await
    }

    async fn open_document(&self, path: &str, text: String) -> Result<(), LspError> {
        self.session.document_did_open(path, text).await
    }

    async fn close_document(&self, path: &str) -> Result<(), LspError> {
        self.session.document_did_close(path).await
    }

    async fn wait_until_ready(&self, timeout: std::time::Duration) -> Result<(), LspError> {
        self.session.wait_until_ready(timeout).await
    }

    async fn session_metrics(&self) -> Option<SessionMetricsSnapshot> {
        let m = self.session.metrics().await;
        Some(SessionMetricsSnapshot {
            start_attempts: m.start_attempts,
            successful_starts: m.successful_starts,
            failed_starts: m.failed_starts,
            last_error: m.last_error.clone(),
            notifications_received: m.notifications_received,
            stderr_lines_observed: m.stderr_lines_observed,
            indexing_messages_seen: m.indexing_messages_seen,
        })
    }
}

/// Helper to construct a session-backed provider for the given configuration.
pub fn session_provider(
    config: LanguageConfig,
    workspace_root: Option<url::Url>,
) -> Arc<dyn DefinitionProvider> {
    session_provider_with_options(config, workspace_root, SessionOptions::default())
}

/// F.2: optional knobs layered onto the basic `session_provider` so
/// Apex (and future languages with nontrivial LSP init contracts) can
/// advertise SFDX-style `workspaceFolders` and opt into a readiness
/// barrier without forcing every other language to care. Every field
/// defaults to the pre-F.2 behaviour, so calling
/// `SessionOptions::default()` is equivalent to the legacy
/// `session_provider` path.
#[derive(Debug, Clone, Default)]
pub struct SessionOptions {
    pub workspace_folders: Vec<crate::infrastructure::lsp::protocol::WorkspaceFolder>,
    pub readiness: crate::infrastructure::lsp::session::ReadinessStrategy,
}

pub fn session_provider_with_options(
    config: LanguageConfig,
    workspace_root: Option<url::Url>,
    options: SessionOptions,
) -> Arc<dyn DefinitionProvider> {
    let config_arc = Arc::new(config);
    let mut supervisor = SessionSupervisor::new((*config_arc).clone(), workspace_root);
    if !options.workspace_folders.is_empty() {
        supervisor.set_workspace_folders(options.workspace_folders);
    }
    supervisor.set_readiness_strategy(options.readiness);
    let session = Arc::new(supervisor);
    Arc::new(SessionDefinitionProvider::new(session, config_arc))
}
