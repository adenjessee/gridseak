//! LSP resolution telemetry types shared across parser, CLI, and analysis.
//!
//! These live in the application layer so infrastructure resolvers and
//! downstream consumers agree on a single contract without coupling
//! analysis to `infrastructure::lsp`.

use serde::{Deserialize, Serialize};

use crate::infrastructure::lsp::errors::LspError;

/// Why an LSP lookup fell back to heuristics or produced no edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    ServerMissing,
    RejectedByAvailability,
    ServerCrashed,
    RequestTimeout,
    ReturnedNull,
    DefinitionUnmappable,
    /// LSP resolved to a definition location that lies *outside* the
    /// analyzed repository (a third-party dependency, language builtin,
    /// or stdlib stub). This is expected, not a fidelity defect — kept
    /// separate from `DefinitionUnmappable` so the histogram does not
    /// cry wolf over uninstalled deps and builtins.
    ExternalDefinition,
    NoCallSiteLocation,
    HeuristicProducedEdge,
}

/// Per-reason miss counters accumulated during a scan.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FallbackReasonCounts {
    #[serde(default)]
    pub server_missing: u64,
    #[serde(default)]
    pub rejected_by_availability: u64,
    #[serde(default)]
    pub server_crashed: u64,
    #[serde(default)]
    pub request_timeout: u64,
    #[serde(default)]
    pub returned_null: u64,
    #[serde(default)]
    pub definition_unmappable: u64,
    #[serde(default)]
    pub external_definition: u64,
    #[serde(default)]
    pub no_call_site_location: u64,
    #[serde(default)]
    pub heuristic_produced_edge: u64,
}

impl FallbackReasonCounts {
    pub fn record(&mut self, reason: FallbackReason) {
        match reason {
            FallbackReason::ServerMissing => self.server_missing += 1,
            FallbackReason::RejectedByAvailability => self.rejected_by_availability += 1,
            FallbackReason::ServerCrashed => self.server_crashed += 1,
            FallbackReason::RequestTimeout => self.request_timeout += 1,
            FallbackReason::ReturnedNull => self.returned_null += 1,
            FallbackReason::DefinitionUnmappable => self.definition_unmappable += 1,
            FallbackReason::ExternalDefinition => self.external_definition += 1,
            FallbackReason::NoCallSiteLocation => self.no_call_site_location += 1,
            FallbackReason::HeuristicProducedEdge => self.heuristic_produced_edge += 1,
        }
    }

    pub fn merge(&mut self, other: &Self) {
        self.server_missing += other.server_missing;
        self.rejected_by_availability += other.rejected_by_availability;
        self.server_crashed += other.server_crashed;
        self.request_timeout += other.request_timeout;
        self.returned_null += other.returned_null;
        self.definition_unmappable += other.definition_unmappable;
        self.external_definition += other.external_definition;
        self.no_call_site_location += other.no_call_site_location;
        self.heuristic_produced_edge += other.heuristic_produced_edge;
    }

    pub fn record_heuristic_edges(&mut self, count: usize) {
        for _ in 0..count {
            self.record(FallbackReason::HeuristicProducedEdge);
        }
    }
}

/// Transport- and definition-layer counters from the LSP client.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspRequestMetrics {
    pub request_successes: u64,
    pub request_timeouts: u64,
    pub definition_hits: u64,
    pub definition_nulls: u64,
    pub definition_errors: u64,
}

/// Classify an [`LspError`] into a fallback reason for telemetry.
pub fn classify_lsp_error(err: &LspError) -> FallbackReason {
    match err {
        LspError::Timeout { .. } => FallbackReason::RequestTimeout,
        LspError::ServerCrashed(_) => FallbackReason::ServerCrashed,
        LspError::ServerNotAvailable(_)
        | LspError::ConnectionFailed(_)
        | LspError::InitializationFailed(_)
        | LspError::InvalidConfig(_) => FallbackReason::ServerMissing,
        LspError::ProtocolError(msg) if msg.contains("timeout") || msg.contains("Timeout") => {
            FallbackReason::RequestTimeout
        }
        LspError::ProtocolError(_)
        | LspError::RequestFailed(_)
        | LspError::ResponseParseFailed(_) => FallbackReason::ReturnedNull,
        LspError::HealthCheckFailed(_) => FallbackReason::ServerMissing,
    }
}

/// Classify a session / supervisor initialization failure.
pub fn classify_init_error(err: &LspError) -> FallbackReason {
    match err {
        LspError::ServerCrashed(_) => FallbackReason::ServerCrashed,
        _ => FallbackReason::ServerMissing,
    }
}

/// Classify a definition that LSP resolved to a location we could not
/// bind to a symbol in our index.
///
/// `location_in_repo` is true when the definition's file was one we
/// parsed (so failing to map it is a genuine symbol-index/location gap)
/// and false when the file lies outside the analyzed repository (a
/// third-party dependency, builtin, or stdlib stub — an expected,
/// non-defect outcome).
pub fn classify_unresolved_location(location_in_repo: bool) -> FallbackReason {
    if location_in_repo {
        FallbackReason::DefinitionUnmappable
    } else {
        FallbackReason::ExternalDefinition
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_timeout_errors() {
        assert_eq!(
            classify_lsp_error(&LspError::timeout(5000)),
            FallbackReason::RequestTimeout
        );
        assert_eq!(
            classify_lsp_error(&LspError::protocol_error("Request timeout")),
            FallbackReason::RequestTimeout
        );
    }

    #[test]
    fn unresolved_location_splits_external_from_unmappable() {
        assert_eq!(
            classify_unresolved_location(true),
            FallbackReason::DefinitionUnmappable,
            "in-repo location that does not map is a genuine index gap"
        );
        assert_eq!(
            classify_unresolved_location(false),
            FallbackReason::ExternalDefinition,
            "out-of-repo location is an expected external/builtin target"
        );

        let mut counts = FallbackReasonCounts::default();
        counts.record(classify_unresolved_location(false));
        counts.record(classify_unresolved_location(true));
        assert_eq!(counts.external_definition, 1);
        assert_eq!(counts.definition_unmappable, 1);
    }

    #[test]
    fn merge_accumulates_counts() {
        let mut a = FallbackReasonCounts::default();
        a.record(FallbackReason::ReturnedNull);
        a.record(FallbackReason::ReturnedNull);
        let mut b = FallbackReasonCounts::default();
        b.record(FallbackReason::ServerCrashed);
        a.merge(&b);
        assert_eq!(a.returned_null, 2);
        assert_eq!(a.server_crashed, 1);
    }
}
