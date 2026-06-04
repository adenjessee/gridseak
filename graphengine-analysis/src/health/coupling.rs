//! Module coupling analysis.
//!
//! For each module, computes the ratio of cross-boundary edges to total edges.
//! Only modules with >= 3 internal nodes are scored (smaller modules produce noise).

use std::collections::BTreeMap;

use super::graph::AnalysisGraph;
use super::path_classification;

#[derive(Debug, Clone)]
pub struct ModuleCoupling {
    pub coupling_score: f64,
    pub internal_edges: usize,
    pub external_edges: usize,
}

#[derive(Debug)]
pub struct CouplingResult {
    /// Keyed by module id. `BTreeMap` — not `HashMap` — so every
    /// downstream iteration (finding generation, avg_coupling
    /// summation) sees modules in a canonical lexical order. See
    /// R35 in FOLLOWUP_RISKS.md.
    pub modules: BTreeMap<String, ModuleCoupling>,
    pub avg_coupling: f64,
    /// Average coupling excluding test-only modules (better calibration for health score).
    pub avg_coupling_excluding_tests: f64,
    pub high_coupling_count: usize,
}

/// Compute coupling at analysis-module level (path-prefix-based boundaries).
/// Modules are defined by path prefixes at a configured depth, not individual folders.
pub fn compute_coupling(graph: &AnalysisGraph) -> CouplingResult {
    compute_coupling_with_config(graph, 3, 0.7, 0.5)
}

/// High-confidence-only variant used by T3 dual-metric emission.
/// Counts only `Confidence::High` edges when computing the
/// internal/external split per module. Heuristic edges that were
/// contributing to a module's cross-boundary count drop out, so
/// high-coupling modules whose coupling was dominated by LSP-resolved
/// (authoritative) imports retain their score while modules whose
/// apparent coupling was driven by heuristic name-similarity matches
/// collapse toward zero.
pub fn compute_coupling_high_only(graph: &AnalysisGraph) -> CouplingResult {
    compute_coupling_impl(graph, 3, 0.7, 0.5, true)
}

/// Configurable version of `compute_coupling`.
pub fn compute_coupling_with_config(
    graph: &AnalysisGraph,
    min_module_size: usize,
    high_coupling_threshold: f64,
    test_module_ratio: f64,
) -> CouplingResult {
    compute_coupling_impl(
        graph,
        min_module_size,
        high_coupling_threshold,
        test_module_ratio,
        false,
    )
}

fn compute_coupling_impl(
    graph: &AnalysisGraph,
    min_module_size: usize,
    high_coupling_threshold: f64,
    test_module_ratio: f64,
    high_only: bool,
) -> CouplingResult {
    let mut modules: BTreeMap<String, ModuleCoupling> = BTreeMap::new();
    let mut coupling_sum = 0.0;
    let mut coupling_count = 0;
    let mut non_test_coupling_sum = 0.0;
    let mut non_test_coupling_count = 0;
    let mut high_coupling_count = 0;

    for module_key in &graph.folder_module_ids {
        let members = match graph.analysis_module_members_of(module_key) {
            Some(m) => m,
            None => continue,
        };

        if members.len() < min_module_size {
            continue;
        }

        let mut internal = 0usize;
        let mut external = 0usize;

        for &ei in &graph.structural_edge_indices {
            let edge = &graph.edges[ei];
            if high_only && !edge.confidence.is_high() {
                continue;
            }
            let from_in = members.contains(&edge.from_id);
            let to_in = members.contains(&edge.to_id);

            if from_in && to_in {
                internal += 1;
            } else if from_in || to_in {
                external += 1;
            }
        }

        let total = internal + external;
        if total == 0 {
            continue;
        }

        let score = external as f64 / total as f64;
        coupling_sum += score;
        coupling_count += 1;

        if !is_non_production_module(module_key, members, graph, test_module_ratio) {
            non_test_coupling_sum += score;
            non_test_coupling_count += 1;
        }

        if score > high_coupling_threshold {
            high_coupling_count += 1;
        }

        modules.insert(
            module_key.clone(),
            ModuleCoupling {
                coupling_score: score,
                internal_edges: internal,
                external_edges: external,
            },
        );
    }

    let avg_coupling = if coupling_count > 0 {
        coupling_sum / coupling_count as f64
    } else {
        0.0
    };

    let avg_coupling_excluding_tests = if non_test_coupling_count > 0 {
        non_test_coupling_sum / non_test_coupling_count as f64
    } else {
        avg_coupling
    };

    CouplingResult {
        modules,
        avg_coupling,
        avg_coupling_excluding_tests,
        high_coupling_count,
    }
}

/// A module is non-production if:
/// 1. Its path matches known test or auxiliary patterns (examples, benchmarks, fixtures), OR
/// 2. A majority of its members have `is_test` classification from the parser.
fn is_non_production_module(
    module_key: &str,
    members: &std::collections::HashSet<String>,
    graph: &AnalysisGraph,
    ratio_threshold: f64,
) -> bool {
    if path_classification::is_non_production_path(module_key) {
        return true;
    }

    let mut test_count = 0usize;
    let mut classified_count = 0usize;
    for id in members {
        if let Some(classification) = graph.classification_of(id) {
            classified_count += 1;
            if classification.is_test {
                test_count += 1;
            }
        }
    }

    classified_count > 0 && test_count as f64 / classified_count as f64 > ratio_threshold
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::graph::*;
    use std::collections::BTreeMap;

    fn make_module_graph(
        node_specs: Vec<(&str, NodeKind)>,
        edges: Vec<(&str, &str, EdgeKind)>,
    ) -> AnalysisGraph {
        let mut nodes = BTreeMap::new();
        for (id, kind) in &node_specs {
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
    fn all_internal_edges() {
        // Module M contains A, B, C. All call edges are internal.
        let g = make_module_graph(
            vec![
                ("M", NodeKind::File),
                ("A", NodeKind::Function),
                ("B", NodeKind::Function),
                ("C", NodeKind::Function),
            ],
            vec![
                ("M", "A", EdgeKind::Contains),
                ("M", "B", EdgeKind::Contains),
                ("M", "C", EdgeKind::Contains),
                ("A", "B", EdgeKind::Call),
                ("B", "C", EdgeKind::Call),
            ],
        );
        let result = compute_coupling(&g);
        if let Some(mc) = result.modules.get("M") {
            assert!((mc.coupling_score - 0.0).abs() < 0.01);
        }
    }

    #[test]
    fn all_external_edges() {
        // Module M contains A, B, C. A calls external X.
        let g = make_module_graph(
            vec![
                ("M", NodeKind::File),
                ("A", NodeKind::Function),
                ("B", NodeKind::Function),
                ("C", NodeKind::Function),
                ("X", NodeKind::Function),
            ],
            vec![
                ("M", "A", EdgeKind::Contains),
                ("M", "B", EdgeKind::Contains),
                ("M", "C", EdgeKind::Contains),
                ("A", "X", EdgeKind::Call),
            ],
        );
        let result = compute_coupling(&g);
        if let Some(mc) = result.modules.get("M") {
            assert!((mc.coupling_score - 1.0).abs() < 0.01);
        }
    }
}
