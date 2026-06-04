//! Module cohesion analysis (LCOM4 variant).
//!
//! For each analysis module, builds an undirected function-relationship subgraph
//! and counts connected components. A cohesive module has all functions in one
//! component; an incohesive module is a grab-bag of unrelated groups.
//!
//! Relationships:
//! - One function calls the other (Call edge in either direction)
//! - Both call the same external function (shared dependency)
//! - Both are called by the same external function (shared consumer)

use std::collections::{BTreeMap, HashMap, HashSet};

use super::graph::AnalysisGraph;

#[derive(Debug, Clone)]
pub struct ModuleCohesion {
    pub cohesion_score: f64,
    pub connected_components: usize,
    pub function_count: usize,
}

#[derive(Debug)]
pub struct CohesionResult {
    /// Keyed by module id. `BTreeMap` — not `HashMap` — so every
    /// downstream iteration (finding-ID assignment, avg_cohesion
    /// summation, HealthReport descriptions) sees modules in a
    /// canonical lexical order. See R35 in FOLLOWUP_RISKS.md for the
    /// non-determinism observations this addresses.
    pub modules: BTreeMap<String, ModuleCohesion>,
}

/// High-confidence-only variant used by T3 dual-metric emission.
/// Only `Confidence::High` edges contribute to the relationship graph
/// used to count connected components. Modules whose apparent
/// cohesion came from heuristic call-graph bridges will report a
/// lower cohesion score in this view.
pub fn compute_cohesion_high_only(graph: &AnalysisGraph, min_functions: usize) -> CohesionResult {
    compute_cohesion_impl(graph, min_functions, true)
}

/// Compute LCOM4 cohesion for each analysis module.
///
/// Only modules with >= `min_functions` function-type members are scored.
/// Non-production modules (tests, examples, etc.) are skipped.
pub fn compute_cohesion(graph: &AnalysisGraph, min_functions: usize) -> CohesionResult {
    compute_cohesion_impl(graph, min_functions, false)
}

fn compute_cohesion_impl(
    graph: &AnalysisGraph,
    min_functions: usize,
    high_only: bool,
) -> CohesionResult {
    let mut modules = BTreeMap::new();

    for module_key in &graph.folder_module_ids {
        if graph.is_non_production_node(module_key) {
            continue;
        }

        let members = match graph.analysis_module_members_of(module_key) {
            Some(m) => m,
            None => continue,
        };

        let fn_members: Vec<&str> = members
            .iter()
            .filter(|id| {
                graph
                    .nodes
                    .get(id.as_str())
                    .map(|n| n.kind.is_function_like())
                    .unwrap_or(false)
            })
            .map(|s| s.as_str())
            .collect();

        if fn_members.len() < min_functions {
            continue;
        }

        let fn_set: HashSet<&str> = fn_members.iter().copied().collect();
        let components = count_connected_components(graph, &fn_set, high_only);

        let cohesion = if components > 0 {
            1.0 / components as f64
        } else {
            1.0
        };

        modules.insert(
            module_key.clone(),
            ModuleCohesion {
                cohesion_score: cohesion,
                connected_components: components,
                function_count: fn_members.len(),
            },
        );
    }

    CohesionResult { modules }
}

/// Count connected components in the undirected function-relationship subgraph.
///
/// Two functions in the module are connected if:
/// 1. One calls the other (direct Call/Import/Uses/Type edge in either direction)
/// 2. Both call the same function (shared outgoing target)
/// 3. Both are called by the same function (shared incoming source)
fn count_connected_components(
    graph: &AnalysisGraph,
    fn_set: &HashSet<&str>,
    high_only: bool,
) -> usize {
    if fn_set.is_empty() {
        return 0;
    }

    // Filter used by every edge traversal below. When `high_only` is
    // true, heuristic edges don't contribute to the relationship
    // graph — meaning cohesion bridges built only from heuristic
    // dispatch (e.g., an Apex resolver that guessed a dispatch target
    // on name similarity) collapse.
    let edge_visible = |ei: usize| !high_only || graph.edges[ei].confidence.is_high();

    // Build adjacency: fn_id -> set of related fn_ids within the module
    let mut adj: HashMap<&str, HashSet<&str>> = HashMap::new();
    for &fid in fn_set {
        adj.entry(fid).or_default();
    }

    // Relationship 1: direct structural edges between module functions
    for &fid in fn_set {
        if let Some(out_indices) = graph.outgoing.get(fid) {
            for &ei in out_indices {
                if !edge_visible(ei) {
                    continue;
                }
                let target = graph.edges[ei].to_id.as_str();
                if fn_set.contains(target) && target != fid {
                    adj.entry(fid).or_default().insert(target);
                    adj.entry(target).or_default().insert(fid);
                }
            }
        }
    }

    // Relationship 2: shared outgoing target (both call the same function)
    let mut target_to_callers: HashMap<&str, Vec<&str>> = HashMap::new();
    for &fid in fn_set {
        if let Some(out_indices) = graph.outgoing.get(fid) {
            for &ei in out_indices {
                if !edge_visible(ei) {
                    continue;
                }
                let target = graph.edges[ei].to_id.as_str();
                target_to_callers.entry(target).or_default().push(fid);
            }
        }
    }
    for callers in target_to_callers.values() {
        if callers.len() >= 2 {
            for i in 0..callers.len() {
                for j in (i + 1)..callers.len() {
                    adj.entry(callers[i]).or_default().insert(callers[j]);
                    adj.entry(callers[j]).or_default().insert(callers[i]);
                }
            }
        }
    }

    // Relationship 3: shared incoming source (both called by the same function)
    let mut source_to_callees: HashMap<&str, Vec<&str>> = HashMap::new();
    for &fid in fn_set {
        if let Some(in_indices) = graph.incoming.get(fid) {
            for &ei in in_indices {
                if !edge_visible(ei) {
                    continue;
                }
                let source = graph.edges[ei].from_id.as_str();
                source_to_callees.entry(source).or_default().push(fid);
            }
        }
    }
    for callees in source_to_callees.values() {
        if callees.len() >= 2 {
            for i in 0..callees.len() {
                for j in (i + 1)..callees.len() {
                    adj.entry(callees[i]).or_default().insert(callees[j]);
                    adj.entry(callees[j]).or_default().insert(callees[i]);
                }
            }
        }
    }

    // BFS to count connected components
    let mut visited: HashSet<&str> = HashSet::new();
    let mut components = 0;

    for &fid in fn_set {
        if visited.contains(fid) {
            continue;
        }
        components += 1;
        let mut queue = vec![fid];
        while let Some(current) = queue.pop() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(neighbors) = adj.get(current) {
                for &neighbor in neighbors {
                    if !visited.contains(neighbor) {
                        queue.push(neighbor);
                    }
                }
            }
        }
    }

    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::graph::*;
    use std::collections::BTreeMap;

    fn make_graph(
        node_specs: Vec<(&str, NodeKind, Option<&str>)>,
        edges: Vec<(&str, &str, EdgeKind)>,
    ) -> AnalysisGraph {
        let mut nodes = BTreeMap::new();
        for (id, kind, path) in &node_specs {
            nodes.insert(
                id.to_string(),
                GraphNode {
                    id: id.to_string(),
                    kind: *kind,
                    fqn: format!("test::{id}"),
                    name: id.to_string(),
                    file_path: None,
                    start_line: None,
                    end_line: None,
                    path_repo_rel: path.map(|p| p.to_string()),
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
                },
            );
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
        g
    }

    #[test]
    fn single_component_all_connected() {
        // A calls B, B calls C -> all in one component
        let g = make_graph(
            vec![
                ("file", NodeKind::File, Some("src/mod/file.ts")),
                ("A", NodeKind::Function, None),
                ("B", NodeKind::Function, None),
                ("C", NodeKind::Function, None),
            ],
            vec![
                ("file", "A", EdgeKind::Contains),
                ("file", "B", EdgeKind::Contains),
                ("file", "C", EdgeKind::Contains),
                ("A", "B", EdgeKind::Call),
                ("B", "C", EdgeKind::Call),
            ],
        );

        let fn_set: HashSet<&str> = ["A", "B", "C"].iter().copied().collect();
        assert_eq!(count_connected_components(&g, &fn_set, false), 1);
    }

    #[test]
    fn two_disconnected_groups() {
        // A calls B, C calls D, no connection between groups
        let g = make_graph(
            vec![
                ("file", NodeKind::File, Some("src/mod/file.ts")),
                ("A", NodeKind::Function, None),
                ("B", NodeKind::Function, None),
                ("C", NodeKind::Function, None),
                ("D", NodeKind::Function, None),
            ],
            vec![
                ("file", "A", EdgeKind::Contains),
                ("file", "B", EdgeKind::Contains),
                ("file", "C", EdgeKind::Contains),
                ("file", "D", EdgeKind::Contains),
                ("A", "B", EdgeKind::Call),
                ("C", "D", EdgeKind::Call),
            ],
        );

        let fn_set: HashSet<&str> = ["A", "B", "C", "D"].iter().copied().collect();
        assert_eq!(count_connected_components(&g, &fn_set, false), 2);
    }

    #[test]
    fn shared_dependency_connects() {
        // A and B both call external X -> connected via shared dependency
        let g = make_graph(
            vec![
                ("file", NodeKind::File, Some("src/mod/file.ts")),
                ("A", NodeKind::Function, None),
                ("B", NodeKind::Function, None),
                ("X", NodeKind::Function, None),
            ],
            vec![
                ("file", "A", EdgeKind::Contains),
                ("file", "B", EdgeKind::Contains),
                ("A", "X", EdgeKind::Call),
                ("B", "X", EdgeKind::Call),
            ],
        );

        let fn_set: HashSet<&str> = ["A", "B"].iter().copied().collect();
        assert_eq!(count_connected_components(&g, &fn_set, false), 1);
    }

    #[test]
    fn shared_consumer_connects() {
        // External X calls both A and B -> connected via shared consumer
        let g = make_graph(
            vec![
                ("file", NodeKind::File, Some("src/mod/file.ts")),
                ("A", NodeKind::Function, None),
                ("B", NodeKind::Function, None),
                ("X", NodeKind::Function, None),
            ],
            vec![
                ("file", "A", EdgeKind::Contains),
                ("file", "B", EdgeKind::Contains),
                ("X", "A", EdgeKind::Call),
                ("X", "B", EdgeKind::Call),
            ],
        );

        let fn_set: HashSet<&str> = ["A", "B"].iter().copied().collect();
        assert_eq!(count_connected_components(&g, &fn_set, false), 1);
    }

    #[test]
    fn isolated_functions() {
        // Three functions with no edges -> 3 components
        let g = make_graph(
            vec![
                ("file", NodeKind::File, Some("src/mod/file.ts")),
                ("A", NodeKind::Function, None),
                ("B", NodeKind::Function, None),
                ("C", NodeKind::Function, None),
            ],
            vec![
                ("file", "A", EdgeKind::Contains),
                ("file", "B", EdgeKind::Contains),
                ("file", "C", EdgeKind::Contains),
            ],
        );

        let fn_set: HashSet<&str> = ["A", "B", "C"].iter().copied().collect();
        assert_eq!(count_connected_components(&g, &fn_set, false), 3);
    }

    #[test]
    fn empty_set() {
        let g = make_graph(vec![], vec![]);
        let fn_set: HashSet<&str> = HashSet::new();
        assert_eq!(count_connected_components(&g, &fn_set, false), 0);
    }
}
