//! Statistics collection for LSP resolution
//!
//! Tracks resolution statistics including edge counts by provenance source,
//! failures, and fallback counts.

use crate::application::ports::ResolutionStatsSummary;
use crate::domain::ProvenanceSource;
use std::collections::HashMap;

/// Tracks a single LSP resolution miss
#[derive(Debug, Clone)]
pub struct LspMiss {
    pub file: String,
    pub symbol: String,
    pub line: u32,
    pub char: u32,
}

/// Statistics for resolution operations
#[derive(Debug, Default)]
pub struct ResolutionStats {
    pub counts: HashMap<ProvenanceSource, usize>,
    pub lsp_failures: Vec<String>,
    pub heuristic_failures: Vec<String>,
    pub heuristic_call_fallbacks: usize,
    pub heuristic_import_fallbacks: usize,
    pub heuristic_type_fallbacks: usize,
    /// See [`ResolutionStatsSummary::heuristic_call_ambiguous_drops`].
    pub heuristic_call_ambiguous_drops: usize,
}

impl ResolutionStats {
    /// Record an edge with the given provenance source
    pub fn record_edge(&mut self, provenance: ProvenanceSource) {
        *self.counts.entry(provenance).or_default() += 1;
    }

    /// Record an LSP failure
    pub fn record_lsp_failure(&mut self, message: String) {
        self.lsp_failures.push(message);
    }

    /// Record a heuristic failure
    pub fn record_heuristic_failure(&mut self, message: String) {
        self.heuristic_failures.push(message);
    }

    /// Convert to summary format
    pub fn into_summary(self) -> ResolutionStatsSummary {
        ResolutionStatsSummary {
            lsp_edges: self
                .counts
                .get(&ProvenanceSource::Lsp)
                .copied()
                .unwrap_or(0),
            heuristic_edges: self
                .counts
                .get(&ProvenanceSource::Heuristic)
                .copied()
                .unwrap_or(0),
            lsp_failures: self.lsp_failures,
            heuristic_failures: self.heuristic_failures,
            heuristic_call_fallbacks: self.heuristic_call_fallbacks,
            heuristic_import_fallbacks: self.heuristic_import_fallbacks,
            heuristic_type_fallbacks: self.heuristic_type_fallbacks,
            heuristic_call_ambiguous_drops: self.heuristic_call_ambiguous_drops,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolution_stats_recording() {
        let mut stats = ResolutionStats::default();
        stats.record_edge(ProvenanceSource::Lsp);
        stats.record_edge(ProvenanceSource::Lsp);
        stats.record_edge(ProvenanceSource::Heuristic);
        stats.record_lsp_failure("test failure".to_string());

        let summary = stats.into_summary();
        assert_eq!(summary.lsp_edges, 2);
        assert_eq!(summary.heuristic_edges, 1);
        assert_eq!(summary.lsp_failures.len(), 1);
    }
}
