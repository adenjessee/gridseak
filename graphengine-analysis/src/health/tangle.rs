//! Package tangle index.
//!
//! Measures what fraction of total structural edges participate in cycles.
//! 0.0 = no cycles, 1.0 = every edge is in a cycle.

use super::graph::AnalysisGraph;

pub fn compute_tangle_index(edges_in_cycles: usize, graph: &AnalysisGraph) -> f64 {
    let total = graph.total_structural_edges();
    if total == 0 {
        return 0.0;
    }
    edges_in_cycles as f64 / total as f64
}
