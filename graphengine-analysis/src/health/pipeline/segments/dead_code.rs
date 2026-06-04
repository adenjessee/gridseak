//! Analysis segment runner (S2-γ).

use anyhow::Result;

use super::super::super::progress;
use super::super::super::report::*;
use super::super::super::run_safe;
use super::super::super::{dead_code, dead_code_classifier, entry_points, repo_classification};
use super::super::session::AnalysisRunContext;

pub fn run(ctx: &mut AnalysisRunContext<'_>) -> Result<Option<HealthReport>> {
    // --- 4. Dead code ---
    // Detect library vs application so exported-symbol entry-point rule can be tuned.
    ctx.repo_type = {
        // Check for user override first
        let override_type = ctx
            .overrides
            .as_ref()
            .and_then(|ov| ov.repo_type_override.as_deref())
            .and_then(|rt| match rt {
                "library" => Some(repo_classification::RepoType::Library),
                "application" => Some(repo_classification::RepoType::Application),
                _ => None,
            });

        if let Some(rt) = override_type {
            eprintln!("[ge-analyze] Repo type: {:?} (user override)", rt);
            rt
        } else {
            let file_paths: Vec<&str> = ctx
                .graph
                .nodes
                .values()
                .filter_map(|n| n.file_path.as_deref())
                .collect();
            let ws_root = repo_classification::infer_workspace_root(&file_paths);
            let rtype = ws_root
                .as_deref()
                .map(repo_classification::classify_repo)
                .unwrap_or(repo_classification::RepoType::Application);
            eprintln!("[ge-analyze] Repo type: {:?}", rtype);
            rtype
        }
    };

    progress::emit_progress("dead_code", 50, "running dead code detection");
    eprintln!("[ge-analyze] Running dead code detection...");
    ctx.dc_cfg = ctx.config.dead_code.clone();
    if ctx.repo_type == repo_classification::RepoType::Library {
        ctx.dc_cfg.exported_symbols = true;
    }
    ctx.dead_result = run_safe("dead_code", &mut ctx.analysis_errors, || {
        dead_code::detect_dead_code(&ctx.graph, &ctx.dc_cfg)
    });

    // Classify every dead node with a reason + evidence via the
    // dead-code classifier registry. The verdict distribution is
    // stamped on `MetricsReport.dead_code.reason_breakdown` and per-
    // function reason + evidence land on `NodeAnnotation`. See
    // `dead_code_classifier/mod.rs` for the trait and fallthrough
    // contract.
    //
    // Scope contract (enforced by `DeadCodeResult` type):
    // - `reason_breakdown` counts ONLY `ctx.dead_result.production`.
    // - Per-node annotations cover `ctx.dead_result.all()` so UI
    //   drilldowns remain complete for test/vendor dead code.
    // This used to be a prose comment with N filter sites kept in
    // sync by hand (R20). Typing the result eliminates the class of
    // bug outright.
    ctx.dead_code_reason_breakdown = dead_code_classifier::empty_reason_breakdown();
    ctx.dead_code_verdicts_all = Vec::new();
    if let Some(ref dr) = ctx.dead_result {
        let classifier = dead_code_classifier::ClassifierRegistry::default();
        let classifiable_all: Vec<String> = dr
            .all()
            .filter(|id| !ctx.override_entry_point_ids.contains(id.as_str()))
            .cloned()
            .collect();
        ctx.dead_code_verdicts_all = classifier.classify_batch(
            &classifiable_all,
            &ctx.graph,
            ctx.config.resolved_ecosystem(),
        );
        // Production-only subset for `reason_breakdown`. Using a
        // BTreeSet for O(log n) membership instead of re-calling
        // `is_non_production_node` per verdict.
        let prod_ids: std::collections::BTreeSet<&str> =
            dr.production.iter().map(String::as_str).collect();
        let dead_code_verdicts_prod: Vec<_> = ctx
            .dead_code_verdicts_all
            .iter()
            .filter(|v| prod_ids.contains(v.node_id.as_str()))
            .cloned()
            .collect();
        ctx.dead_code_reason_breakdown =
            dead_code_classifier::build_reason_breakdown(&dead_code_verdicts_prod);
    }

    if let Some(ref dr) = ctx.dead_result {
        for id in dr.all() {
            if ctx.override_entry_point_ids.contains(id) {
                continue;
            }
            if let Some(ann) = ctx.node_annotations.get_mut(id) {
                ann.is_dead = true;
            }
        }
        // Stamp every classified verdict on per-node annotations,
        // production and non-production, so the UI drilldown stays
        // complete. Only the aggregate `reason_breakdown` above is
        // constrained to production-only.
        for v in &ctx.dead_code_verdicts_all {
            if let Some(ann) = ctx.node_annotations.get_mut(&v.node_id) {
                ann.dead_code_reason = Some(v.reason);
                ann.dead_code_evidence = Some(v.evidence.clone());
                ann.dead_code_classifier = Some(v.classifier.to_string());
                ann.dead_code_confidence = Some(v.confidence);
            }
        }
        let prod_dead: Vec<String> = dr
            .production
            .iter()
            .filter(|id| !ctx.override_entry_point_ids.contains(id.as_str()))
            .cloned()
            .collect();
        if !prod_dead.is_empty() {
            ctx.findings.push(Finding {
                id: "dead-1".into(),
                finding_type: FindingType::PotentiallyUnreachable,
                severity: Severity::Info,
                description: format!(
                    "{} potentially unreachable functions",
                    prod_dead.len()
                ),
                detail: Some(
                    "Dead code refers to functions that have no incoming call edges in the dependency ctx.graph. \
                     They may be unused, called via reflection, or invoked by external frameworks — verify before removing."
                        .into(),
                ),
                node_ids: prod_dead.clone(),
                edge_ids: None,
                primary_node_id: None,
                metric_name: Some("dead_function_count".into()),
                metric_value: Some(prod_dead.len() as f64),
                impact: None,
                blast_radius: None,
                recommendation: Some(
                    "Review these functions for removal or confirm they are called via reflection/framework mechanisms."
                        .into(),
                ),
                cycle_length: None,
                fan_in: None,
                coupling_score: None,
                internal_edges: None,
                external_edges: None,
                count: Some(prod_dead.len()),
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

    // --- 4.5 Entry point reporting ---
    progress::emit_progress("entry_points", 58, "collecting entry points");
    eprintln!("[ge-analyze] Collecting entry points...");
    {
        let mut entry_node_ids: Vec<String> = Vec::new();
        for id in &ctx.graph.function_node_ids {
            if entry_points::is_entry_point(&ctx.graph, id, &ctx.dc_cfg)
                && !ctx.graph.is_non_production_node(id)
            {
                entry_node_ids.push(id.clone());
            }
        }
        if !entry_node_ids.is_empty() {
            eprintln!(
                "[ge-analyze] {} production entry points detected.",
                entry_node_ids.len()
            );
            ctx.findings.push(Finding {
                id: "entry-points-1".into(),
                finding_type: FindingType::EntryPoint,
                severity: Severity::Info,
                description: format!("{} entry points identified", entry_node_ids.len()),
                detail: Some(
                    "Entry points are functions that can be invoked from outside the codebase (e.g., main, exported APIs, framework hooks). \
                     They define the public surface and are natural starting points for execution flow."
                        .into(),
                ),
                node_ids: entry_node_ids.clone(),
                edge_ids: None,
                primary_node_id: None,
                metric_name: Some("entry_point_count".into()),
                metric_value: Some(entry_node_ids.len() as f64),
                impact: None,
                blast_radius: None,
                recommendation: None,
                cycle_length: None,
                fan_in: None,
                coupling_score: None,
                internal_edges: None,
                external_edges: None,
                count: Some(entry_node_ids.len()),
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

    Ok(None)
}
