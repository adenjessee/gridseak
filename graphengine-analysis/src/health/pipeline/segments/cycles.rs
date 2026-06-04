//! Analysis segment runner (S2-γ).

use std::collections::HashSet;

use anyhow::Result;

use super::super::super::build_cycle_finding;
use super::super::super::cycles;
use super::super::super::progress;
use super::super::super::report::*;
use super::super::session::AnalysisRunContext;

pub fn run(ctx: &mut AnalysisRunContext<'_>) -> Result<Option<HealthReport>> {
    // --- 1. Cycle detection ---
    progress::emit_progress("cycle_detection", 25, "running cycle detection");
    eprintln!("[ge-analyze] Running cycle detection...");
    ctx.cycle_result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cycles::detect_cycles(&ctx.graph)
    })) {
        Ok(r) => Some(r),
        Err(_) => {
            ctx.analysis_errors.push(AnalysisError {
                algorithm: "cycle_detection".into(),
                error: "Panicked during cycle detection".into(),
                nodes_affected: None,
            });
            None
        }
    };

    ctx.cycle_node_set = HashSet::new();
    ctx.total_cycle_nodes = 0usize;
    ctx.edges_in_cycles = 0usize;

    if let Some(ref cr) = ctx.cycle_result {
        ctx.total_cycle_nodes = cr.total_cycle_nodes;
        ctx.edges_in_cycles = cr.edges_in_cycles;

        for cycle in &cr.cycles {
            for nid in &cycle.node_ids {
                ctx.cycle_node_set.insert(nid.clone());
                if let Some(ann) = ctx.node_annotations.get_mut(nid) {
                    ann.cycle_member = true;
                    ann.cycle_ids.push(cycle.id.clone());
                }
            }
            let all_non_prod = cycle
                .node_ids
                .iter()
                .all(|nid| ctx.graph.is_non_production_node(nid));
            if !all_non_prod {
                ctx.findings.push(build_cycle_finding(
                    cycle,
                    &ctx.graph,
                    &ctx.config.thresholds,
                ));
            }
        }
    }

    Ok(None)
}
