//! Analysis segment runner (S2-γ).

use anyhow::Result;

use super::super::super::fan_metrics;
use super::super::super::progress;
use super::super::super::report::*;
use super::super::super::run_safe;
use super::super::session::AnalysisRunContext;

pub fn run(ctx: &mut AnalysisRunContext<'_>) -> Result<Option<HealthReport>> {
    // --- 2. Fan-in / Fan-out ---
    progress::emit_progress("fan_metrics", 35, "running fan-in/fan-out computation");
    eprintln!("[ge-analyze] Running fan-in/fan-out computation...");
    let hs_pct = ctx.config.thresholds.hotspot_percentile;
    let hs_small = ctx.config.thresholds.hotspot_small_graph_threshold;
    let hs_fixed = ctx.config.thresholds.hotspot_small_graph_fixed;
    ctx.fan_result = run_safe("fan_metrics", &mut ctx.analysis_errors, || {
        fan_metrics::compute_fan_metrics_with_config(&ctx.graph, hs_pct, hs_small, hs_fixed, false)
    });

    if let Some(ref fr) = ctx.fan_result {
        for (id, fm) in &fr.metrics {
            if let Some(ann) = ctx.node_annotations.get_mut(id) {
                ann.fan_in = fm.fan_in;
                ann.fan_out = fm.fan_out;
                ann.is_hotspot = fm.is_hotspot;
            }
        }
    }

    Ok(None)
}
