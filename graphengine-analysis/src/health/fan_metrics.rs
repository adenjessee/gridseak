//! Fan-in, fan-out, and hotspot detection.
//!
//! Counts incoming/outgoing non-containment edges per node.
//! Flags hotspots at the 95th percentile of fan-in among function nodes.

use super::graph::AnalysisGraph;

#[derive(Debug, Clone)]
pub struct FanMetrics {
    pub fan_in: usize,
    pub fan_out: usize,
    pub is_hotspot: bool,
}

#[derive(Debug)]
pub struct FanResult {
    /// Per-node fan metrics, keyed by node ID.
    pub metrics: std::collections::HashMap<String, FanMetrics>,
    /// The fan_in threshold used for hotspot detection.
    pub hotspot_threshold: usize,
    /// Number of hotspot nodes.
    pub hotspot_count: usize,
    /// Sum of fan_in across all hotspot nodes.
    pub sum_hotspot_fan_in: usize,
    /// Sum of fan_in across all nodes.
    pub total_fan_in: usize,
}

pub fn compute_fan_metrics(graph: &AnalysisGraph) -> FanResult {
    compute_fan_metrics_with_config(graph, 95, 20, 8, false)
}

/// High-confidence-only variant used by T3 dual-metric emission.
/// Each node's fan-in / fan-out is computed via `fan_in_high_only` /
/// `fan_out_high_only`, which drop heuristic edges. Hotspot detection
/// then runs on the reduced counts.
pub fn compute_fan_metrics_high_only(graph: &AnalysisGraph) -> FanResult {
    compute_fan_metrics_with_config(graph, 95, 20, 8, true)
}

pub fn compute_fan_metrics_with_config(
    graph: &AnalysisGraph,
    hotspot_percentile: usize,
    small_graph_threshold: usize,
    small_graph_fixed: usize,
    high_only: bool,
) -> FanResult {
    let mut metrics = std::collections::HashMap::new();

    for id in graph.nodes.keys() {
        let (fi, fo) = if high_only {
            (graph.fan_in_high_only(id), graph.fan_out_high_only(id))
        } else {
            (graph.fan_in(id), graph.fan_out(id))
        };
        metrics.insert(
            id.clone(),
            FanMetrics {
                fan_in: fi,
                fan_out: fo,
                is_hotspot: false,
            },
        );
    }

    // Compute hotspot threshold: 95th percentile of fan_in among function nodes
    let mut fn_fan_ins: Vec<usize> = graph
        .function_node_ids
        .iter()
        .filter_map(|id| metrics.get(id).map(|m| m.fan_in))
        .collect();

    fn_fan_ins.sort_unstable();

    let hotspot_threshold = if fn_fan_ins.len() < small_graph_threshold {
        small_graph_fixed
    } else {
        let idx_95 =
            (fn_fan_ins.len() as f64 * (hotspot_percentile as f64 / 100.0)).ceil() as usize;
        let idx = idx_95.min(fn_fan_ins.len() - 1);
        fn_fan_ins[idx]
    };

    // Flag hotspots
    let mut hotspot_count = 0;
    let mut sum_hotspot_fan_in = 0;
    let mut total_fan_in = 0;

    for id in &graph.function_node_ids {
        if let Some(m) = metrics.get_mut(id) {
            total_fan_in += m.fan_in;
            if m.fan_in >= hotspot_threshold && hotspot_threshold > 0 {
                m.is_hotspot = true;
                hotspot_count += 1;
                sum_hotspot_fan_in += m.fan_in;
            }
        }
    }

    FanResult {
        metrics,
        hotspot_threshold,
        hotspot_count,
        sum_hotspot_fan_in,
        total_fan_in,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::graph::*;
    use std::collections::BTreeMap;

    fn make_graph(node_specs: Vec<(&str, NodeKind)>, edges: Vec<(&str, &str)>) -> AnalysisGraph {
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
            .map(|(f, t)| GraphEdge {
                from_id: f.to_string(),
                to_id: t.to_string(),
                kind: EdgeKind::Call,
                confidence: crate::health::graph::Confidence::High,
            })
            .collect();
        let mut g = AnalysisGraph::build(nodes, edges);
        g.compute_module_membership();
        g
    }

    #[test]
    fn isolated_node() {
        let g = make_graph(vec![("A", NodeKind::Function)], vec![]);
        let result = compute_fan_metrics(&g);
        let m = &result.metrics["A"];
        assert_eq!(m.fan_in, 0);
        assert_eq!(m.fan_out, 0);
    }

    #[test]
    fn linear_chain() {
        let g = make_graph(
            vec![
                ("A", NodeKind::Function),
                ("B", NodeKind::Function),
                ("C", NodeKind::Function),
            ],
            vec![("A", "B"), ("B", "C")],
        );
        let result = compute_fan_metrics(&g);
        assert_eq!(result.metrics["A"].fan_in, 0);
        assert_eq!(result.metrics["A"].fan_out, 1);
        assert_eq!(result.metrics["B"].fan_in, 1);
        assert_eq!(result.metrics["B"].fan_out, 1);
        assert_eq!(result.metrics["C"].fan_in, 1);
        assert_eq!(result.metrics["C"].fan_out, 0);
    }

    #[test]
    fn fan_in_sink() {
        let g = make_graph(
            vec![
                ("A", NodeKind::Function),
                ("B", NodeKind::Function),
                ("C", NodeKind::Function),
                ("D", NodeKind::Function),
            ],
            vec![("B", "A"), ("C", "A"), ("D", "A")],
        );
        let result = compute_fan_metrics(&g);
        assert_eq!(result.metrics["A"].fan_in, 3);
        assert_eq!(result.metrics["A"].fan_out, 0);
    }
}
