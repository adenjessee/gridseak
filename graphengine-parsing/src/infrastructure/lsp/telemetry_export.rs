//! End-of-scan LSP telemetry JSON export (Sprint D.4).
//!
//! Serializable snapshot of `ResolutionStatsSummary` plus derived
//! fields, written to a user-chosen path by the CLI's `--lsp-telemetry`
//! flag. This is the interchange format that lets a downstream
//! analyzer (typically on a different host, e.g. CI) emit the
//! [`health::resolution_degraded`] finding without needing direct
//! access to the parser's SQLite database.
//!
//! # Contract
//!
//! The JSON schema mirrors the keys written to the graph metadata by
//! [`orchestrator.rs`](crate::application::use_cases::parse_repo::pipeline::orchestrator),
//! so a consumer can reconstruct the same
//! [`ResolutionStatsSnapshot`](graphengine_analysis::health::resolution_degraded::ResolutionStatsSnapshot)
//! from either source. Derived fields (`fallback_rate`,
//! `total_resolution_work`) are computed once here so consumers don't
//! drift from the analysis crate's definition of "fallback rate".
//!
//! We intentionally version this file (`schema_version`) so a future
//! schema change can be detected without ambiguity.

use serde::{Deserialize, Serialize};

use crate::application::ports::ResolutionStatsSummary;

/// Current on-disk schema version. Bump when adding / renaming /
/// removing fields.
pub const SCHEMA_VERSION: &str = "1";

/// Full telemetry document written by `--lsp-telemetry`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LspTelemetryReport {
    pub schema_version: String,
    /// Wall-clock time the scan finished, in seconds since the Unix
    /// epoch (UTC). We use a plain integer instead of an RFC3339
    /// string to avoid pulling a datetime crate just for one field —
    /// downstream consumers can format it as they see fit.
    pub generated_at_unix_s: u64,
    pub language: String,
    pub scan_duration_ms: u64,
    pub counters: ResolutionCounters,
    pub derived: DerivedResolutionMetrics,
    /// `SessionSupervisor`-level metrics (start attempts, crash
    /// counts, last error). Currently `None` because
    /// [`SessionMetrics`](super::session::SessionMetrics) is not yet
    /// threaded through `ParseRepositoryUseCase` → `ResolvedGraph`.
    /// The field is present in v1 of the schema so downstream
    /// consumers can start expecting it without a schema bump when
    /// the wiring lands. Being explicit (rather than silently
    /// omitting) prevents the "we thought LSP was healthy" failure
    /// mode this whole telemetry layer exists to prevent.
    #[serde(default)]
    pub session_metrics: Option<SessionMetricsSnapshot>,
}

/// Session-lifecycle snapshot serializable alongside resolution
/// counters. Mirrors
/// [`crate::application::ports::SessionMetricsSnapshot`] field for
/// field. Duplicated here (rather than re-exported) so the on-disk
/// telemetry contract is stable against any refactor of the
/// application-layer port type — a breaking port rename must not
/// force a schema bump on external consumers without an explicit
/// choice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SessionMetricsSnapshot {
    pub start_attempts: u64,
    pub successful_starts: u64,
    pub failed_starts: u64,
    pub last_error: Option<String>,
    /// Sprint F.1 — see
    /// [`crate::application::ports::SessionMetricsSnapshot::notifications_received`].
    /// `#[serde(default)]` so v0 telemetry files (no observability
    /// fields) still deserialize cleanly.
    #[serde(default)]
    pub notifications_received: u64,
    #[serde(default)]
    pub stderr_lines_observed: u64,
    #[serde(default)]
    pub indexing_messages_seen: u64,
}

impl From<&crate::application::ports::SessionMetricsSnapshot> for SessionMetricsSnapshot {
    fn from(m: &crate::application::ports::SessionMetricsSnapshot) -> Self {
        Self {
            start_attempts: m.start_attempts,
            successful_starts: m.successful_starts,
            failed_starts: m.failed_starts,
            last_error: m.last_error.clone(),
            notifications_received: m.notifications_received,
            stderr_lines_observed: m.stderr_lines_observed,
            indexing_messages_seen: m.indexing_messages_seen,
        }
    }
}

impl From<&crate::infrastructure::lsp::session::SessionMetrics> for SessionMetricsSnapshot {
    fn from(m: &crate::infrastructure::lsp::session::SessionMetrics) -> Self {
        Self {
            start_attempts: m.start_attempts,
            successful_starts: m.successful_starts,
            failed_starts: m.failed_starts,
            last_error: m.last_error.clone(),
            notifications_received: m.notifications_received,
            stderr_lines_observed: m.stderr_lines_observed,
            indexing_messages_seen: m.indexing_messages_seen,
        }
    }
}

/// Raw resolution counters. Field names intentionally match the
/// metadata keys written to SQLite (sans the `resolution_` prefix) so
/// the two channels stay obviously paired.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolutionCounters {
    pub lsp_edges: usize,
    pub heuristic_edges: usize,
    pub heuristic_call_fallbacks: usize,
    pub heuristic_import_fallbacks: usize,
    pub heuristic_type_fallbacks: usize,
    pub heuristic_call_ambiguous_drops: usize,
}

impl From<&ResolutionStatsSummary> for ResolutionCounters {
    fn from(s: &ResolutionStatsSummary) -> Self {
        Self {
            lsp_edges: s.lsp_edges,
            heuristic_edges: s.heuristic_edges,
            heuristic_call_fallbacks: s.heuristic_call_fallbacks,
            heuristic_import_fallbacks: s.heuristic_import_fallbacks,
            heuristic_type_fallbacks: s.heuristic_type_fallbacks,
            heuristic_call_ambiguous_drops: s.heuristic_call_ambiguous_drops,
        }
    }
}

/// Metrics computed from the raw counters. Keeping them here (rather
/// than recomputing in every consumer) fixes a single definition of
/// "fallback rate" across the parser CLI, the analysis crate, the UI,
/// and external dashboards. See
/// [`graphengine_analysis::health::resolution_degraded`] for the
/// analytical counterpart.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct DerivedResolutionMetrics {
    /// `lsp_edges + heuristic_edges + heuristic_call_ambiguous_drops`.
    /// Dropped sites are included because they represent resolution
    /// work that happened — LSP would have converted them to edges.
    pub total_resolution_work: usize,
    /// `heuristic_edges + heuristic_call_ambiguous_drops`.
    pub heuristic_and_drops: usize,
    /// `heuristic_and_drops / total_resolution_work` in `[0, 1]`.
    /// `None` when `total_resolution_work == 0`.
    pub fallback_rate: Option<f64>,
}

impl DerivedResolutionMetrics {
    pub fn from_counters(c: &ResolutionCounters) -> Self {
        let total = c
            .lsp_edges
            .saturating_add(c.heuristic_edges)
            .saturating_add(c.heuristic_call_ambiguous_drops);
        let heur_drops = c
            .heuristic_edges
            .saturating_add(c.heuristic_call_ambiguous_drops);
        let rate = if total == 0 {
            None
        } else {
            Some(heur_drops as f64 / total as f64)
        };
        Self {
            total_resolution_work: total,
            heuristic_and_drops: heur_drops,
            fallback_rate: rate,
        }
    }
}

impl LspTelemetryReport {
    pub fn build(stats: &ResolutionStatsSummary, language: String, scan_duration_ms: u64) -> Self {
        let counters = ResolutionCounters::from(stats);
        let derived = DerivedResolutionMetrics::from_counters(&counters);
        let generated_at_unix_s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            generated_at_unix_s,
            language,
            scan_duration_ms,
            counters,
            derived,
            session_metrics: None,
        }
    }

    /// Attach an LSP session-lifecycle snapshot (from either the
    /// application-layer port type or the infra-layer supervisor
    /// type). Generic over `Into<SessionMetricsSnapshot>` so the CLI
    /// (which works with port types) and infra-level tests (which
    /// instantiate `SessionMetrics` directly) can both use this
    /// without glue code.
    pub fn with_session_metrics<M>(mut self, metrics: M) -> Self
    where
        M: Into<SessionMetricsSnapshot>,
    {
        self.session_metrics = Some(metrics.into());
        self
    }

    /// Serialize to pretty-printed JSON. Pretty-printing is worth the
    /// extra bytes because these files are typically tiny (hundreds
    /// of bytes) and humans read them when triaging scan quality.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_stats() -> ResolutionStatsSummary {
        ResolutionStatsSummary {
            lsp_edges: 700,
            heuristic_edges: 200,
            lsp_failures: Vec::new(),
            heuristic_failures: Vec::new(),
            heuristic_call_fallbacks: 40,
            heuristic_import_fallbacks: 10,
            heuristic_type_fallbacks: 5,
            heuristic_call_ambiguous_drops: 100,
        }
    }

    #[test]
    fn counters_mirror_stats_summary_field_for_field() {
        let stats = sample_stats();
        let counters = ResolutionCounters::from(&stats);
        assert_eq!(counters.lsp_edges, 700);
        assert_eq!(counters.heuristic_edges, 200);
        assert_eq!(counters.heuristic_call_fallbacks, 40);
        assert_eq!(counters.heuristic_import_fallbacks, 10);
        assert_eq!(counters.heuristic_type_fallbacks, 5);
        assert_eq!(counters.heuristic_call_ambiguous_drops, 100);
    }

    #[test]
    fn derived_metrics_compute_fallback_rate_correctly() {
        let stats = sample_stats();
        let counters = ResolutionCounters::from(&stats);
        let derived = DerivedResolutionMetrics::from_counters(&counters);

        // total = 700 + 200 + 100 = 1000
        assert_eq!(derived.total_resolution_work, 1000);
        // heur_drops = 200 + 100 = 300
        assert_eq!(derived.heuristic_and_drops, 300);
        // rate = 300 / 1000 = 0.30
        assert!((derived.fallback_rate.unwrap() - 0.30).abs() < 1e-9);
    }

    #[test]
    fn derived_metrics_return_none_for_empty_scan() {
        let counters = ResolutionCounters {
            lsp_edges: 0,
            heuristic_edges: 0,
            heuristic_call_fallbacks: 0,
            heuristic_import_fallbacks: 0,
            heuristic_type_fallbacks: 0,
            heuristic_call_ambiguous_drops: 0,
        };
        let derived = DerivedResolutionMetrics::from_counters(&counters);
        assert_eq!(derived.total_resolution_work, 0);
        assert!(
            derived.fallback_rate.is_none(),
            "empty scan must not produce a fabricated 0.0 fallback rate"
        );
    }

    #[test]
    fn report_schema_version_is_pinned() {
        let report = LspTelemetryReport::build(&sample_stats(), "apex".into(), 1234);
        assert_eq!(
            report.schema_version, SCHEMA_VERSION,
            "schema_version must match the pinned constant — bump SCHEMA_VERSION \
             when changing field layout"
        );
    }

    #[test]
    fn report_roundtrips_through_json() {
        let report = LspTelemetryReport::build(&sample_stats(), "apex".into(), 4321);
        let json = report.to_json().expect("serialize");
        let parsed: LspTelemetryReport = serde_json::from_str(&json).expect("roundtrip");
        assert_eq!(report, parsed);
    }

    #[test]
    fn session_metrics_snapshot_roundtrips_and_defaults_to_none() {
        use crate::infrastructure::lsp::session::SessionMetrics;
        let report = LspTelemetryReport::build(&sample_stats(), "apex".into(), 1);
        assert!(
            report.session_metrics.is_none(),
            "session_metrics should default to None until attached"
        );
        let metrics = SessionMetrics {
            start_attempts: 3,
            successful_starts: 2,
            failed_starts: 1,
            last_error: Some("boom".into()),
            ..Default::default()
        };
        let report = report.with_session_metrics(&metrics);
        let snap = report.session_metrics.as_ref().expect("attached");
        assert_eq!(snap.start_attempts, 3);
        assert_eq!(snap.successful_starts, 2);
        assert_eq!(snap.failed_starts, 1);
        assert_eq!(snap.last_error.as_deref(), Some("boom"));
        let json = report.to_json().expect("serialize");
        let parsed: LspTelemetryReport = serde_json::from_str(&json).expect("roundtrip");
        assert_eq!(parsed.session_metrics, report.session_metrics);
    }

    #[test]
    fn session_metrics_attaches_from_port_layer_snapshot() {
        use crate::application::ports::SessionMetricsSnapshot as PortSnap;
        let port_snap = PortSnap {
            start_attempts: 5,
            successful_starts: 4,
            failed_starts: 1,
            last_error: Some("init failed".into()),
            ..Default::default()
        };
        let report = LspTelemetryReport::build(&sample_stats(), "apex".into(), 10)
            .with_session_metrics(&port_snap);
        let snap = report.session_metrics.as_ref().expect("attached");
        assert_eq!(snap.start_attempts, 5);
        assert_eq!(snap.successful_starts, 4);
        assert_eq!(snap.failed_starts, 1);
        assert_eq!(snap.last_error.as_deref(), Some("init failed"));
    }

    #[test]
    fn report_includes_expected_top_level_keys() {
        // Guard against accidental renames. External dashboards will
        // break if these keys change without a schema_version bump.
        let report = LspTelemetryReport::build(&sample_stats(), "apex".into(), 0);
        let json = report.to_json().expect("serialize");
        for key in [
            "\"schema_version\"",
            "\"generated_at_unix_s\"",
            "\"language\"",
            "\"scan_duration_ms\"",
            "\"counters\"",
            "\"derived\"",
            "\"lsp_edges\"",
            "\"heuristic_edges\"",
            "\"heuristic_call_ambiguous_drops\"",
            "\"fallback_rate\"",
            "\"total_resolution_work\"",
        ] {
            assert!(
                json.contains(key),
                "telemetry JSON must contain {key}; actual: {json}"
            );
        }
    }
}
