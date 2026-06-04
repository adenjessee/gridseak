//! Analysis segment runner (S2-γ).

use anyhow::Result;

use super::super::super::coupling;
use super::super::super::progress;
use super::super::super::report::*;
use super::super::super::{build_coupling_finding, run_safe};
use super::super::session::AnalysisRunContext;

pub fn run(ctx: &mut AnalysisRunContext<'_>) -> Result<Option<HealthReport>> {
    // --- 3. Module coupling ---
    progress::emit_progress("module_coupling", 42, "running module coupling analysis");
    eprintln!("[ge-analyze] Running module coupling analysis...");
    let min_mod = ctx.config.modules.min_module_size;
    let hi_cpl = ctx.config.thresholds.high_coupling_threshold;
    let test_ratio = ctx.config.modules.test_module_ratio;
    ctx.coupling_result = run_safe("coupling", &mut ctx.analysis_errors, || {
        coupling::compute_coupling_with_config(&ctx.graph, min_mod, hi_cpl, test_ratio)
    });

    if let Some(ref cr) = ctx.coupling_result {
        for (mid, mc) in &cr.modules {
            if mc.coupling_score > ctx.config.thresholds.coupling_finding
                && !ctx.graph.is_non_production_node(mid)
            {
                ctx.findings
                    .push(build_coupling_finding(mid, mc, &ctx.config.thresholds));
            }
        }
    }

    Ok(None)
}
