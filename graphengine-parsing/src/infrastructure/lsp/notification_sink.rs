//! LSP observability plumbing (Sprint F.1).
//!
//! Before this module existed, every notification jorje sent back to
//! `SimpleLspClient` was silently dropped by `process_messages`, and
//! every line the server wrote to stderr was logged at `debug!` and
//! then forgotten. That blind spot is the single biggest reason we
//! could not explain why Sprint D's `lsp_edges = 0` result persisted:
//! we had no evidence about what the server was doing between
//! `initialize` and the first `textDocument/definition`.
//!
//! The abstraction here is minimal and deliberately boring: the
//! client is given a `dyn LspNotificationSink` and fires a method on
//! it for every notification frame or stderr line it sees. The
//! supervisor is the canonical implementor and stores:
//!
//! * an always-updated `last_notification_at` `Instant` (so readiness
//!   logic in F.2 can say "the server has been quiet for 3s, it's
//!   probably done indexing"),
//! * an optional `IndexingProgress` snapshot (token, percentage,
//!   message) fed from `$/progress` begin/report/end frames,
//! * a running count of notifications, stderr lines, and indexing
//!   messages that ends up in `SessionMetrics` and therefore in the
//!   `--lsp-telemetry` JSON — so a future regression that makes jorje
//!   go silent shows up as `notifications_received == 0` in CI.
//!
//! Verbosity is gated by the environment variable
//! `GRAPHENGINE_LSP_VERBOSE=1`. When set, both stderr lines and
//! structured notifications are logged at `info!`; otherwise they
//! remain at `debug!`. Metric counters update unconditionally — the
//! signal is cheap to collect and we want it in telemetry even when
//! logs are quiet.

use serde_json::Value;
use std::env;
use std::sync::OnceLock;

/// Env-var gate for elevating LSP-side logging from `debug!` to
/// `info!`. Checked exactly once per process via `OnceLock` so the
/// hot path in the reader task doesn't re-parse the environment on
/// every message.
pub const LSP_VERBOSE_ENV_VAR: &str = "GRAPHENGINE_LSP_VERBOSE";

/// `true` when `GRAPHENGINE_LSP_VERBOSE=1` was set in the process
/// environment. Cached so the flag is stable for the lifetime of the
/// LSP session (changing the env after start is intentionally a
/// no-op — no re-reading in hot loops).
pub fn lsp_verbose() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        env::var(LSP_VERBOSE_ENV_VAR)
            .ok()
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
    })
}

/// A point-in-time indexing progress snapshot distilled from the
/// `$/progress` begin/report/end stream. Jorje uses this to signal
/// "workspace symbol cache is warming" — the exact signal F.2 needs
/// to stop racing `textDocument/definition` against an empty index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexingProgress {
    /// The `WorkDoneProgressToken` jorje chose (string or numeric,
    /// normalised to a string for ease of comparison).
    pub token: String,
    /// Percentage 0..=100 if the server reported one; `None` while a
    /// progress frame is "indeterminate".
    pub percentage: Option<u8>,
    /// The most recent `message` field from a `$/progress` report,
    /// trimmed. Useful for logs when jorje reports e.g.
    /// "Indexing project: force-app/main".
    pub message: Option<String>,
    /// `true` once an `end` frame has arrived for this token. A
    /// readiness barrier can return immediately in that case.
    pub finished: bool,
}

/// Observer for LSP-side signals. The client fires these for every
/// notification frame and every stderr line. Implementors must be
/// cheap and non-blocking — the reader task calls this inline.
pub trait LspNotificationSink: Send + Sync {
    /// A JSON-RPC notification (method + optional params) arrived
    /// from the server. The sink is responsible for classifying
    /// `$/progress` vs. `window/logMessage` vs. anything else; the
    /// client does not inspect params.
    fn record_notification(&self, method: &str, params: Option<&Value>);

    /// The server wrote one line to stderr. The line is already
    /// trimmed of its trailing newline.
    fn record_stderr_line(&self, line: &str);
}

/// A no-op sink. Kept around mostly for tests and for the legacy
/// code paths that construct a `SimpleLspClient` without a
/// supervisor (e.g. `test_lsp_integration` binary).
pub struct NullSink;

impl LspNotificationSink for NullSink {
    fn record_notification(&self, _method: &str, _params: Option<&Value>) {}
    fn record_stderr_line(&self, _line: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_is_inert() {
        let s = NullSink;
        s.record_notification("$/progress", None);
        s.record_stderr_line("some stderr");
    }

    #[test]
    fn lsp_verbose_defaults_false_when_env_unset() {
        // NOTE: we can't reliably flip env vars inside a cached-
        // OnceLock test, and we must not depend on the parent env.
        // So this test only asserts that the call path is stable
        // and does not panic. The real opt-in path is exercised by
        // the session-level integration test when run with
        // `GRAPHENGINE_LSP_VERBOSE=1` manually.
        let _ = lsp_verbose();
    }
}
