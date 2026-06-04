//! Analysis segment runner (S2-γ).

use anyhow::Result;

use super::super::super::report::*;
use super::super::super::{
    assign_module_risk_levels, assign_node_risk_levels, cap_findings,
    tag_confidence_from_resolution,
};
use super::super::super::{graph, resolution_degraded};
use super::super::session::AnalysisRunContext;

pub fn run(ctx: &mut AnalysisRunContext<'_>) -> Result<Option<HealthReport>> {
    // Set complexity annotations for all measured functions (not just ctx.findings)
    for id in &ctx.graph.function_node_ids {
        if let Some(node) = ctx.graph.nodes.get(id) {
            if let Some(ann) = ctx.node_annotations.get_mut(id) {
                if ann.cyclomatic_complexity.is_none() {
                    ann.cyclomatic_complexity = node.cyclomatic_complexity;
                }
                if ann.cognitive_complexity.is_none() {
                    ann.cognitive_complexity = node.cognitive_complexity;
                }
            }
        }
    }

    // --- Build module annotations (path-prefix-based analysis modules) ---
    for module_key in &ctx.graph.folder_module_ids {
        let coupling_data = ctx
            .coupling_result
            .as_ref()
            .and_then(|cr| cr.modules.get(module_key));
        let instability_data = ctx
            .instability_result
            .as_ref()
            .and_then(|ir| ir.get(module_key));
        let abstractness_data = ctx
            .abstractness_result
            .as_ref()
            .and_then(|ar| ar.get(module_key));
        let distance_data = ctx
            .distance_result
            .as_ref()
            .and_then(|dr| dr.get(module_key));
        let api_data = ctx.api_result.as_ref().and_then(|ar| ar.get(module_key));
        let cohesion_data = ctx
            .cohesion_result
            .as_ref()
            .and_then(|cr| cr.modules.get(module_key));
        let layer_data = ctx
            .layer_result
            .as_ref()
            .and_then(|lr| lr.module_stats.get(module_key));

        let members = ctx
            .graph
            .analysis_module_members_of(module_key)
            .cloned()
            .unwrap_or_default();
        let total_nodes = members.len();
        let total_fns = members
            .iter()
            .filter(|d| {
                ctx.graph
                    .nodes
                    .get(d.as_str())
                    .map(|n| n.kind.is_function_like() && !graph::is_synthetic_node(n))
                    .unwrap_or(false)
            })
            .count();

        let module_loc: usize = members
            .iter()
            .filter_map(|id| ctx.fn_loc.get(id.as_str()))
            .sum();

        let module_is_production = !ctx.graph.is_non_production_node(module_key);

        ctx.module_annotations.insert(
            module_key.clone(),
            ModuleAnnotation {
                is_production: module_is_production,
                coupling_score: coupling_data.map(|c| c.coupling_score).unwrap_or(0.0),
                internal_edges: coupling_data.map(|c| c.internal_edges).unwrap_or(0),
                external_edges: coupling_data.map(|c| c.external_edges).unwrap_or(0),
                instability: instability_data.map(|i| i.instability).unwrap_or(0.0),
                afferent_coupling: instability_data.map(|i| i.afferent_coupling).unwrap_or(0),
                efferent_coupling: instability_data.map(|i| i.efferent_coupling).unwrap_or(0),
                abstractness: abstractness_data.map(|a| a.abstractness).unwrap_or(0.0),
                abstract_types: abstractness_data.map(|a| a.abstract_types).unwrap_or(0),
                total_types: abstractness_data.map(|a| a.total_types).unwrap_or(0),
                distance_from_main_sequence: distance_data.map(|d| d.distance).unwrap_or(0.0),
                zone: distance_data.map(|d| d.zone.to_string()),
                cohesion_score: cohesion_data.map(|c| c.cohesion_score).unwrap_or(0.0),
                connected_components: cohesion_data.map(|c| c.connected_components).unwrap_or(0),
                api_surface_ratio: api_data.map(|a| a.api_surface_ratio).unwrap_or(0.0),
                exported_functions: api_data.map(|a| a.exported_functions).unwrap_or(0),
                total_functions: total_fns,
                total_nodes,
                total_loc: module_loc,
                avg_layer_depth: layer_data.map(|l| l.avg_layer_depth).unwrap_or(0.0),
                layer_violation_count: layer_data.map(|l| l.violation_count).unwrap_or(0),
                risk_level: RiskLevel::Healthy,
            },
        );
    }

    // --- Sprint D.2: ResolutionDegraded finding ---
    //
    // Read the parser's resolution telemetry counters from ctx.graph
    // metadata and, if the heuristic fallback rate exceeds the
    // threshold, emit a single finding explaining the degraded
    // analysis. See `health::resolution_degraded` for the threshold
    // rationale and behavior.
    {
        let parse_u = |key: &str| -> usize {
            graph::read_metadata(&ctx.conn, key)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0)
        };
        let stats_snapshot = resolution_degraded::ResolutionStatsSnapshot {
            lsp_edges: parse_u("resolution_lsp_edges"),
            heuristic_edges: parse_u("resolution_heuristic_edges"),
            heuristic_call_fallbacks: parse_u("resolution_heuristic_call_fallbacks"),
            heuristic_import_fallbacks: parse_u("resolution_heuristic_import_fallbacks"),
            heuristic_type_fallbacks: parse_u("resolution_heuristic_type_fallbacks"),
            heuristic_call_ambiguous_drops: parse_u("resolution_heuristic_call_ambiguous_drops"),
        };
        if let Some(f) = resolution_degraded::evaluate(
            &stats_snapshot,
            resolution_degraded::Thresholds::default(),
        ) {
            eprintln!(
                "[ge-analyze] ResolutionDegraded: {:.1}% fallback rate (severity {:?})",
                stats_snapshot.fallback_rate().unwrap_or(0.0) * 100.0,
                f.severity,
            );
            ctx.findings.push(f);
        }
    }

    // --- Tag finding confidence based on resolution quality ---
    tag_confidence_from_resolution(&mut ctx.findings, &ctx.resolution_quality);

    // --- Assign risk levels (before capping, so all ctx.findings influence risk) ---
    assign_node_risk_levels(&ctx.findings, &mut ctx.node_annotations);
    assign_module_risk_levels(&ctx.findings, &mut ctx.module_annotations);

    // --- Sort ctx.findings: severity desc, then id asc ---
    ctx.findings.sort_by(|a, b| {
        b.severity
            .rank()
            .cmp(&a.severity.rank())
            .then_with(|| a.id.cmp(&b.id))
    });

    // --- Apply finding triage suppression (from previous run ctx.overrides) ---
    if let Some(ov) = ctx.overrides {
        let suppressed = crate::validation::overrides::suppressed_finding_ids(ov);
        if !suppressed.is_empty() {
            let before = ctx.findings.len();
            ctx.findings.retain(|f| !suppressed.contains(&f.id));
            let after = ctx.findings.len();
            if before != after {
                eprintln!(
                    "[ge-analyze] Suppressed {} finding(s) via triage overrides.",
                    before - after
                );
            }
        }
    }

    // --- Cap findings per type: keep top N, summarize the rest ---
    ctx.findings = cap_findings(
        std::mem::take(&mut ctx.findings),
        ctx.config.max_findings_per_type,
    );

    Ok(None)
}
