//! L1 fast path: reuse cached global segments, rerun complexity only.

use std::time::Instant;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags};

use super::super::complexity;
use super::super::config::AnalysisConfig;
use super::super::graph::AnalysisGraph;
use super::super::health_score::{self, ScoreInputs};
use super::super::report::{
    AnalysisProvenance, FindingType, HealthReport, CAVEAT_INCREMENTAL_ANALYSIS_SEGMENTS_MERGED_V1,
};
use super::cache::read_segment_cache;
use super::merge::HealthScoreSegmentPayload;
use super::scope::AnalysisDelta;
use super::segments::AnalysisSegment;

fn node_in_changed_paths(graph: &AnalysisGraph, node_id: &str, changed_paths: &[String]) -> bool {
    let Some(fp) = graph
        .nodes
        .get(node_id)
        .and_then(|n| n.file_path.as_deref())
    else {
        return false;
    };
    changed_paths
        .iter()
        .any(|cp| cp == fp || fp.ends_with(cp.as_str()) || cp.ends_with(fp))
}

fn filter_function_ids_to_changed_paths(
    graph: &AnalysisGraph,
    delta: &AnalysisDelta,
) -> Vec<String> {
    if delta.changed_paths.is_empty() {
        return graph.function_node_ids.clone();
    }
    graph
        .function_node_ids
        .iter()
        .filter(|id| node_in_changed_paths(graph, id, &delta.changed_paths))
        .cloned()
        .collect()
}

pub fn try_l1_fast_merge(
    conn: &Connection,
    db_path: &str,
    structure_fp: &str,
    delta: &AnalysisDelta,
    config: Option<AnalysisConfig>,
) -> Result<Option<HealthReport>> {
    let Some(raw) = read_segment_cache(conn, AnalysisSegment::HealthScore.as_str(), structure_fp)?
    else {
        return Ok(None);
    };

    let payload: HealthScoreSegmentPayload = serde_json::from_str(&raw)?;
    let start = Instant::now();

    let graph_conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let detected = super::super::graph::detect_ecosystem(&graph_conn);
    let cfg = match config {
        Some(mut c) => {
            if c.ecosystem.is_none() {
                c.ecosystem = Some(detected);
            }
            c
        }
        None => AnalysisConfig::for_ecosystem(detected),
    };

    let mut graph = AnalysisGraph::load_with_module_config(
        &graph_conn,
        cfg.modules.analysis_depth,
        cfg.modules.strip_build_convention_dirs,
    )?;

    let scoped_fn_ids = filter_function_ids_to_changed_paths(&graph, delta);
    if !delta.changed_paths.is_empty() {
        graph.function_node_ids = scoped_fn_ids.clone();
    }
    let complexity_result = complexity::analyze_complexity(&graph, &cfg.thresholds);
    let elapsed = start.elapsed().as_millis() as u64;

    let mut report = payload.report;
    report.db_path = db_path.to_string();
    report.generated_at = chrono::Utc::now().to_rfc3339();
    report.analysis_duration_ms = elapsed;

    if delta.changed_paths.is_empty() {
        report
            .findings
            .retain(|f| f.finding_type != FindingType::ExcessiveComplexity);
    } else {
        report.findings.retain(|f| {
            f.finding_type != FindingType::ExcessiveComplexity
                || f.primary_node_id
                    .as_ref()
                    .is_none_or(|id| !node_in_changed_paths(&graph, id, &delta.changed_paths))
        });
    }
    report.findings.extend(complexity_result.findings.clone());
    report.findings.sort_by(|a, b| a.id.cmp(&b.id));

    if delta.changed_paths.is_empty() {
        if let Some(ref mut metrics) = report.metrics.complexity {
            metrics.avg_cyclomatic = complexity_result.avg_cyclomatic;
            metrics.avg_cognitive = complexity_result.avg_cognitive;
            metrics.max_cyclomatic = complexity_result.max_cyclomatic;
            metrics.max_cognitive = complexity_result.max_cognitive;
            metrics.functions_above_threshold = complexity_result.functions_above_threshold;
            metrics.description = format!(
                "Average cyclomatic complexity {:.1} ({} functions above warning threshold)",
                complexity_result.avg_cyclomatic, complexity_result.functions_above_threshold
            );
        }
    }

    let annotation_ids = if delta.changed_paths.is_empty() {
        graph.function_node_ids.clone()
    } else {
        scoped_fn_ids
    };
    for id in &annotation_ids {
        if let Some(node) = graph.nodes.get(id) {
            if let Some(ann) = report.node_annotations.get_mut(id) {
                ann.cyclomatic_complexity = node.cyclomatic_complexity;
                ann.cognitive_complexity = node.cognitive_complexity;
            }
        }
    }

    let score_inputs = ScoreInputs {
        total_nodes: report.summary.total_nodes,
        total_cycle_nodes: report.summary.cycle_total_nodes,
        avg_coupling_score: report.metrics.coupling.avg_coupling,
        coupling_baseline: cfg.thresholds.coupling_baseline,
        sum_hotspot_fan_in: report.summary.hotspot_count,
        total_fan_in: report.summary.total_nodes.max(1),
        dead_functions: report.summary.dead_functions,
        total_functions: report.summary.total_functions.max(1),
        max_call_depth: report.metrics.depth.max_call_depth,
        avg_cyclomatic: if delta.changed_paths.is_empty() {
            complexity_result.avg_cyclomatic
        } else {
            report
                .metrics
                .complexity
                .as_ref()
                .map(|c| c.avg_cyclomatic)
                .unwrap_or(complexity_result.avg_cyclomatic)
        },
        avg_cohesion: report
            .metrics
            .cohesion
            .as_ref()
            .map(|c| c.avg_cohesion)
            .unwrap_or(1.0),
        avg_distance: report
            .metrics
            .distance_from_main_sequence
            .as_ref()
            .map(|d| d.avg_distance)
            .unwrap_or(0.0),
        hidden_coupling_pairs: report
            .metrics
            .temporal_coupling
            .as_ref()
            .map(|t| t.hidden_coupling_pairs)
            .unwrap_or(0),
        total_file_pairs_analyzed: report
            .metrics
            .temporal_coupling
            .as_ref()
            .and_then(|t| t.module_level.as_ref().map(|m| m.total_module_pairs))
            .unwrap_or(0),
    };
    let (hs_value, components) =
        health_score::score_from_formulas(&cfg.score_weights, &score_inputs);
    report.health_score = Some(hs_value);
    report.health_score_components = components;

    if !report
        .integrity_status
        .schema_caveats
        .iter()
        .any(|c| c == CAVEAT_INCREMENTAL_ANALYSIS_SEGMENTS_MERGED_V1)
    {
        report
            .integrity_status
            .schema_caveats
            .push(CAVEAT_INCREMENTAL_ANALYSIS_SEGMENTS_MERGED_V1.to_string());
    }

    let reused = [
        AnalysisSegment::Cycles,
        AnalysisSegment::FanMetrics,
        AnalysisSegment::ModuleCoupling,
        AnalysisSegment::DeadCode,
        AnalysisSegment::BlastRadius,
        AnalysisSegment::FindingsAssembly,
    ];
    report.analysis_provenance = Some(AnalysisProvenance {
        analysis_mode: "segmented_sync".into(),
        trust_level: "L1".into(),
        structure_fingerprint: structure_fp.to_string(),
        structure_changed: false,
        segments_reused: reused.iter().map(|s| s.as_str().to_string()).collect(),
        segments_rerun: vec![
            AnalysisSegment::GraphPrep.as_str().into(),
            AnalysisSegment::Complexity.as_str().into(),
            AnalysisSegment::HealthScore.as_str().into(),
        ],
        changed_paths: delta.changed_paths.clone(),
        delta_fingerprint: String::new(),
        query_trust_note: None,
    });

    Ok(Some(report))
}
