//! Analysis segment runner (S2-γ).

use anyhow::Result;

use super::super::super::progress;
use super::super::super::report;
use super::super::super::report::*;
use super::super::super::{
    abstractness, api_surface, distance_from_main_sequence, hub_score, information_flow,
    instability, tangle, temporal_coupling,
};
use super::super::super::{percentile_threshold_usize, run_safe};
use super::super::session::AnalysisRunContext;

pub fn run(ctx: &mut AnalysisRunContext<'_>) -> Result<Option<HealthReport>> {
    // --- 7. Instability ---
    ctx.instability_result = run_safe("instability", &mut ctx.analysis_errors, || {
        instability::compute_instability(&ctx.graph)
    });

    // --- 7.5 Abstractness & Distance from Main Sequence ---
    ctx.abstractness_result = run_safe("abstractness", &mut ctx.analysis_errors, || {
        abstractness::compute_abstractness(&ctx.graph)
    });

    ctx.distance_result = match (&ctx.abstractness_result, &ctx.instability_result) {
        (Some(abst), Some(inst)) => Some(distance_from_main_sequence::compute_distance(abst, inst)),
        _ => None,
    };

    // --- 7.6 Zone of pain / zone of uselessness ctx.findings ---
    if let Some(ref dr) = ctx.distance_result {
        let mut zone_idx = 0;
        let mut sorted_modules: Vec<_> = dr.iter().collect();
        sorted_modules.sort_by(|a, b| a.0.cmp(b.0));
        for (module_key, md) in sorted_modules {
            if ctx.graph.is_non_production_node(module_key) {
                continue;
            }
            let (finding_type, zone_label) = match md.zone {
                distance_from_main_sequence::Zone::ZoneOfPain => {
                    (FindingType::ZoneOfPain, "Zone of Pain")
                }
                distance_from_main_sequence::Zone::ZoneOfUselessness => {
                    (FindingType::ZoneOfUselessness, "Zone of Uselessness")
                }
                _ => continue,
            };

            if md.distance < 0.3 {
                continue;
            }

            zone_idx += 1;
            let severity =
                if md.distance > 0.7 && md.zone == distance_from_main_sequence::Zone::ZoneOfPain {
                    Severity::High
                } else if md.distance > 0.5 {
                    Severity::Warning
                } else {
                    Severity::Info
                };

            let recommendation = match md.zone {
                distance_from_main_sequence::Zone::ZoneOfPain => {
                    "This module is highly concrete and heavily depended upon. Introduce abstractions (interfaces/traits) to reduce the cost of change."
                }
                distance_from_main_sequence::Zone::ZoneOfUselessness => {
                    "This module is highly abstract but rarely depended upon. It may contain unused abstractions — consolidate or remove dead interfaces."
                }
                _ => unreachable!(),
            };

            ctx.findings.push(Finding {
                id: format!("zone-{zone_idx}"),
                finding_type,
                severity,
                description: format!(
                    "{module_key}: {zone_label} (D={:.2}, A={:.2}, I={:.2})",
                    md.distance, md.abstractness, md.instability,
                ),
                detail: Some(format!(
                    "Distance from main sequence is {:.2}. Abstractness={:.2}, Instability={:.2}. \
                     The ideal is A + I = 1 (D = 0).",
                    md.distance, md.abstractness, md.instability,
                )),
                node_ids: vec![module_key.clone()],
                edge_ids: None,
                primary_node_id: Some(module_key.clone()),
                metric_name: Some("distance_from_main_sequence".into()),
                metric_value: Some(md.distance),
                impact: None,
                blast_radius: None,
                recommendation: Some(recommendation.into()),
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
        if zone_idx > 0 {
            eprintln!("[ge-analyze] {} zone finding(s) generated.", zone_idx);
        }
    }

    // --- 8. Tangle index ---
    ctx.tangle_idx = tangle::compute_tangle_index(ctx.edges_in_cycles, &ctx.graph);

    // --- 9. Information flow complexity ---
    ctx.ifc_result = ctx.fan_result.as_ref().and_then(|fr| {
        run_safe("information_flow", &mut ctx.analysis_errors, || {
            information_flow::compute_ifc(&ctx.graph, fr)
        })
    });

    if let Some(ref ifc) = ctx.ifc_result {
        for (id, &score) in &ifc.scores {
            if let Some(ann) = ctx.node_annotations.get_mut(id) {
                ann.information_flow_complexity = score;
            }
        }

        let ifc_threshold = percentile_threshold_usize(
            ifc.scores.values().copied().collect(),
            ctx.config.thresholds.ifc_percentile,
            ctx.config.thresholds.ifc_floor,
        );

        let mut ifc_idx = 0;
        let mut ifc_entries: Vec<_> = ifc
            .scores
            .iter()
            .filter(|(_, &s)| s > ifc_threshold)
            .collect();
        ifc_entries.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));

        let ifc_p99 = percentile_threshold_usize(
            ifc.scores.values().copied().collect(),
            ctx.config.thresholds.ifc_severity_critical_percentile,
            ctx.config.thresholds.ifc_floor_critical,
        );
        let ifc_p97 = percentile_threshold_usize(
            ifc.scores.values().copied().collect(),
            ctx.config.thresholds.ifc_severity_high_percentile,
            ctx.config.thresholds.ifc_floor_high,
        );

        for (id, &score) in &ifc_entries {
            if ctx.graph.is_non_production_node(id) {
                continue;
            }
            ifc_idx += 1;
            let severity = if score > ifc_p99 {
                Severity::Critical
            } else if score > ifc_p97 {
                Severity::High
            } else {
                Severity::Warning
            };
            let name = ctx
                .graph
                .nodes
                .get(id.as_str())
                .map(|n| n.display_name())
                .unwrap_or_else(|| "unknown".to_string());

            ctx.findings.push(Finding {
                id: format!("ifc-{ifc_idx}"),
                finding_type: FindingType::InformationFlowBottleneck,
                severity,
                description: format!(
                    "{name}: information flow complexity {score} (fan_in × fan_out interaction)"
                ),
                detail: Some(
                    "Information flow complexity (fan_in × fan_out) measures how much data flows through a function. \
                     High values indicate a bottleneck that both receives from many sources and distributes to many consumers."
                        .into(),
                ),
                node_ids: vec![id.to_string()],
                edge_ids: None,
                primary_node_id: Some(id.to_string()),
                metric_name: Some("information_flow_complexity".into()),
                metric_value: Some(score as f64),
                impact: None,
                blast_radius: None,
                recommendation: Some(
                    "This function both receives from and distributes to many consumers. Consider splitting its responsibilities."
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

    // --- 10. Hub score ---
    ctx.hub_result = run_safe("hub_score", &mut ctx.analysis_errors, || {
        hub_score::compute_hub_scores(&ctx.graph)
    });

    if let Some(ref hr) = ctx.hub_result {
        let bridge_counts: Vec<usize> = hr
            .values()
            .map(|hm| hm.source_modules + hm.target_modules)
            .collect();
        let hub_threshold = percentile_threshold_usize(
            bridge_counts.clone(),
            ctx.config.thresholds.hub_percentile,
            ctx.config.thresholds.hub_floor,
        );
        let hub_p99 = percentile_threshold_usize(
            bridge_counts.clone(),
            ctx.config.thresholds.hub_severity_high_percentile,
            ctx.config.thresholds.hub_floor_high,
        );
        let _hub_p97 = percentile_threshold_usize(
            bridge_counts,
            ctx.config.thresholds.hub_severity_warning_percentile,
            ctx.config.thresholds.hub_floor_warning,
        );

        let mut hub_idx = 0;
        let mut hub_entries: Vec<_> = hr
            .iter()
            .map(|(id, hm)| (id, hm, hm.source_modules + hm.target_modules))
            .filter(|(_, _, b)| *b > hub_threshold)
            .collect();
        hub_entries.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(b.0)));

        for (id, hm, bridges) in &hub_entries {
            if let Some(ann) = ctx.node_annotations.get_mut(id.as_str()) {
                ann.hub_score = hm.hub_score;
            }

            if ctx.graph.is_non_production_node(id) {
                continue;
            }
            hub_idx += 1;
            let severity = if *bridges > hub_p99 {
                Severity::High
            } else {
                Severity::Warning
            };
            let name = ctx
                .graph
                .nodes
                .get(id.as_str())
                .map(|n| n.display_name())
                .unwrap_or_else(|| "unknown".to_string());

            ctx.findings.push(Finding {
                id: format!("hub-{hub_idx}"),
                finding_type: FindingType::HubNode,
                severity,
                description: format!(
                    "{name} bridges {bridges} distinct modules",
                ),
                detail: Some(
                    "Hub nodes are functions that connect many different modules in the call ctx.graph. \
                     They create tight coupling across the codebase and become single points of failure for changes."
                        .into(),
                ),
                node_ids: vec![id.to_string()],
                edge_ids: None,
                primary_node_id: Some(id.to_string()),
                metric_name: Some("hub_bridges".into()),
                metric_value: Some(*bridges as f64),
                impact: None,
                blast_radius: None,
                recommendation: Some(
                    "This function is a critical cross-cutting dependency. Consider dependency injection to reduce direct coupling."
                        .into(),
                ),
                cycle_length: None,
                fan_in: None,
                coupling_score: None,
                internal_edges: None,
                external_edges: None,
                count: None,
                hub_score: Some(hm.hub_score),
                file_a: None,
                file_b: None,
                co_change_count: None,
                temporal_coupling_score: None,
                has_import_edge: None,
                confidence: None,
            });
        }

        // Set hub_score annotation for all hub results (not just ctx.findings)
        for (id, hm) in hr {
            if let Some(ann) = ctx.node_annotations.get_mut(id) {
                ann.hub_score = hm.hub_score;
            }
        }
    }

    // --- 11. API surface ---
    ctx.api_result = run_safe("api_surface", &mut ctx.analysis_errors, || {
        api_surface::compute_api_surface(&ctx.graph)
    });

    if let Some(ref ar) = ctx.api_result {
        let mut api_idx = 0;
        for (mid, am) in ar {
            if ctx.graph.is_non_production_node(mid) {
                continue;
            }
            if am.api_surface_ratio > ctx.config.thresholds.api_surface_warning {
                api_idx += 1;
                let severity = if am.api_surface_ratio > ctx.config.thresholds.api_surface_critical
                {
                    Severity::Critical
                } else if am.api_surface_ratio > ctx.config.thresholds.api_surface_high {
                    Severity::High
                } else {
                    Severity::Warning
                };
                let name = ctx
                    .graph
                    .nodes
                    .get(mid.as_str())
                    .map(|n| n.display_name())
                    .unwrap_or_else(|| mid.to_string());

                ctx.findings.push(Finding {
                    id: format!("encap-{api_idx}"),
                    finding_type: FindingType::LowEncapsulation,
                    severity,
                    description: format!(
                        "{name}: {}/{} functions ({:.0}%) are called externally",
                        am.exported_functions,
                        am.total_functions,
                        am.api_surface_ratio * 100.0,
                    ),
                    detail: Some(
                        "Low encapsulation means a module exposes too much of its internal implementation as a public API. \
                         This increases coupling and makes refactoring risky, since external callers depend on implementation details."
                            .into(),
                    ),
                    node_ids: vec![mid.clone()],
                    edge_ids: None,
                    primary_node_id: Some(mid.clone()),
                    metric_name: Some("api_surface_ratio".into()),
                    metric_value: Some(am.api_surface_ratio),
                    impact: None,
                    blast_radius: None,
                    recommendation: Some(
                        "This module exposes too much of its implementation. Consider making internal functions private and routing through a smaller public API."
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
    }

    // --- 14. Temporal coupling (optional, requires --git-dir) ---
    ctx.temporal_result = if let Some(gd) = ctx.git_dir {
        progress::emit_progress(
            "temporal_coupling",
            88,
            "running temporal coupling analysis",
        );
        eprintln!("[ge-analyze] Running temporal coupling analysis...");
        let git_path = std::path::Path::new(gd);
        match temporal_coupling::analyze_temporal_coupling(
            git_path,
            &ctx.graph,
            &ctx.config.thresholds,
        ) {
            Ok(result) => {
                ctx.findings.extend(result.findings.clone());
                Some(result)
            }
            Err(e) => {
                eprintln!("[ge-analyze] Warning: temporal coupling analysis failed: {e}");
                ctx.analysis_errors.push(AnalysisError {
                    algorithm: "temporal_coupling".into(),
                    error: e,
                    nodes_affected: None,
                });
                None
            }
        }
    } else {
        None
    };

    // --- 14b. Module-level temporal coupling ctx.findings ---
    if let Some(ref tr) = ctx.temporal_result {
        let mut mod_tc_idx = 0;
        for mp in &tr.module_pairs {
            if mp.has_import_coupling {
                continue;
            }
            mod_tc_idx += 1;
            if mod_tc_idx > ctx.config.max_findings_per_type {
                break;
            }
            let severity = if mp.coupling_score > ctx.config.thresholds.temporal_hidden_high_score {
                report::Severity::High
            } else {
                report::Severity::Warning
            };
            ctx.findings.push(report::Finding {
                id: format!("module-temporal-{}", mod_tc_idx),
                finding_type: report::FindingType::TemporalCoupling,
                severity,
                description: format!(
                    "Modules {} and {} have high temporal coupling ({:.0}%, {} co-changes) but no import relationship",
                    mp.module_a, mp.module_b,
                    mp.coupling_score * 100.0,
                    mp.co_change_count,
                ),
                detail: Some(
                    "Module-level temporal coupling aggregates file-level co-change data. \
                     Modules that change together without import edges indicate hidden architectural coupling."
                        .into(),
                ),
                node_ids: vec![],
                edge_ids: None,
                primary_node_id: None,
                metric_name: Some("module_temporal_coupling_score".into()),
                metric_value: Some(mp.coupling_score),
                impact: None,
                blast_radius: None,
                recommendation: Some(
                    "Investigate shared state, configuration, or implicit contracts between these modules. \
                     Consider making the dependency explicit or extracting a shared module."
                        .into(),
                ),
                cycle_length: None,
                fan_in: None,
                coupling_score: None,
                internal_edges: None,
                external_edges: None,
                count: None,
                hub_score: None,
                file_a: Some(mp.module_a.clone()),
                file_b: Some(mp.module_b.clone()),
                co_change_count: Some(mp.co_change_count),
                temporal_coupling_score: Some(mp.coupling_score),
                has_import_edge: Some(mp.has_import_coupling),
                confidence: None,
            });
        }
        if mod_tc_idx > 0 {
            eprintln!(
                "[ge-analyze] {} module-level hidden temporal coupling finding(s) generated.",
                mod_tc_idx
            );
        }
    }

    Ok(None)
}
