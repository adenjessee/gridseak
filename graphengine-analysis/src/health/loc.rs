//! Lines-of-code computation from existing location data on graph nodes.
//!
//! LOC is derived from `end_line - start_line + 1` which the parser already
//! stores. No re-parsing or source access is needed.

use super::graph::AnalysisGraph;

/// Per-function LOC result for annotation enrichment.
pub fn compute_function_loc(graph: &AnalysisGraph) -> std::collections::HashMap<String, usize> {
    let mut results = std::collections::HashMap::new();

    for id in &graph.function_node_ids {
        if let Some(node) = graph.nodes.get(id) {
            if let (Some(start), Some(end)) = (node.start_line, node.end_line) {
                if end >= start {
                    results.insert(id.clone(), (end - start + 1) as usize);
                }
            }
        }
    }

    results
}

/// Aggregate LOC per analysis module.
pub fn compute_module_loc(
    graph: &AnalysisGraph,
    fn_loc: &std::collections::HashMap<String, usize>,
) -> std::collections::HashMap<String, usize> {
    let mut module_totals: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for (fn_id, &loc) in fn_loc {
        if let Some(module_key) = graph.folder_of(fn_id) {
            *module_totals.entry(module_key.clone()).or_default() += loc;
        }
    }

    module_totals
}

/// Total project LOC (sum of all function LOCs).
pub fn total_project_loc(fn_loc: &std::collections::HashMap<String, usize>) -> usize {
    fn_loc.values().sum()
}
