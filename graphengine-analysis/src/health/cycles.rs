//! Cycle detection via Tarjan's Strongly Connected Components algorithm.
//!
//! Finds all non-trivial SCCs (size > 1) in the structural (non-containment) edge graph.
//! Each SCC represents a circular dependency.

use std::collections::HashMap;

use super::graph::AnalysisGraph;

/// A strongly connected component (cycle) in the graph.
#[derive(Debug, Clone)]
pub struct Cycle {
    pub id: String,
    pub node_ids: Vec<String>,
    pub edge_indices: Vec<usize>,
}

/// Result of cycle detection.
#[derive(Debug)]
pub struct CycleResult {
    pub cycles: Vec<Cycle>,
    /// Total number of unique nodes participating in any cycle.
    pub total_cycle_nodes: usize,
    /// Indices of edges that participate in at least one cycle.
    pub edges_in_cycles: usize,
}

pub fn detect_cycles(graph: &AnalysisGraph) -> CycleResult {
    let sccs = tarjan_scc(graph);

    let mut cycles = Vec::new();
    let mut all_cycle_node_ids = std::collections::HashSet::new();
    let mut cycle_edge_count = 0;

    for (idx, scc) in sccs.iter().enumerate() {
        if scc.len() <= 1 {
            continue;
        }

        let scc_set: std::collections::HashSet<&str> = scc.iter().map(|s| s.as_str()).collect();

        let mut edge_indices = Vec::new();
        for &ei in &graph.production_structural_edge_indices {
            let edge = &graph.edges[ei];
            if scc_set.contains(edge.from_id.as_str()) && scc_set.contains(edge.to_id.as_str()) {
                edge_indices.push(ei);
            }
        }

        cycle_edge_count += edge_indices.len();
        for nid in scc {
            all_cycle_node_ids.insert(nid.clone());
        }

        let mut sorted_nodes = scc.clone();
        sorted_nodes.sort();

        cycles.push(Cycle {
            id: format!("cycle-{}", idx + 1),
            node_ids: sorted_nodes,
            edge_indices,
        });
    }

    // Sort cycles by size descending, then by first node id for determinism
    cycles.sort_by(|a, b| {
        b.node_ids
            .len()
            .cmp(&a.node_ids.len())
            .then_with(|| a.node_ids.first().cmp(&b.node_ids.first()))
    });

    // Re-number after sorting
    for (i, c) in cycles.iter_mut().enumerate() {
        c.id = format!("cycle-{}", i + 1);
    }

    CycleResult {
        total_cycle_nodes: all_cycle_node_ids.len(),
        edges_in_cycles: cycle_edge_count,
        cycles,
    }
}

// ---------------------------------------------------------------------------
// Tarjan's SCC (iterative to avoid stack overflow on large cycles)
// ---------------------------------------------------------------------------

fn tarjan_scc(graph: &AnalysisGraph) -> Vec<Vec<String>> {
    // Use production edges only — excludes synthetic nodes AND test/example/vendor
    // nodes so that test→production call edges don't create false cycles.
    let edge_set = &graph.production_structural_edge_indices;

    let mut relevant_nodes: Vec<&str> = Vec::new();
    for &ei in edge_set {
        let edge = &graph.edges[ei];
        relevant_nodes.push(&edge.from_id);
        relevant_nodes.push(&edge.to_id);
    }
    relevant_nodes.sort();
    relevant_nodes.dedup();

    let node_to_idx: HashMap<&str, usize> = relevant_nodes
        .iter()
        .enumerate()
        .map(|(i, &n)| (n, i))
        .collect();
    let n = relevant_nodes.len();

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &ei in edge_set {
        let edge = &graph.edges[ei];
        if let (Some(&from), Some(&to)) = (
            node_to_idx.get(edge.from_id.as_str()),
            node_to_idx.get(edge.to_id.as_str()),
        ) {
            adj[from].push(to);
        }
    }

    // Iterative Tarjan's
    let mut index_counter: usize = 0;
    let mut stack: Vec<usize> = Vec::new();
    let mut on_stack = vec![false; n];
    let mut indices = vec![usize::MAX; n];
    let mut lowlinks = vec![usize::MAX; n];
    let mut result: Vec<Vec<String>> = Vec::new();

    // DFS work stack: (node, neighbor_iterator_position)
    let mut dfs_stack: Vec<(usize, usize)> = Vec::new();

    for start in 0..n {
        if indices[start] != usize::MAX {
            continue;
        }

        dfs_stack.push((start, 0));
        indices[start] = index_counter;
        lowlinks[start] = index_counter;
        index_counter += 1;
        stack.push(start);
        on_stack[start] = true;

        while let Some(&mut (v, ref mut ni)) = dfs_stack.last_mut() {
            if *ni < adj[v].len() {
                let w = adj[v][*ni];
                *ni += 1;

                if indices[w] == usize::MAX {
                    // Not yet visited — push onto DFS stack
                    indices[w] = index_counter;
                    lowlinks[w] = index_counter;
                    index_counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    dfs_stack.push((w, 0));
                } else if on_stack[w] {
                    lowlinks[v] = lowlinks[v].min(indices[w]);
                }
            } else {
                // All neighbors processed — check if root of SCC
                if lowlinks[v] == indices[v] {
                    let mut scc = Vec::new();
                    loop {
                        let w = stack.pop().unwrap();
                        on_stack[w] = false;
                        scc.push(relevant_nodes[w].to_string());
                        if w == v {
                            break;
                        }
                    }
                    result.push(scc);
                }

                let finished = dfs_stack.pop().unwrap();
                // Propagate lowlink to parent
                if let Some(&mut (parent, _)) = dfs_stack.last_mut() {
                    lowlinks[parent] = lowlinks[parent].min(lowlinks[finished.0]);
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::graph::*;
    use std::collections::BTreeMap;

    fn make_graph(edges: Vec<(&str, &str, EdgeKind)>) -> AnalysisGraph {
        let mut nodes = BTreeMap::new();
        for (from, to, _) in &edges {
            for id in [*from, *to] {
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
            .map(|(f, t, k)| GraphEdge {
                from_id: f.to_string(),
                to_id: t.to_string(),
                kind: k,
                confidence: crate::health::graph::Confidence::High,
            })
            .collect();

        let mut g = AnalysisGraph::build(nodes, edges);
        g.compute_module_membership();
        g.finalize_production_edges();
        g
    }

    #[test]
    fn no_cycles_linear() {
        let g = make_graph(vec![("A", "B", EdgeKind::Call), ("B", "C", EdgeKind::Call)]);
        let result = detect_cycles(&g);
        assert_eq!(result.cycles.len(), 0);
        assert_eq!(result.total_cycle_nodes, 0);
    }

    #[test]
    fn simple_two_cycle() {
        let g = make_graph(vec![("A", "B", EdgeKind::Call), ("B", "A", EdgeKind::Call)]);
        let result = detect_cycles(&g);
        assert_eq!(result.cycles.len(), 1);
        assert_eq!(result.cycles[0].node_ids.len(), 2);
    }

    #[test]
    fn simple_three_cycle() {
        let g = make_graph(vec![
            ("A", "B", EdgeKind::Call),
            ("B", "C", EdgeKind::Call),
            ("C", "A", EdgeKind::Call),
        ]);
        let result = detect_cycles(&g);
        assert_eq!(result.cycles.len(), 1);
        assert_eq!(result.cycles[0].node_ids.len(), 3);
    }

    #[test]
    fn two_independent_cycles() {
        let g = make_graph(vec![
            ("A", "B", EdgeKind::Call),
            ("B", "A", EdgeKind::Call),
            ("C", "D", EdgeKind::Call),
            ("D", "C", EdgeKind::Call),
        ]);
        let result = detect_cycles(&g);
        assert_eq!(result.cycles.len(), 2);
    }

    #[test]
    fn containment_edges_excluded() {
        // A→(Contains)→B, B→(Call)→C, C→(Call)→A
        // Structural graph: B→C, C→A — a chain, NOT a cycle (no A→B structural edge).
        let g = make_graph(vec![
            ("A", "B", EdgeKind::Contains),
            ("B", "C", EdgeKind::Call),
            ("C", "A", EdgeKind::Call),
        ]);
        let result = detect_cycles(&g);
        assert_eq!(result.cycles.len(), 0);
    }

    #[test]
    fn containment_excluded_but_structural_cycle_remains() {
        // A→(Contains)→B, A→(Call)→C, C→(Call)→A — structural cycle between A and C
        let g = make_graph(vec![
            ("A", "B", EdgeKind::Contains),
            ("A", "C", EdgeKind::Call),
            ("C", "A", EdgeKind::Call),
        ]);
        let result = detect_cycles(&g);
        assert_eq!(result.cycles.len(), 1);
        assert_eq!(result.cycles[0].node_ids.len(), 2);
    }
}
