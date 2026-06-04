//! Call chain depth analysis.
//!
//! Computes the maximum call depth and per-node depth from roots using BFS propagation.
//! O(V+E) — each node/edge visited at most once. Cycle edges are broken using SCC results.

use std::collections::{HashMap, HashSet, VecDeque};

use super::graph::AnalysisGraph;

#[derive(Debug)]
pub struct DepthResult {
    pub max_call_depth: usize,
    /// Per-node depth from nearest root. Only function nodes are tracked.
    pub depth_from_root: HashMap<String, usize>,
}

pub fn compute_depth(graph: &AnalysisGraph, cycle_nodes: &HashSet<String>) -> DepthResult {
    // Build Call-only adjacency using production edges only (no synthetic/test nodes)
    let mut call_outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut all_call_nodes: HashSet<&str> = HashSet::new();

    for &ei in &graph.production_structural_edge_indices {
        let edge = &graph.edges[ei];
        // Call-depth is a call-semantic metric; include framework and
        // declarative dispatch edges alongside normal `Call`. See
        // DISCOVERY_REPORT.md §8 Decision 5. Prior to universal-
        // fidelity T1 this was `== EdgeKind::Call` literally, which
        // caused framework dispatches (VF page → controller method) to
        // be invisible to depth analysis.
        if !edge.kind.is_call_like() {
            continue;
        }

        // Skip edges between cycle members (breaks cycles for topological traversal)
        if cycle_nodes.contains(edge.from_id.as_str()) && cycle_nodes.contains(edge.to_id.as_str())
        {
            continue;
        }

        call_outgoing
            .entry(edge.from_id.as_str())
            .or_default()
            .push(edge.to_id.as_str());

        *in_degree.entry(edge.to_id.as_str()).or_insert(0) += 1;
        in_degree.entry(edge.from_id.as_str()).or_insert(0);

        all_call_nodes.insert(edge.from_id.as_str());
        all_call_nodes.insert(edge.to_id.as_str());
    }

    // BFS longest-path: start from roots (in-degree 0), propagate max depth
    let mut depth_from_root: HashMap<String, usize> = HashMap::new();
    let mut remaining_in: HashMap<&str, usize> = in_degree.clone();
    let mut queue: VecDeque<&str> = VecDeque::new();

    for &node in &all_call_nodes {
        if *remaining_in.get(node).unwrap_or(&0) == 0 {
            queue.push_back(node);
            depth_from_root.insert(node.to_string(), 0);
        }
    }

    let mut max_depth: usize = 0;

    while let Some(node) = queue.pop_front() {
        let current_depth = depth_from_root.get(node).copied().unwrap_or(0);

        if let Some(targets) = call_outgoing.get(node) {
            for &target in targets {
                let new_depth = current_depth + 1;
                let entry = depth_from_root.entry(target.to_string()).or_insert(0);
                *entry = (*entry).max(new_depth);
                max_depth = max_depth.max(new_depth);

                if let Some(rem) = remaining_in.get_mut(target) {
                    *rem = rem.saturating_sub(1);
                    if *rem == 0 {
                        queue.push_back(target);
                    }
                }
            }
        }
    }

    DepthResult {
        max_call_depth: max_depth,
        depth_from_root,
    }
}

#[cfg(test)]
mod tests {
    use super::super::graph::EdgeKind;
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
    fn linear_depth() {
        let g = make_fn_graph(vec![("A", "B"), ("B", "C"), ("C", "D")]);
        let result = compute_depth(&g, &HashSet::new());
        assert_eq!(result.max_call_depth, 3);
        assert_eq!(result.depth_from_root["A"], 0);
        assert_eq!(result.depth_from_root["D"], 3);
    }

    #[test]
    fn branching_depth() {
        // A → B → D, A → C → D: max depth is 2 via either path
        let g = make_fn_graph(vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")]);
        let result = compute_depth(&g, &HashSet::new());
        assert_eq!(result.max_call_depth, 2);
    }

    #[test]
    fn cycle_broken() {
        // A → B → C → A (cycle). With cycle nodes {A,B,C}, cycle edges are skipped.
        let g = make_fn_graph(vec![("A", "B"), ("B", "C"), ("C", "A")]);
        let mut cycle_set = HashSet::new();
        cycle_set.insert("A".to_string());
        cycle_set.insert("B".to_string());
        cycle_set.insert("C".to_string());
        let result = compute_depth(&g, &cycle_set);
        // All cycle edges skipped, so max depth is 0
        assert_eq!(result.max_call_depth, 0);
    }
}
