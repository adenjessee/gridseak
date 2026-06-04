//! Analysis segment runner (S2-γ).

use anyhow::Result;

use super::super::super::progress;
use super::super::super::report::*;
use super::super::super::run_safe;
use super::super::super::{cohesion, complexity, loc};
use super::super::session::AnalysisRunContext;

pub fn run(ctx: &mut AnalysisRunContext<'_>) -> Result<Option<HealthReport>> {
    // --- 12. Complexity analysis ---
    progress::emit_progress("complexity", 85, "running complexity analysis");
    eprintln!("[ge-analyze] Running complexity analysis...");
    ctx.complexity_result = run_safe("complexity", &mut ctx.analysis_errors, || {
        complexity::analyze_complexity(&ctx.graph, &ctx.config.thresholds)
    });

    if let Some(ref cr) = ctx.complexity_result {
        for f in &cr.findings {
            for nid in &f.node_ids {
                if let Some(ann) = ctx.node_annotations.get_mut(nid) {
                    let node = ctx.graph.nodes.get(nid);
                    ann.cyclomatic_complexity = node.and_then(|n| n.cyclomatic_complexity);
                    ann.cognitive_complexity = node.and_then(|n| n.cognitive_complexity);
                }
            }
        }
        ctx.findings.extend(cr.findings.clone());
    }

    // --- 13. LOC computation ---
    ctx.fn_loc = loc::compute_function_loc(&ctx.graph);
    for (id, &l) in &ctx.fn_loc {
        if let Some(ann) = ctx.node_annotations.get_mut(id) {
            ann.loc = l;
        }
    }

    // --- 13.5 God function detection ---
    {
        let mut god_idx = 0;
        for id in &ctx.graph.function_node_ids {
            if ctx.graph.is_non_production_node(id) {
                continue;
            }

            let node = match ctx.graph.nodes.get(id) {
                Some(n) => n,
                None => continue,
            };

            let cyclomatic = node.cyclomatic_complexity.unwrap_or(0);
            let fan_out = ctx.graph.fan_out(id);
            let func_loc = ctx.fn_loc.get(id.as_str()).copied().unwrap_or(0);

            if cyclomatic >= ctx.config.thresholds.god_function_cyclomatic_min
                && fan_out >= ctx.config.thresholds.god_function_fan_out_min
                && func_loc >= ctx.config.thresholds.god_function_loc_min
            {
                god_idx += 1;
                let severity = if cyclomatic
                    >= ctx.config.thresholds.god_function_cyclomatic_critical
                    && fan_out >= ctx.config.thresholds.god_function_fan_out_critical
                    && func_loc >= ctx.config.thresholds.god_function_loc_critical
                {
                    Severity::Critical
                } else if cyclomatic >= ctx.config.thresholds.cyclomatic_high
                    || fan_out >= ctx.config.thresholds.god_function_fan_out_critical
                    || func_loc >= ctx.config.thresholds.god_function_loc_critical
                {
                    Severity::High
                } else {
                    Severity::Warning
                };

                ctx.findings.push(Finding {
                    id: format!("god-{god_idx}"),
                    finding_type: FindingType::GodFunction,
                    severity,
                    description: format!(
                        "{}: cyclomatic={}, fan_out={}, LOC={} — exceeds all three god-function thresholds",
                        node.display_name(), cyclomatic, fan_out, func_loc,
                    ),
                    detail: Some(
                        "God functions combine high cyclomatic complexity, many outgoing calls, and large size. \
                         They concentrate too much logic in one place, making testing and maintenance difficult."
                            .into(),
                    ),
                    node_ids: vec![id.clone()],
                    edge_ids: None,
                    primary_node_id: Some(id.clone()),
                    metric_name: Some("god_function".into()),
                    metric_value: Some(cyclomatic as f64),
                    impact: None,
                    blast_radius: ctx.node_annotations.get(id).map(|a| a.blast_radius),
                    recommendation: Some(
                        "This function is too complex, touches too many dependencies, and is too long. Break it into smaller, focused functions with clear responsibilities."
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
        if god_idx > 0 {
            eprintln!("[ge-analyze] {} god function(s) detected.", god_idx);
        }
    }

    // --- 15. Module cohesion (LCOM4) ---
    progress::emit_progress("module_cohesion", 93, "running module cohesion analysis");
    eprintln!("[ge-analyze] Running module cohesion analysis...");
    ctx.cohesion_result = run_safe("cohesion", &mut ctx.analysis_errors, || {
        cohesion::compute_cohesion(&ctx.graph, ctx.config.modules.min_module_size)
    });

    if let Some(ref cohr) = ctx.cohesion_result {
        let mut coh_idx = 0;
        for (mid, mc) in &cohr.modules {
            if mc.cohesion_score < ctx.config.thresholds.cohesion_finding {
                coh_idx += 1;
                let severity = if mc.cohesion_score < ctx.config.thresholds.cohesion_critical {
                    Severity::Critical
                } else if mc.cohesion_score < ctx.config.thresholds.cohesion_high {
                    Severity::High
                } else {
                    Severity::Warning
                };
                ctx.findings.push(Finding {
                    id: format!("cohesion-{coh_idx}"),
                    finding_type: FindingType::LowCohesion,
                    severity,
                    description: format!(
                        "{mid}: cohesion {:.2} ({} connected components among {} functions)",
                        mc.cohesion_score, mc.connected_components, mc.function_count,
                    ),
                    detail: Some(
                        "Low cohesion (LCOM4) means the module's functions form disconnected groups that don't interact. \
                         This suggests unrelated responsibilities were combined — splitting improves clarity and maintainability."
                            .into(),
                    ),
                    node_ids: vec![],
                    edge_ids: None,
                    primary_node_id: None,
                    metric_name: Some("cohesion".into()),
                    metric_value: Some(mc.cohesion_score),
                    impact: None,
                    blast_radius: None,
                    recommendation: Some(
                        "This module contains unrelated groups of functions that don't interact. Consider splitting into separate, focused modules."
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
        eprintln!(
            "[ge-analyze] {} modules analyzed for cohesion, {} low-cohesion findings.",
            cohr.modules.len(),
            coh_idx,
        );
    }

    Ok(None)
}
