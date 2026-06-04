//! Blast radius computation via reverse BFS.
//!
//! For each function node, counts how many unique nodes transitively depend on it
//! by following structural edges in reverse.

use std::collections::{HashMap, HashSet, VecDeque};

use super::graph::AnalysisGraph;

#[derive(Debug)]
pub struct BlastRadiusResult {
    /// blast_radius per node ID.
    pub radii: HashMap<String, usize>,
}

pub fn compute_blast_radius(graph: &AnalysisGraph) -> BlastRadiusResult {
    // Build reverse adjacency from production edges only so that test→production
    // call edges don't inflate blast radius of production functions.
    let mut reverse_adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for &ei in &graph.production_structural_edge_indices {
        let edge = &graph.edges[ei];
        reverse_adj
            .entry(edge.to_id.as_str())
            .or_default()
            .push(edge.from_id.as_str());
    }

    let mut radii = HashMap::new();

    // Only compute for function-like nodes. Others default to 0.
    // Optimization: skip nodes with fan_in < 2 (blast radius is trivially fan_in for
    // leaf-adjacent nodes, but we compute anyway for correctness; the BFS is fast for small radii).
    for id in &graph.function_node_ids {
        let radius = bfs_count_reachable(id, &reverse_adj);
        radii.insert(id.clone(), radius);
    }

    BlastRadiusResult { radii }
}

/// BFS from `start` in the reverse graph, counting unique reachable nodes (excluding start itself).
fn bfs_count_reachable(start: &str, reverse_adj: &HashMap<&str, Vec<&str>>) -> usize {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    visited.insert(start);
    queue.push_back(start);

    while let Some(current) = queue.pop_front() {
        if let Some(predecessors) = reverse_adj.get(current) {
            for &pred in predecessors {
                if visited.insert(pred) {
                    queue.push_back(pred);
                }
            }
        }
    }

    // Exclude start itself
    visited.len().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::graph::*;
    use std::collections::BTreeMap;

    fn make_fn_graph(edges: Vec<(&str, &str)>) -> AnalysisGraph {
        let mut nodes = BTreeMap::new();
        for (f, t) in &edges {
            for id in [*f, *t] {
                nodes.entry(id.to_string()).or_insert_with(|| GraphNode {
                    id: id.to_string(),
                    kind: NodeKind::Function,
                    fqn: format!("test::{id}"),
                    name: id.to_string(),
                    file_path: None,
                    start_line: None,
                    end_line: None,
                    path_repo_rel: None,
                    role: None,
                    is_test: false,
                    is_vendor: false,
                    is_build_output: false,
                    is_generated: false,
                    cyclomatic_complexity: None,
                    cognitive_complexity: None,
                    visibility: None,
                    import_sources: vec![],
                    is_trait_impl: false,
                    trait_name: None,
                    is_attribute_invoked: false,
                    is_callback_target: false,
                    entry_point_tags: vec![],
                    language: None,
                    frameworks: vec![],
                    is_synthetic: false,
                });
            }
        }
        let edges: Vec<GraphEdge> = edges
            .into_iter()
            .map(|(f, t)| GraphEdge {
                from_id: f.to_string(),
                to_id: t.to_string(),
                kind: EdgeKind::Call,
                confidence: crate::health::graph::Confidence::High,
            })
            .collect();
        let mut g = AnalysisGraph::build(nodes, edges);
        g.compute_module_membership();
        g.finalize_production_edges();
        g
    }

    #[test]
    fn leaf_node_zero_radius() {
        // A → B → C: C has blast_radius 0 (nobody depends on C... wait, actually
        // B depends on C, A depends on B. So C's blast radius = 2 (B and A transitively depend on C).
        // Root A: blast_radius = 0 (nobody depends on A).
        let g = make_fn_graph(vec![("A", "B"), ("B", "C")]);
        let result = compute_blast_radius(&g);
        assert_eq!(result.radii["A"], 0);
        assert_eq!(result.radii["B"], 1); // Only A depends on B
        assert_eq!(result.radii["C"], 2); // A and B depend on C transitively
    }

    #[test]
    fn sink_node() {
        // B → A, C → A
        let g = make_fn_graph(vec![("B", "A"), ("C", "A")]);
        let result = compute_blast_radius(&g);
        assert_eq!(result.radii["A"], 2);
        assert_eq!(result.radii["B"], 0);
        assert_eq!(result.radii["C"], 0);
    }

    #[test]
    fn transitive_chain() {
        // A → B → C → D
        let g = make_fn_graph(vec![("A", "B"), ("B", "C"), ("C", "D")]);
        let result = compute_blast_radius(&g);
        assert_eq!(result.radii["D"], 3);
        assert_eq!(result.radii["C"], 2);
        assert_eq!(result.radii["B"], 1);
        assert_eq!(result.radii["A"], 0);
    }
}
