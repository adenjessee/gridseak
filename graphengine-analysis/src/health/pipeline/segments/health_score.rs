//! Analysis segment runner (S2-γ).

use std::collections::BTreeMap;

use anyhow::Result;

use super::super::super::progress;
use super::super::super::report;
use super::super::super::report::*;
use super::super::super::{
    build_classifications, build_integrity_status, persist_project_language,
};
use super::super::super::{
    cohesion, coupling, cycles, dead_code, dead_code_confidence, depth,
    distance_from_main_sequence, fan_metrics, graph, health_score, instability, metric_status,
    norms,
};
use super::super::session::AnalysisRunContext;

pub fn run(ctx: &mut AnalysisRunContext<'_>) -> Result<Option<HealthReport>> {
    // --- Build score inputs ---
    progress::emit_progress("health_score", 98, "computing health score");
    eprintln!("[ge-analyze] Computing health score...");
    let fan_ref = ctx.fan_result.as_ref();
    // Count production dead functions only (consistent with the
    // finding). The typed `DeadCodeResult.production` slice is the
    // single source of truth for this number — no secondary filter
    // needed.
    let prod_dead_count = ctx
        .dead_result
        .as_ref()
        .map(|d| d.production.len())
        .unwrap_or(0);

    let avg_cyclomatic = ctx
        .complexity_result
        .as_ref()
        .map(|cr| cr.avg_cyclomatic)
        .unwrap_or(0.0);

    let avg_cohesion = ctx
        .cohesion_result
        .as_ref()
        .map(|cr| {
            let scores: Vec<f64> = cr.modules.values().map(|m| m.cohesion_score).collect();
            if scores.is_empty() {
                1.0
            } else {
                scores.iter().sum::<f64>() / scores.len() as f64
            }
        })
        .unwrap_or(1.0);

    let avg_distance = ctx
        .distance_result
        .as_ref()
        .map(|dr| {
            let distances: Vec<f64> = dr.values().map(|d| d.distance).collect();
            if distances.is_empty() {
                0.0
            } else {
                distances.iter().sum::<f64>() / distances.len() as f64
            }
        })
        .unwrap_or(0.0);

    let (hidden_coupling_pairs, total_file_pairs_analyzed) = ctx
        .temporal_result
        .as_ref()
        .map(|tr| (tr.hidden_coupling_pairs, tr.pairs.len()))
        .unwrap_or((0, 0));

    // Use redistributed weights when temporal coupling data is unavailable
    let effective_weights = if ctx.git_dir.is_some() {
        ctx.config.score_weights.clone()
    } else {
        ctx.config.score_weights.without_temporal()
    };

    let score_inputs = health_score::ScoreInputs {
        total_nodes: ctx.graph.total_nodes(),
        total_cycle_nodes: ctx.total_cycle_nodes,
        avg_coupling_score: ctx
            .coupling_result
            .as_ref()
            .map(|c| c.avg_coupling_excluding_tests)
            .unwrap_or(0.0),
        coupling_baseline: ctx.config.thresholds.coupling_baseline,
        sum_hotspot_fan_in: fan_ref.map(|f| f.sum_hotspot_fan_in).unwrap_or(0),
        total_fan_in: fan_ref.map(|f| f.total_fan_in).unwrap_or(0),
        dead_functions: prod_dead_count,
        total_functions: ctx.graph.total_functions(),
        max_call_depth: ctx
            .depth_result
            .as_ref()
            .map(|d| d.max_call_depth)
            .unwrap_or(0),
        avg_cyclomatic,
        avg_cohesion,
        avg_distance,
        hidden_coupling_pairs,
        total_file_pairs_analyzed,
    };

    let metric_values = health_score::extract_metric_values(&score_inputs, ctx.tangle_idx);

    // --- Build Layer A: MetricsReport (always present) ---
    let dead_fn_count = prod_dead_count;
    let total_fns = ctx.graph.total_functions();
    let max_depth = ctx
        .depth_result
        .as_ref()
        .map(|d| d.max_call_depth)
        .unwrap_or(0);
    let total_structural_edges_for_tangle = ctx.graph.structural_edge_indices.len();

    // ------------------------------------------------------------------
    // T3 dual-metric emission: re-compute Layer-3 metrics over just the
    // `Confidence::High` subset of production edges so the ctx.report can
    // surface a fidelity gap alongside each headline number.
    //
    // Must run BEFORE the `is_production_module` closure below
    // immutably captures `ctx.graph`, because it needs a mutable borrow to
    // swap the production-edge index vectors.
    //
    // Scope note: we cover the metrics that read from
    // `production_structural_edge_indices` (cycles, depth, tangle) and
    // dead_code (via the high-only fan-in variant). Metrics that
    // compute over earlier-stage edge sets (`structural_edge_indices`
    // for coupling/instability, `clean_structural_edge_indices` for
    // layers) or over the full adjacency (hotspot_concentration,
    // cohesion, distance) emit `fidelity: None` here — they require
    // their own confidence-filtered edge subsets and are tracked as T3
    // follow-up (see `FidelityGap` docs).
    // ------------------------------------------------------------------
    let all_edges_count_for_fidelity = ctx.graph.production_structural_edge_indices.len();
    let high_only_edges_count_for_fidelity =
        ctx.graph.high_only_production_structural_edge_indices.len();

    // Dead-code high-only recount: does not depend on swapping edge
    // indices — `fan_in_high_only` filters edges at query time.
    let dead_high_only_result =
        dead_code::detect_dead_code_high_only(&ctx.graph, &ctx.config.dead_code);
    let dead_high_only_prod_count = dead_high_only_result
        .all()
        .filter(|id| !ctx.graph.is_non_production_node(id.as_str()))
        .count();
    let dead_ratio_high_only = if total_fns > 0 {
        dead_high_only_prod_count as f64 / total_fns as f64
    } else {
        0.0
    };
    let dead_code_fidelity = Some(FidelityGap::from_values(
        metric_values.dead_ratio,
        dead_ratio_high_only,
        all_edges_count_for_fidelity,
        high_only_edges_count_for_fidelity,
    ));

    // Swap production edges to the high-only subset, re-run cycle/depth
    // analysis, swap back. RAII-guarded so a panic inside either re-run
    // cannot leave the ctx.graph in a swapped state (subsequent post-metric
    // code reads `production_structural_edge_indices` and must see the
    // full set).
    let (high_only_cycle_nodes, high_only_edges_in_cycles, high_only_max_depth) = {
        struct SwapGuard<'a> {
            graph: &'a mut crate::health::graph::AnalysisGraph,
        }
        impl Drop for SwapGuard<'_> {
            fn drop(&mut self) {
                std::mem::swap(
                    &mut self.graph.production_structural_edge_indices,
                    &mut self.graph.high_only_production_structural_edge_indices,
                );
            }
        }
        std::mem::swap(
            &mut ctx.graph.production_structural_edge_indices,
            &mut ctx.graph.high_only_production_structural_edge_indices,
        );
        let guard = SwapGuard {
            graph: &mut ctx.graph,
        };
        let cycle_high = cycles::detect_cycles(guard.graph);
        let cycle_nodes_high: std::collections::HashSet<String> = cycle_high
            .cycles
            .iter()
            .flat_map(|c| c.node_ids.iter().cloned())
            .collect();
        let depth_high = depth::compute_depth(guard.graph, &cycle_nodes_high);
        (
            cycle_high.total_cycle_nodes,
            cycle_high.edges_in_cycles,
            depth_high.max_call_depth,
        )
    };

    let cycle_ratio_high_only = if ctx.graph.total_nodes() > 0 {
        high_only_cycle_nodes as f64 / ctx.graph.total_nodes() as f64
    } else {
        0.0
    };
    let tangle_high_only = if total_structural_edges_for_tangle > 0 {
        high_only_edges_in_cycles as f64 / total_structural_edges_for_tangle as f64
    } else {
        0.0
    };

    let cycles_fidelity = Some(FidelityGap::from_values(
        metric_values.cycle_ratio,
        cycle_ratio_high_only,
        all_edges_count_for_fidelity,
        high_only_edges_count_for_fidelity,
    ));
    let tangle_fidelity = Some(FidelityGap::from_values(
        ctx.tangle_idx,
        tangle_high_only,
        all_edges_count_for_fidelity,
        high_only_edges_count_for_fidelity,
    ));
    let depth_fidelity = Some(FidelityGap::from_values(
        max_depth as f64,
        high_only_max_depth as f64,
        all_edges_count_for_fidelity,
        high_only_edges_count_for_fidelity,
    ));

    // Hotspot concentration high-only view. Fan-metrics high-only
    // recomputes fan-in via `fan_in_high_only`, so hotspot detection
    // and the resulting "what share of fan-in is absorbed by the top
    // 5%" ratio both change. Uses the same percentile / small-ctx.graph
    // config as the main fan_metrics pass so the two numbers are
    // comparable.
    let hs_pct_cfg = ctx.config.thresholds.hotspot_percentile;
    let hs_small_cfg = ctx.config.thresholds.hotspot_small_graph_threshold;
    let hs_fixed_cfg = ctx.config.thresholds.hotspot_small_graph_fixed;
    let fan_high_only = fan_metrics::compute_fan_metrics_with_config(
        &ctx.graph,
        hs_pct_cfg,
        hs_small_cfg,
        hs_fixed_cfg,
        true,
    );
    let concentration_high_only = if fan_high_only.total_fan_in > 0 {
        fan_high_only.sum_hotspot_fan_in as f64 / fan_high_only.total_fan_in as f64
    } else {
        0.0
    };

    // Coupling high-only: re-runs the coupling computation filtering
    // out non-`High` edges at the per-module internal/external split
    // step. Headline value compared to ctx.report is `avg_coupling_
    // excluding_tests`.
    let coupling_high_only = coupling::compute_coupling_high_only(&ctx.graph);

    // Cohesion high-only: the module-relationship ctx.graph is built from
    // only `Confidence::High` edges, so modules whose apparent
    // cohesion was driven by heuristic bridges fragment into more
    // components. Conditional on ctx.cohesion_result being present — the
    // original pass is itself opt-in via `ctx.config.cohesion_enabled`.
    let cohesion_high_only_avg = ctx.cohesion_result.as_ref().map(|_| {
        let cr =
            cohesion::compute_cohesion_high_only(&ctx.graph, ctx.config.modules.min_module_size);
        let scores: Vec<f64> = cr.modules.values().map(|m| m.cohesion_score).collect();
        if scores.is_empty() {
            1.0
        } else {
            scores.iter().sum::<f64>() / scores.len() as f64
        }
    });

    // Distance-from-main-sequence high-only: recompute instability
    // with confidence filtering, then feed through `compute_distance`
    // alongside the original abstractness (abstractness is node-based,
    // not edge-based, so it doesn't have a high-only variant).
    let distance_high_only_avg = match (&ctx.abstractness_result, &ctx.instability_result) {
        (Some(abst), Some(_)) => {
            let inst_high = instability::compute_instability_high_only(&ctx.graph);
            let dr = distance_from_main_sequence::compute_distance(abst, &inst_high);
            let ds: Vec<f64> = dr.values().map(|d| d.distance).collect();
            Some(if ds.is_empty() {
                0.0
            } else {
                ds.iter().sum::<f64>() / ds.len() as f64
            })
        }
        _ => None,
    };

    let is_production_module = |key: &str| !ctx.graph.is_non_production_node(key);
    let coupling_modules_measured = ctx
        .coupling_result
        .as_ref()
        .map(|c| {
            c.modules
                .keys()
                .filter(|k| is_production_module(k.as_str()))
                .count()
        })
        .unwrap_or(0);
    let coupling_above_070 = ctx
        .coupling_result
        .as_ref()
        .map(|c| {
            c.modules
                .iter()
                .filter(|(k, m)| is_production_module(k.as_str()) && m.coupling_score > 0.70)
                .count()
        })
        .unwrap_or(0);
    let coupling_above_050 = ctx
        .coupling_result
        .as_ref()
        .map(|c| {
            c.modules
                .iter()
                .filter(|(k, m)| is_production_module(k.as_str()) && m.coupling_score > 0.50)
                .count()
        })
        .unwrap_or(0);
    let avg_coupling_val = ctx
        .coupling_result
        .as_ref()
        .map(|c| c.avg_coupling_excluding_tests)
        .unwrap_or(0.0);

    let hotspot_fns = fan_ref.map(|f| f.hotspot_count).unwrap_or(0);
    let hotspot_fan_in_sum = fan_ref.map(|f| f.sum_hotspot_fan_in).unwrap_or(0);
    let total_fan_in_val = fan_ref.map(|f| f.total_fan_in).unwrap_or(0);
    let concentration_ratio = if total_fan_in_val > 0 {
        hotspot_fan_in_sum as f64 / total_fan_in_val as f64
    } else {
        0.0
    };

    let total_structural_edges = ctx.graph.structural_edge_indices.len();

    // --- Per-metric confidence based on ctx.graph quality signals ---
    let cross_file_call_edges = ctx.graph.production_structural_edge_indices.len();
    let metric_confidence = {
        let mut conf = BTreeMap::new();
        let depth_conf = if cross_file_call_edges < 5 {
            MetricConfidence {
                level: Confidence::Low,
                reason: format!("{cross_file_call_edges} production cross-file call edges — depth may be underreported"),
            }
        } else if cross_file_call_edges < 20 {
            MetricConfidence {
                level: Confidence::Medium,
                reason: format!("{cross_file_call_edges} production cross-file call edges"),
            }
        } else {
            MetricConfidence {
                level: Confidence::High,
                reason: format!("{cross_file_call_edges} production cross-file call edges"),
            }
        };
        conf.insert("depth".into(), depth_conf);

        // Dead-code metric confidence is the *minimum* of two axes:
        // cross-file resolution tier (LSP vs heuristic vs nothing) and
        // framework-attribute extraction coverage for the dominant
        // language. Apex earns High because the classifier knows its
        // dispatch model; Rust/Python/Java/etc. cap at Medium until
        // their attribute-macro / decorator / annotation patterns are
        // traversed end-to-end. See `dead_code_confidence` for the
        // per-ecosystem table and rationale.
        let dead_code_conf = dead_code_confidence::compute(
            ctx.config.resolved_ecosystem(),
            ctx.resolution_quality.resolution_tier,
        );
        conf.insert("dead_code".into(), dead_code_conf);

        let coupling_conf = if coupling_modules_measured < 3 {
            MetricConfidence {
                level: Confidence::Low,
                reason: format!("Only {coupling_modules_measured} modules measured — coupling average may not be representative"),
            }
        } else {
            MetricConfidence {
                level: Confidence::High,
                reason: format!("{coupling_modules_measured} modules measured"),
            }
        };
        conf.insert("coupling".into(), coupling_conf);

        let blast_conf = if cross_file_call_edges < 5 {
            MetricConfidence {
                level: Confidence::Low,
                reason: "Sparse call graph — blast radius may be underreported".into(),
            }
        } else {
            MetricConfidence {
                level: Confidence::High,
                reason: "Sufficient call edges for blast radius computation".into(),
            }
        };
        conf.insert("blast_radius".into(), blast_conf);

        conf
    };

    // --- Compute per-metric status (data-contract integrity layer) ---
    // Populates the `status` field on each MetricDetail so downstream
    // consumers (desktop UI, CI action, cloud dashboards) don't have to
    // re-implement "is this number trustworthy?" checks. See
    // `metric_status` module for the rules.
    let status_inputs = metric_status::StatusInputs {
        graph: &ctx.graph,
        thresholds: &ctx.config.thresholds,
        ecosystem: metric_status::Ecosystem::classify(ctx.config.resolved_ecosystem()),
        resolution: Some(&ctx.resolution_quality),
        cycles_computation_ok: ctx.cycle_result.is_some(),
        coupling_computation_ok: ctx.coupling_result.is_some(),
        depth_computation_ok: ctx.depth_result.is_some(),
        dead_code_computation_ok: ctx.dead_result.is_some(),
    };
    let cycles_status_value = metric_status::cycles_status(&status_inputs);
    let tangle_status_value = metric_status::tangle_status(&status_inputs);
    let depth_status_value = metric_status::depth_status(&status_inputs);
    let dead_code_status_value = metric_status::dead_code_status(&status_inputs);
    let coupling_status_value =
        metric_status::coupling_status(&status_inputs, coupling_modules_measured);
    // Hotspot concentration is a percentile over fan-in; it is meaningful
    // whenever fan metrics computed successfully, regardless of ctx.graph
    // density. Treat it the same as dead-code for status purposes.
    let hotspot_status_value = if ctx.fan_result.is_some() {
        dead_code_status_value
    } else {
        report::MetricStatus::ComputationFailed
    };

    let metrics_report = MetricsReport {
        cycles: MetricDetail {
            count: ctx.total_cycle_nodes,
            total: ctx.graph.total_nodes(),
            ratio: metric_values.cycle_ratio,
            description: format!(
                "{} of {} nodes participate in {} dependency cycles",
                ctx.total_cycle_nodes,
                ctx.graph.total_nodes(),
                ctx.cycle_result.as_ref().map(|c| c.cycles.len()).unwrap_or(0),
            ),
            status: cycles_status_value,
            fidelity: cycles_fidelity,
        },
        coupling: CouplingMetricDetail {
            modules_measured: coupling_modules_measured,
            modules_above_070: coupling_above_070,
            modules_above_050: coupling_above_050,
            avg_coupling: avg_coupling_val,
            description: format!(
                "{} of {} modules exceed 0.70 coupling (avg {:.3})",
                coupling_above_070, coupling_modules_measured, avg_coupling_val,
            ),
            status: coupling_status_value,
            fidelity: Some(FidelityGap::from_values(
                avg_coupling_val,
                coupling_high_only.avg_coupling_excluding_tests,
                all_edges_count_for_fidelity,
                high_only_edges_count_for_fidelity,
            )),
        },
        hotspot_concentration: MetricDetail {
            count: hotspot_fns,
            total: total_fns,
            ratio: concentration_ratio,
            description: format!(
                "{} hotspot functions (top 5%) absorb {:.1}% of all incoming dependencies",
                hotspot_fns,
                concentration_ratio * 100.0,
            ),
            status: hotspot_status_value,
            fidelity: Some(FidelityGap::from_values(
                concentration_ratio,
                concentration_high_only,
                all_edges_count_for_fidelity,
                high_only_edges_count_for_fidelity,
            )),
        },
        dead_code: DeadCodeMetricDetail {
            count: dead_fn_count,
            total: total_fns,
            ratio: metric_values.dead_ratio,
            description: format!(
                "{} of {} functions have zero incoming non-containment edges ({:.1}%)",
                dead_fn_count, total_fns, metric_values.dead_ratio * 100.0,
            ),
            status: dead_code_status_value,
            reason_breakdown: ctx.dead_code_reason_breakdown.clone(),
            // B3: surface per-bucket caveats whenever the dominant
            // language's framework-attribute extraction is incomplete.
            // A 0 in `framework_annotation_unresolved` (or sibling
            // buckets) is structurally unable to be non-zero for
            // those languages today; consumers MUST surface this
            // alongside the bucket count instead of trusting 0 as
            // "no hits found." See `dead_code_confidence` for the
            // per-ecosystem caveat table.
            reason_breakdown_caveats: dead_code_confidence::reason_breakdown_caveats(
                ctx.config.resolved_ecosystem(),
            ),
            fidelity: dead_code_fidelity,
            // T8 companion counters. Populated post-classifier by
            // `coverage_attach::recompute_no_callers_confidence_split`
            // once the attach step has run. Leaving `None` here
            // keeps reports produced before the attach (or with
            // `--no-git-signals`/similar skip paths) honest about
            // the absence of coverage evidence.
            no_callers_total: None,
            no_callers_high_confidence: None,
        },
        depth: DepthMetricDetail {
            max_call_depth: max_depth,
            description: format!("Longest call chain is {} calls deep", max_depth),
            status: depth_status_value,
            fidelity: depth_fidelity,
        },
        tangle_index: MetricDetail {
            count: ctx.edges_in_cycles,
            total: total_structural_edges,
            ratio: ctx.tangle_idx,
            description: format!(
                "{:.2}% of structural edges participate in cycles",
                ctx.tangle_idx * 100.0,
            ),
            status: tangle_status_value,
            fidelity: tangle_fidelity,
        },
        complexity: ctx.complexity_result.as_ref().map(|cr| {
            report::ComplexityMetricDetail {
                avg_cyclomatic: cr.avg_cyclomatic,
                avg_cognitive: cr.avg_cognitive,
                max_cyclomatic: cr.max_cyclomatic,
                max_cognitive: cr.max_cognitive,
                functions_above_threshold: cr.functions_above_threshold,
                description: format!(
                    "Average cyclomatic complexity {:.1} ({} functions above warning threshold)",
                    cr.avg_cyclomatic, cr.functions_above_threshold,
                ),
            }
        }),
        cohesion: ctx.cohesion_result.as_ref().map(|cr| {
            let scores: Vec<f64> = cr.modules.values().map(|m| m.cohesion_score).collect();
            let avg = if scores.is_empty() { 1.0 } else { scores.iter().sum::<f64>() / scores.len() as f64 };
            let low_count = cr.modules.values().filter(|m| m.cohesion_score < ctx.config.thresholds.cohesion_finding).count();
            report::CohesionMetricDetail {
                avg_cohesion: avg,
                low_cohesion_modules: low_count,
                description: format!(
                    "Average module cohesion {:.2} ({} modules below {:.0}% threshold)",
                    avg, low_count, ctx.config.thresholds.cohesion_finding * 100.0,
                ),
                fidelity: cohesion_high_only_avg.map(|ho| FidelityGap::from_values(
                    avg,
                    ho,
                    all_edges_count_for_fidelity,
                    high_only_edges_count_for_fidelity,
                )),
            }
        }),
        distance_from_main_sequence: ctx.distance_result.as_ref().map(|dr| {
            let distances: Vec<f64> = dr.values().map(|d| d.distance).collect();
            let avg = if distances.is_empty() { 0.0 } else { distances.iter().sum::<f64>() / distances.len() as f64 };
            let pain_count = dr.values().filter(|d| d.zone == distance_from_main_sequence::Zone::ZoneOfPain).count();
            let useless_count = dr.values().filter(|d| d.zone == distance_from_main_sequence::Zone::ZoneOfUselessness).count();
            report::DistanceMetricDetail {
                avg_distance: avg,
                zone_of_pain_modules: pain_count,
                zone_of_uselessness_modules: useless_count,
                description: format!(
                    "Average distance {:.2} ({} in zone of pain, {} in zone of uselessness)",
                    avg, pain_count, useless_count,
                ),
                fidelity: distance_high_only_avg.map(|ho| FidelityGap::from_values(
                    avg,
                    ho,
                    all_edges_count_for_fidelity,
                    high_only_edges_count_for_fidelity,
                )),
            }
        }),
        temporal_coupling: ctx.temporal_result.as_ref().map(|tr| {
            {
                let hidden_module_pairs = tr.module_pairs.iter().filter(|p| !p.has_import_coupling).count();
                let top_pairs: Vec<report::ModuleTemporalPair> = tr.module_pairs
                    .iter()
                    .take(20)
                    .map(|mp| report::ModuleTemporalPair {
                        module_a: mp.module_a.clone(),
                        module_b: mp.module_b.clone(),
                        co_change_count: mp.co_change_count,
                        coupling_score: mp.coupling_score,
                        has_import_coupling: mp.has_import_coupling,
                    })
                    .collect();

                let module_level = if tr.module_pairs.is_empty() {
                    None
                } else {
                    Some(report::ModuleTemporalCouplingDetail {
                        total_module_pairs: tr.module_pairs.len(),
                        hidden_module_pairs,
                        top_pairs,
                    })
                };

                report::TemporalCouplingMetricDetail {
                    high_coupling_pairs: tr.high_coupling_pairs,
                    hidden_coupling_pairs: tr.hidden_coupling_pairs,
                    description: format!(
                        "{} file pairs with temporal coupling >= {:.0}% ({} with no import relationship)",
                        tr.high_coupling_pairs,
                        ctx.config.thresholds.temporal_hidden_high_score * 100.0,
                        tr.hidden_coupling_pairs,
                    ),
                    module_level,
                }
            }
        }),
        metric_confidence: Some(metric_confidence),
    };

    // --- Build Layer B: PercentilesReport (only when norms available) ---
    let population = ctx
        .norms_path
        .and_then(|path| match norms::load_population(path) {
            Ok(pop) => Some(pop),
            Err(e) => {
                eprintln!("[ge-analyze] Warning: could not load norms database: {e}");
                ctx.analysis_errors.push(AnalysisError {
                    algorithm: "percentiles".into(),
                    error: format!("Failed to load norms database: {e}"),
                    nodes_affected: None,
                });
                None
            }
        });

    let percentiles_report = population.as_ref().map(|pop| {
        let version = norms::population_version(pop);
        health_score::build_percentiles(&metric_values, pop, &version, &effective_weights)
    });

    // --- Compute health score ---
    let (hs_value, components) = if let Some(ref pct) = percentiles_report {
        let (score, comps) = health_score::score_from_percentiles(&effective_weights, pct);
        (Some(score), comps)
    } else {
        let (score, comps) = health_score::score_from_formulas(&effective_weights, &score_inputs);
        (Some(score), comps)
    };

    // --- Build summary ---
    let summary = Summary {
        total_nodes: ctx.graph.total_nodes(),
        total_edges: ctx.graph.total_edges(),
        total_functions: ctx.graph.total_functions(),
        total_modules: ctx.graph.total_folder_modules(),
        cycles_found: ctx
            .cycle_result
            .as_ref()
            .map(|c| c.cycles.len())
            .unwrap_or(0),
        cycle_total_nodes: ctx.total_cycle_nodes,
        hotspot_count: fan_ref.map(|f| f.hotspot_count).unwrap_or(0),
        hotspot_threshold_fan_in: fan_ref.map(|f| f.hotspot_threshold).unwrap_or(0),
        high_coupling_modules: ctx
            .coupling_result
            .as_ref()
            .map(|c| {
                c.modules
                    .iter()
                    .filter(|(k, m)| {
                        is_production_module(k.as_str())
                            && m.coupling_score > ctx.config.thresholds.high_coupling_threshold
                    })
                    .count()
            })
            .unwrap_or(0),
        dead_functions: dead_fn_count,
        max_call_depth: max_depth,
        tangle_index: ctx.tangle_idx,
        avg_module_coupling: avg_coupling_val,
        avg_fan_in: if ctx.graph.total_functions() > 0 {
            fan_ref.map(|f| f.total_fan_in).unwrap_or(0) as f64 / ctx.graph.total_functions() as f64
        } else {
            0.0
        },
        avg_fan_out: if ctx.graph.total_functions() > 0 {
            let total_fo: usize = ctx
                .graph
                .function_node_ids
                .iter()
                .map(|id| ctx.graph.fan_out(id))
                .sum();
            total_fo as f64 / ctx.graph.total_functions() as f64
        } else {
            0.0
        },
    };

    let elapsed = ctx.start.elapsed().as_millis() as u64;

    // --- Build module classifications ---
    let classifications = build_classifications(&ctx.graph, &ctx.config);

    let invariant_violations = ctx
        .analysis_errors
        .iter()
        .any(|e| e.algorithm == "graph_invariants");

    let mut report = HealthReport {
        version: "1.0.0".into(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        analysis_duration_ms: elapsed,
        db_path: ctx.db_path.clone(),
        health_score: hs_value,
        health_score_components: components,
        metrics: metrics_report,
        percentiles: percentiles_report,
        summary,
        findings: std::mem::take(&mut ctx.findings),
        node_annotations: std::mem::take(&mut ctx.node_annotations),
        module_annotations: std::mem::take(&mut ctx.module_annotations),
        classifications,
        boundary_violations: vec![],
        resolution_quality: Some(ctx.resolution_quality.clone()),
        analysis_errors: std::mem::take(&mut ctx.analysis_errors),
        integrity_status: build_integrity_status(
            invariant_violations,
            ctx.stale_parse_db,
            ctx.graph.unknown_edge_kind_count(),
        ),
        git_signals: None,
        file_extraction_coverage: Vec::new(),
        primary_language: None,
        analysis_provenance: None,
    };

    // A3: compute and persist canonical primary language. The parser
    // can't reliably set this on its own because the polyglot
    // orchestrator runs one pipeline pass per language and clobbers
    // the Project node's `properties.language` with each pass's value
    // (last-writer-wins via `INSERT OR REPLACE INTO nodes …`).
    //
    // We compute it here from File-node majority — the same signal
    // `detect_ecosystem` already uses for routing — and then write
    // the result back into the Project node so any downstream
    // consumer that queries the ctx.graph DB directly sees the canonical
    // value instead of the parser's last guess.
    //
    // The write-back is best-effort: failures are logged and ignored
    // so an unwritable ctx.graph DB (e.g. read-only mount) does not block
    // the ctx.report. The in-memory ctx.report still carries the right value.
    report.primary_language = graph::detect_primary_language(&ctx.conn);
    if let Some(lang) = report.primary_language.as_deref() {
        if let Err(err) = persist_project_language(&ctx.db_path, lang) {
            eprintln!(
                "[ge-analyze] WARNING: failed to write canonical primary_language='{}' back to graph DB Project node: {}. The HealthReport still carries the correct value.",
                lang, err
            );
        }
    }

    eprintln!(
        "[ge-analyze] Health score: {}/100 | {} findings | completed in {:.1}s",
        report.health_score.unwrap_or(0),
        report.findings.len(),
        elapsed as f64 / 1000.0,
    );

    ctx.report = Some(report);
    Ok(None)
}
