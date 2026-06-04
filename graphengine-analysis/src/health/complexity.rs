//! Complexity analysis: reads cyclomatic/cognitive values from parsed node
//! properties and generates excessive-complexity findings.
//!
//! The complexity values themselves are computed in the parsing crate during
//! tree-sitter AST traversal. This module aggregates and reports on them.

use super::config::ThresholdConfig;
use super::graph::AnalysisGraph;
use super::report::{Finding, FindingType, Severity};

#[derive(Debug)]
pub struct ComplexityResult {
    pub findings: Vec<Finding>,
    pub avg_cyclomatic: f64,
    pub avg_cognitive: f64,
    pub max_cyclomatic: u32,
    pub max_cognitive: u32,
    pub functions_above_threshold: usize,
    pub functions_measured: usize,
}

pub fn analyze_complexity(graph: &AnalysisGraph, thresholds: &ThresholdConfig) -> ComplexityResult {
    use rayon::prelude::*;

    let mut sum_cyclomatic: u64 = 0;
    let mut sum_cognitive: u64 = 0;
    let mut max_cyclomatic: u32 = 0;
    let mut max_cognitive: u32 = 0;
    let mut measured: usize = 0;

    for id in &graph.function_node_ids {
        if graph.is_non_production_node(id) {
            continue;
        }
        let node = match graph.nodes.get(id) {
            Some(n) => n,
            None => continue,
        };
        let cc = match node.cyclomatic_complexity {
            Some(v) => v,
            None => continue,
        };
        let cog = node.cognitive_complexity.unwrap_or(0);
        measured += 1;
        sum_cyclomatic += cc as u64;
        sum_cognitive += cog as u64;
        max_cyclomatic = max_cyclomatic.max(cc);
        max_cognitive = max_cognitive.max(cog);
    }

    let mut raw_findings: Vec<(String, Finding)> = graph
        .function_node_ids
        .par_iter()
        .filter_map(|id| {
            if graph.is_non_production_node(id) {
                return None;
            }
            let node = graph.nodes.get(id)?;
            let cc = node.cyclomatic_complexity?;
            let cog = node.cognitive_complexity.unwrap_or(0);
            let sev = classify_severity(cc, cog, thresholds)?;
            let loc = node_loc(node);
            let name = node.display_name();
            Some((
                id.clone(),
                Finding {
                    id: String::new(),
                    finding_type: FindingType::ExcessiveComplexity,
                    severity: sev,
                    description: format!(
                        "{name}: cyclomatic complexity {cc}, cognitive complexity {cog} ({loc} lines)"
                    ),
                    detail: Some(
                        "Excessive cyclomatic or cognitive complexity indicates many branching paths and nested logic. \
                         Such functions are harder to test, understand, and modify — consider extracting smaller units."
                            .into(),
                    ),
                    node_ids: vec![id.clone()],
                    edge_ids: None,
                    primary_node_id: Some(id.clone()),
                    metric_name: Some("cyclomatic_complexity".into()),
                    metric_value: Some(cc as f64),
                    impact: None,
                    blast_radius: None,
                    recommendation: Some(
                        "Extract conditional branches into helper functions. \
                         Consider a strategy pattern for complex switches."
                            .into(),
                    ),
                    cycle_length: None,
                    fan_in: None,
                    coupling_score: None,
                    internal_edges: None,
                    external_edges: None,
                    count: None,
                    hub_score: None,
                    file_a: None,
                    file_b: None,
                    co_change_count: None,
                    temporal_coupling_score: None,
                    has_import_edge: None,
                    confidence: None,
                },
            ))
        })
        .collect();

    raw_findings.sort_by(|a, b| a.0.cmp(&b.0));
    let above_threshold = raw_findings.len();
    let findings: Vec<Finding> = raw_findings
        .into_iter()
        .enumerate()
        .map(|(idx, (_, mut f))| {
            f.id = format!("complexity-{}", idx + 1);
            f
        })
        .collect();

    ComplexityResult {
        findings,
        avg_cyclomatic: if measured > 0 {
            sum_cyclomatic as f64 / measured as f64
        } else {
            0.0
        },
        avg_cognitive: if measured > 0 {
            sum_cognitive as f64 / measured as f64
        } else {
            0.0
        },
        max_cyclomatic,
        max_cognitive,
        functions_above_threshold: above_threshold,
        functions_measured: measured,
    }
}

fn classify_severity(cc: u32, cog: u32, t: &ThresholdConfig) -> Option<Severity> {
    if cc > t.cyclomatic_critical || cog > t.cognitive_critical {
        Some(Severity::Critical)
    } else if cc > t.cyclomatic_high || cog > t.cognitive_high {
        Some(Severity::High)
    } else if cc > t.cyclomatic_warning || cog > t.cognitive_warning {
        Some(Severity::Warning)
    } else {
        None
    }
}

fn node_loc(node: &super::graph::GraphNode) -> usize {
    match (node.start_line, node.end_line) {
        (Some(s), Some(e)) if e >= s => (e - s + 1) as usize,
        _ => 0,
    }
}
