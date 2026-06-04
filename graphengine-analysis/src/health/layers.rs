//! Automatic layer detection and violation analysis.
//!
//! Assigns a BFS-derived layer depth to every function from entry points (roots).
//! A **layer violation** is a Call edge that jumps across 2+ layers — the caller
//! bypasses an intermediate abstraction layer to reach a deeper implementation.
//!
//! Entry points (layer 0) are functions with zero incoming Call edges.
//! Depth is derived from the longest path from any root (matching depth.rs semantics).

use std::collections::{HashMap, HashSet};

use super::graph::AnalysisGraph;

#[derive(Debug, Clone)]
pub struct LayerViolation {
    pub caller_id: String,
    pub callee_id: String,
    pub caller_layer: usize,
    pub callee_layer: usize,
    pub layer_gap: usize,
}

#[derive(Debug, Clone)]
pub struct ModuleLayerStats {
    pub avg_layer_depth: f64,
    pub violation_count: usize,
}

#[derive(Debug)]
pub struct LayerResult {
    pub violations: Vec<LayerViolation>,
    pub max_layer: usize,
    pub functions_with_layer: usize,
    pub module_stats: HashMap<String, ModuleLayerStats>,
}

/// Detect layer violations using pre-computed depth data.
///
/// `depth_from_root` maps function node IDs to their BFS depth from roots.
/// `cycle_nodes` are excluded from violation detection (depth may be unreliable).
/// `high_fan_in_nodes` are excluded because they represent universal utility
/// functions (`.len()`, `.to_string()`, etc.) whose deep layer position produces
/// noise rather than meaningful architectural violations.
/// `min_gap` is the minimum layer distance to consider a violation (typically 2).
pub fn detect_layer_violations(
    graph: &AnalysisGraph,
    depth_from_root: &HashMap<String, usize>,
    cycle_nodes: &HashSet<String>,
    high_fan_in_nodes: &HashSet<String>,
    min_gap: usize,
) -> LayerResult {
    let mut violations = Vec::new();
    let max_layer = depth_from_root.values().copied().max().unwrap_or(0);

    for &ei in &graph.clean_structural_edge_indices {
        let edge = &graph.edges[ei];
        // Layer violations are a call-reach property; include framework
        // and declarative dispatch alongside `Call`. See
        // DISCOVERY_REPORT.md §8 Decision 5. Framework dispatch from a
        // shallow layer into a deep layer is just as much a layering
        // violation as a direct call; T1 stops hiding these cases.
        if !edge.kind.is_call_like() {
            continue;
        }

        if cycle_nodes.contains(edge.from_id.as_str()) || cycle_nodes.contains(edge.to_id.as_str())
        {
            continue;
        }

        // Skip calls to universal utility functions (high fan-in nodes like .len())
        if high_fan_in_nodes.contains(edge.to_id.as_str()) {
            continue;
        }

        let caller_layer = match depth_from_root.get(edge.from_id.as_str()) {
            Some(&d) => d,
            None => continue,
        };
        let callee_layer = match depth_from_root.get(edge.to_id.as_str()) {
            Some(&d) => d,
            None => continue,
        };

        // Only flag forward violations: caller at higher layer calling deeper layer.
        // The gap must exceed min_gap (caller at layer 0 calling layer 2 = gap of 2).
        if callee_layer > caller_layer {
            let gap = callee_layer - caller_layer;
            if gap >= min_gap {
                violations.push(LayerViolation {
                    caller_id: edge.from_id.clone(),
                    callee_id: edge.to_id.clone(),
                    caller_layer,
                    callee_layer,
                    layer_gap: gap,
                });
            }
        }
    }

    violations.sort_by(|a, b| {
        b.layer_gap
            .cmp(&a.layer_gap)
            .then_with(|| a.caller_id.cmp(&b.caller_id))
    });

    // Compute per-module layer statistics
    let mut module_depths: HashMap<String, Vec<usize>> = HashMap::new();
    for (node_id, &depth) in depth_from_root {
        if let Some(folder) = graph.folder_of(node_id) {
            module_depths.entry(folder.clone()).or_default().push(depth);
        }
    }

    let mut module_violation_counts: HashMap<String, usize> = HashMap::new();
    for v in &violations {
        if let Some(folder) = graph.folder_of(&v.caller_id) {
            *module_violation_counts.entry(folder.clone()).or_default() += 1;
        }
    }

    let mut module_stats = HashMap::new();
    for (module_key, depths) in &module_depths {
        let avg = if depths.is_empty() {
            0.0
        } else {
            depths.iter().sum::<usize>() as f64 / depths.len() as f64
        };
        module_stats.insert(
            module_key.clone(),
            ModuleLayerStats {
                avg_layer_depth: avg,
                violation_count: module_violation_counts
                    .get(module_key)
                    .copied()
                    .unwrap_or(0),
            },
        );
    }

    LayerResult {
        violations,
        max_layer,
        functions_with_layer: depth_from_root.len(),
        module_stats,
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

    fn depths(edges: &[(&str, &str)]) -> HashMap<String, usize> {
        let g = make_fn_graph(edges.to_vec());
        let dr = crate::health::depth::compute_depth(&g, &HashSet::new());
        dr.depth_from_root
    }

    #[test]
    fn no_violations_in_clean_layers() {
        // A(0) -> B(1) -> C(2): each calls one layer deeper, no violation
        let edges = vec![("A", "B"), ("B", "C")];
        let g = make_fn_graph(edges.clone());
        let dr = depths(&edges);
        let result = detect_layer_violations(&g, &dr, &HashSet::new(), &HashSet::new(), 2);
        assert!(result.violations.is_empty());
        assert_eq!(result.max_layer, 2);
    }

    #[test]
    fn detects_skip_layer() {
        // A(0) -> B(1) -> C(2), A(0) -> C(2): A skips layer 1 to reach C
        let edges = vec![("A", "B"), ("B", "C"), ("A", "C")];
        let g = make_fn_graph(edges.clone());
        let dr = depths(&edges);
        let result = detect_layer_violations(&g, &dr, &HashSet::new(), &HashSet::new(), 2);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].caller_id, "A");
        assert_eq!(result.violations[0].callee_id, "C");
        assert_eq!(result.violations[0].layer_gap, 2);
    }

    #[test]
    fn detects_deep_skip() {
        // A(0) -> B(1) -> C(2) -> D(3), A(0) -> D(3): gap of 3
        let edges = vec![("A", "B"), ("B", "C"), ("C", "D"), ("A", "D")];
        let g = make_fn_graph(edges.clone());
        let dr = depths(&edges);
        let result = detect_layer_violations(&g, &dr, &HashSet::new(), &HashSet::new(), 2);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].layer_gap, 3);
    }

    #[test]
    fn cycle_nodes_excluded() {
        let edges = vec![("A", "B"), ("B", "C"), ("C", "A"), ("A", "C")];
        let g = make_fn_graph(edges.clone());
        let mut cycle_nodes = HashSet::new();
        cycle_nodes.insert("A".to_string());
        cycle_nodes.insert("B".to_string());
        cycle_nodes.insert("C".to_string());
        let dr = depths(&edges);
        let result = detect_layer_violations(&g, &dr, &cycle_nodes, &HashSet::new(), 2);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn min_gap_adjustable() {
        // A(0) -> B(1) -> C(2), A(0) -> C(2): gap is 2
        let edges = vec![("A", "B"), ("B", "C"), ("A", "C")];
        let g = make_fn_graph(edges.clone());
        let dr = depths(&edges);

        // With min_gap=3, no violation
        let result = detect_layer_violations(&g, &dr, &HashSet::new(), &HashSet::new(), 3);
        assert!(result.violations.is_empty());

        // With min_gap=2, one violation
        let result = detect_layer_violations(&g, &dr, &HashSet::new(), &HashSet::new(), 2);
        assert_eq!(result.violations.len(), 1);
    }

    #[test]
    fn high_fan_in_nodes_excluded() {
        // A(0) -> B(1) -> C(2), A(0) -> C(2): normally a violation
        // But if C is a high-fan-in utility, the violation should be suppressed
        let edges = vec![("A", "B"), ("B", "C"), ("A", "C")];
        let g = make_fn_graph(edges.clone());
        let dr = depths(&edges);

        let mut high_fi = HashSet::new();
        high_fi.insert("C".to_string());

        let result = detect_layer_violations(&g, &dr, &HashSet::new(), &high_fi, 2);
        assert!(result.violations.is_empty());
    }
}
