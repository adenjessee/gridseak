//! Edge aggregation and deduplication for LSP resolution
//!
//! Provides functionality to aggregate edges from multiple resolution phases,
//! deduplicate them, and track statistics.

use crate::application::ports::{ResolutionStatsSummary, UnresolvedReference};
use crate::domain::{Edge, EdgeKind};
use crate::infrastructure::lsp::stats::collector::ResolutionStats;
use std::collections::HashSet;

/// Outcome of LSP-based resolution.
///
/// `unresolved_calls` carries the [`UnresolvedReference`] variant the
/// LSP attempt could not bind, not a stripped `CallSite`. Keeping the
/// variant intact means the heuristic fallback can still emit the
/// correct `Framework(_)` / `Declarative(_)` edge kind for a binding
/// LSP happened to miss — the forward-compat dissolving-element for
/// the P1.d rework.
#[derive(Debug, Default)]
pub struct LspResolutionOutcome {
    pub edges: Vec<Edge>,
    pub unresolved_calls: Vec<UnresolvedReference>,
}

/// Aggregates edges from multiple resolution phases with deduplication
#[derive(Debug, Default)]
pub struct ResolutionAggregator {
    edges: Vec<Edge>,
    seen: HashSet<(String, String, EdgeKind)>,
    stats: ResolutionStats,
}

impl ResolutionAggregator {
    /// Create a new aggregator
    pub fn new() -> Self {
        Self::default()
    }

    /// Add edges to the aggregator, deduplicating by (from_id, to_id, kind)
    pub fn add_edges(&mut self, edges: Vec<Edge>) {
        for edge in edges {
            let key = (edge.from_id.clone(), edge.to_id.clone(), edge.kind);
            if self.seen.insert(key) {
                self.stats.record_edge(edge.provenance.source);
                self.edges.push(edge);
            }
        }
    }

    /// Record an LSP failure
    pub fn record_lsp_failure(&mut self, message: String) {
        self.stats.record_lsp_failure(message);
    }

    /// Record a heuristic failure
    pub fn record_heuristic_failure(&mut self, message: String) {
        self.stats.record_heuristic_failure(message);
    }

    /// Set heuristic fallback counts
    pub fn set_heuristic_fallbacks(
        &mut self,
        call_fallbacks: usize,
        import_fallbacks: usize,
        type_fallbacks: usize,
    ) {
        self.stats.heuristic_call_fallbacks = call_fallbacks;
        self.stats.heuristic_import_fallbacks = import_fallbacks;
        self.stats.heuristic_type_fallbacks = type_fallbacks;
    }

    /// Consume the aggregator and return edges and statistics
    pub fn into_parts(self) -> (Vec<Edge>, ResolutionStatsSummary) {
        let summary = self.stats.into_summary();
        (self.edges, summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Confidence, Provenance, ProvenanceSource};

    #[test]
    fn test_aggregator_deduplicates() {
        let mut aggregator = ResolutionAggregator::new();

        let edge1 = Edge::new(
            "a".into(),
            "b".into(),
            EdgeKind::Call,
            Provenance::new(ProvenanceSource::Lsp, Confidence::High),
        );
        let edge2 = Edge::new(
            "a".into(),
            "b".into(),
            EdgeKind::Call,
            Provenance::new(ProvenanceSource::Heuristic, Confidence::Medium),
        );

        aggregator.add_edges(vec![edge1.clone()]);
        aggregator.add_edges(vec![edge2]);

        let (edges, summary) = aggregator.into_parts();
        assert_eq!(edges.len(), 1, "duplicate edges should be collapsed");
        assert_eq!(summary.lsp_edges, 1);
    }

    #[test]
    fn test_aggregator_tracks_failures() {
        let mut aggregator = ResolutionAggregator::new();
        aggregator.record_lsp_failure("lsp down".to_string());
        aggregator.record_heuristic_failure("heuristic blew up".to_string());

        let (edges, summary) = aggregator.into_parts();
        assert!(edges.is_empty());
        assert_eq!(summary.lsp_failures.len(), 1);
        assert_eq!(summary.heuristic_failures.len(), 1);
    }
}
