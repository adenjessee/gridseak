//! Analysis segment runner (S2-γ).

use std::collections::HashSet;

use anyhow::Result;

use super::super::super::progress;
use super::super::super::report::*;
use super::super::super::{blast_radius, depth, layers};
use super::super::super::{build_hotspot_findings, layer_label_for_depth, run_safe};
use super::super::session::AnalysisRunContext;

pub fn run(ctx: &mut AnalysisRunContext<'_>) -> Result<Option<HealthReport>> {
    // --- 5. Blast radius ---
    progress::emit_progress("blast_radius", 65, "running blast radius computation");
    eprintln!("[ge-analyze] Running blast radius computation...");
    ctx.blast_result = run_safe("blast_radius", &mut ctx.analysis_errors, || {
        blast_radius::compute_blast_radius(&ctx.graph)
    });

    if let Some(ref br) = ctx.blast_result {
        for (id, &radius) in &br.radii {
            if let Some(ann) = ctx.node_annotations.get_mut(id) {
                ann.blast_radius = radius;
            }
        }

        // Generate hotspot ctx.findings (nodes that are both hotspots and have high blast radius)
        if let Some(ref fr) = ctx.fan_result {
            let mut hotspot_findings = build_hotspot_findings(&ctx.graph, fr, br);
            ctx.findings.append(&mut hotspot_findings);
        }
    }

    // --- 6. Depth ---
    progress::emit_progress("depth", 72, "running depth analysis");
    eprintln!("[ge-analyze] Running depth analysis...");
    ctx.depth_result = run_safe("depth", &mut ctx.analysis_errors, || {
        depth::compute_depth(&ctx.graph, &ctx.cycle_node_set)
    });

    if let Some(ref dr) = ctx.depth_result {
        for (id, &d) in &dr.depth_from_root {
            if let Some(ann) = ctx.node_annotations.get_mut(id) {
                ann.depth_from_root = d;
                ann.inferred_layer = Some(d);
                ann.layer_label = Some(layer_label_for_depth(d, dr.max_call_depth).into());
            }
        }

        if dr.max_call_depth > ctx.config.thresholds.depth_warning {
            let severity = if dr.max_call_depth > ctx.config.thresholds.depth_critical {
                Severity::Critical
            } else if dr.max_call_depth > ctx.config.thresholds.depth_high {
                Severity::High
            } else {
                Severity::Warning
            };
            ctx.findings.push(Finding {
                id: "depth-1".into(),
                finding_type: FindingType::DeepCallChain,
                severity,
                description: format!(
                    "Maximum call chain depth of {} detected",
                    dr.max_call_depth
                ),
                detail: Some(
                    "Deep call chains indicate long sequences of function calls from entry points to leaf functions. \
                     They increase cognitive load when debugging and make it harder to trace execution flow."
                        .into(),
                ),
                node_ids: vec![],
                edge_ids: None,
                primary_node_id: None,
                metric_name: Some("max_call_depth".into()),
                metric_value: Some(dr.max_call_depth as f64),
                impact: None,
                blast_radius: None,
                recommendation: Some(
                    "Deep call chains increase cognitive complexity and make debugging harder. Consider flattening orchestration."
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
            });
        }
    }

    // --- 6.5. Layer violation detection ---
    ctx.layer_result = if let Some(ref dr) = ctx.depth_result {
        progress::emit_progress("layer_violations", 78, "running layer violation detection");
        eprintln!("[ge-analyze] Running layer violation detection...");

        // Compute high-fan-in exclusion set: callees in the top N percentile
        // are universal utilities (e.g. .len(), .to_string()) whose deep layer
        // assignment produces noise, not meaningful architectural violations.
        let high_fan_in_nodes = {
            let mut fan_ins: Vec<usize> = ctx
                .graph
                .function_node_ids
                .iter()
                .map(|id| ctx.graph.fan_in(id))
                .filter(|&fi| fi > 0)
                .collect();
            fan_ins.sort_unstable();
            let threshold = if fan_ins.is_empty() {
                usize::MAX
            } else {
                let pct = ctx
                    .config
                    .thresholds
                    .layer_violation_fan_in_exclude_percentile;
                let idx = (fan_ins.len() * pct / 100).min(fan_ins.len() - 1);
                fan_ins[idx]
            };
            ctx.graph
                .function_node_ids
                .iter()
                .filter(|id| ctx.graph.fan_in(id) > threshold)
                .cloned()
                .collect::<HashSet<String>>()
        };

        let lr = layers::detect_layer_violations(
            &ctx.graph,
            &dr.depth_from_root,
            &ctx.cycle_node_set,
            &high_fan_in_nodes,
            ctx.config.thresholds.layer_violation_min_gap,
        );
        eprintln!(
            "[ge-analyze] {} layer violations detected (max layer {}).",
            lr.violations.len(),
            lr.max_layer,
        );

        for (idx, v) in lr.violations.iter().enumerate() {
            if ctx.graph.is_non_production_node(&v.caller_id)
                || ctx.graph.is_non_production_node(&v.callee_id)
            {
                continue;
            }

            let caller_name = ctx
                .graph
                .nodes
                .get(v.caller_id.as_str())
                .map(|n| n.display_name())
                .unwrap_or_else(|| "unknown".to_string());
            let callee_name = ctx
                .graph
                .nodes
                .get(v.callee_id.as_str())
                .map(|n| n.display_name())
                .unwrap_or_else(|| "unknown".to_string());

            let severity = if v.layer_gap >= ctx.config.thresholds.layer_violation_critical_gap {
                Severity::Critical
            } else if v.layer_gap >= ctx.config.thresholds.layer_violation_high_gap {
                Severity::High
            } else {
                Severity::Warning
            };

            ctx.findings.push(Finding {
                id: format!("layer-{}", idx + 1),
                finding_type: FindingType::LayerViolation,
                severity,
                description: format!(
                    "{caller_name} (layer {}) calls {callee_name} (layer {}), skipping {} layer(s)",
                    v.caller_layer, v.callee_layer, v.layer_gap - 1,
                ),
                detail: Some(
                    "Layer violations occur when higher-level code bypasses intermediate abstraction layers to call lower-level code directly. \
                     This undermines architectural boundaries and makes the system harder to evolve."
                        .into(),
                ),
                node_ids: vec![v.caller_id.clone(), v.callee_id.clone()],
                edge_ids: None,
                primary_node_id: Some(v.caller_id.clone()),
                metric_name: Some("layer_gap".into()),
                metric_value: Some(v.layer_gap as f64),
                impact: None,
                blast_radius: None,
                recommendation: Some(
                    "This function bypasses intermediate abstraction layers. Route the call through the appropriate service/middleware layer."
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
            });
        }

        Some(lr)
    } else {
        None
    };

    Ok(None)
}
