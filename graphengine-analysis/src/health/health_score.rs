//! Health score computation.
//!
//! Two modes:
//!   1. **Percentile-based** (preferred): When a norms database is provided, the composite
//!      health score equals the weighted average of per-metric percentiles. Each sub-score
//!      in `HealthScoreComponents` is the metric's percentile (0–100).
//!   2. **Fallback** (no norms): During the transition period, a simple formula-based
//!      score is emitted so existing consumers aren't broken. This uses direct ratios
//!      rather than the old quadratic curves — intentionally simpler and less opinionated.

use std::collections::BTreeMap;

use super::config::ScoreWeights;
use super::report::{HealthScoreComponents, PercentileEntry, PercentilesReport, ScoreComponent};

pub struct ScoreInputs {
    pub total_nodes: usize,
    pub total_cycle_nodes: usize,
    pub avg_coupling_score: f64,
    /// Ecosystem-specific coupling baseline. Coupling at or below this value is
    /// considered healthy (scores 100). Only coupling above this incurs penalties.
    pub coupling_baseline: f64,
    pub sum_hotspot_fan_in: usize,
    pub total_fan_in: usize,
    pub dead_functions: usize,
    pub total_functions: usize,
    pub max_call_depth: usize,
    pub avg_cyclomatic: f64,
    pub avg_cohesion: f64,
    pub avg_distance: f64,
    pub hidden_coupling_pairs: usize,
    pub total_file_pairs_analyzed: usize,
}

/// Metric values extracted from the analysis, used for percentile lookups.
pub struct MetricValues {
    pub cycle_ratio: f64,
    pub avg_coupling: f64,
    pub dead_ratio: f64,
    pub hotspot_concentration: f64,
    pub max_depth: usize,
    pub tangle_index: f64,
    pub avg_cyclomatic: f64,
    pub avg_cohesion: f64,
    pub avg_distance: f64,
    pub temporal_coupling_score: f64,
}

/// Compute health score + components from a populated `PercentilesReport`.
/// Each component score becomes the metric's percentile (0–100).
pub fn score_from_percentiles(
    weights: &ScoreWeights,
    percentiles: &PercentilesReport,
) -> (u32, HealthScoreComponents) {
    let get_pct = |key: &str| -> u32 {
        percentiles
            .per_metric
            .get(key)
            .map(|e| e.percentile)
            .unwrap_or(50)
    };

    let cycle_pct = get_pct("cycle_ratio");
    let coupling_pct = get_pct("avg_coupling");
    let hotspot_pct = get_pct("hotspot_concentration");
    let dead_pct = get_pct("dead_ratio");
    let depth_pct = get_pct("max_depth");
    let complexity_pct = get_pct("avg_cyclomatic");
    let cohesion_pct = get_pct("avg_cohesion");
    let distance_pct = get_pct("avg_distance");
    let temporal_pct = get_pct("temporal_coupling_score");

    let composite = (cycle_pct as f64 * weights.cycle_severity)
        + (coupling_pct as f64 * weights.coupling_health)
        + (hotspot_pct as f64 * weights.hotspot_concentration)
        + (dead_pct as f64 * weights.dead_code_ratio)
        + (depth_pct as f64 * weights.depth_complexity)
        + (complexity_pct as f64 * weights.complexity)
        + (cohesion_pct as f64 * weights.cohesion)
        + (distance_pct as f64 * weights.distance)
        + (temporal_pct as f64 * weights.temporal_coupling);

    let final_score = composite.round() as u32;

    let components = HealthScoreComponents {
        cycle_severity: ScoreComponent {
            score: cycle_pct,
            weight: weights.cycle_severity,
        },
        coupling_health: ScoreComponent {
            score: coupling_pct,
            weight: weights.coupling_health,
        },
        hotspot_concentration: ScoreComponent {
            score: hotspot_pct,
            weight: weights.hotspot_concentration,
        },
        dead_code_ratio: ScoreComponent {
            score: dead_pct,
            weight: weights.dead_code_ratio,
        },
        depth_complexity: ScoreComponent {
            score: depth_pct,
            weight: weights.depth_complexity,
        },
        complexity: ScoreComponent {
            score: complexity_pct,
            weight: weights.complexity,
        },
        cohesion: ScoreComponent {
            score: cohesion_pct,
            weight: weights.cohesion,
        },
        distance: ScoreComponent {
            score: distance_pct,
            weight: weights.distance,
        },
        temporal_coupling: ScoreComponent {
            score: temporal_pct,
            weight: weights.temporal_coupling,
        },
    };

    (final_score, components)
}

/// Fallback: formula-based score when no population data is available.
/// Uses simple linear penalties (not quadratic curves). Intentionally conservative
/// so users are not misled by a number with no external meaning.
pub fn score_from_formulas(
    weights: &ScoreWeights,
    inputs: &ScoreInputs,
) -> (u32, HealthScoreComponents) {
    let cycle_score = fallback_cycle(inputs.total_cycle_nodes, inputs.total_nodes);
    let coupling_score = fallback_coupling(inputs.avg_coupling_score, inputs.coupling_baseline);
    let hotspot_score = fallback_hotspot(inputs.sum_hotspot_fan_in, inputs.total_fan_in);
    let dead_code_score = fallback_dead_code(inputs.dead_functions, inputs.total_functions);
    let depth_score = fallback_depth(inputs.max_call_depth);
    let complexity_score = fallback_complexity(inputs.avg_cyclomatic);
    let cohesion_score = fallback_cohesion(inputs.avg_cohesion);
    let distance_score = fallback_distance(inputs.avg_distance);
    let temporal_score = fallback_temporal(
        inputs.hidden_coupling_pairs,
        inputs.total_file_pairs_analyzed,
    );

    let composite = (cycle_score as f64 * weights.cycle_severity)
        + (coupling_score as f64 * weights.coupling_health)
        + (hotspot_score as f64 * weights.hotspot_concentration)
        + (dead_code_score as f64 * weights.dead_code_ratio)
        + (depth_score as f64 * weights.depth_complexity)
        + (complexity_score as f64 * weights.complexity)
        + (cohesion_score as f64 * weights.cohesion)
        + (distance_score as f64 * weights.distance)
        + (temporal_score as f64 * weights.temporal_coupling);

    let final_score = composite.round() as u32;

    let components = HealthScoreComponents {
        cycle_severity: ScoreComponent {
            score: cycle_score,
            weight: weights.cycle_severity,
        },
        coupling_health: ScoreComponent {
            score: coupling_score,
            weight: weights.coupling_health,
        },
        hotspot_concentration: ScoreComponent {
            score: hotspot_score,
            weight: weights.hotspot_concentration,
        },
        dead_code_ratio: ScoreComponent {
            score: dead_code_score,
            weight: weights.dead_code_ratio,
        },
        depth_complexity: ScoreComponent {
            score: depth_score,
            weight: weights.depth_complexity,
        },
        complexity: ScoreComponent {
            score: complexity_score,
            weight: weights.complexity,
        },
        cohesion: ScoreComponent {
            score: cohesion_score,
            weight: weights.cohesion,
        },
        distance: ScoreComponent {
            score: distance_score,
            weight: weights.distance,
        },
        temporal_coupling: ScoreComponent {
            score: temporal_score,
            weight: weights.temporal_coupling,
        },
    };

    (final_score, components)
}

/// Build the `MetricValues` struct from raw `ScoreInputs` + tangle index.
pub fn extract_metric_values(inputs: &ScoreInputs, tangle_index: f64) -> MetricValues {
    MetricValues {
        cycle_ratio: if inputs.total_nodes > 0 {
            inputs.total_cycle_nodes as f64 / inputs.total_nodes as f64
        } else {
            0.0
        },
        avg_coupling: inputs.avg_coupling_score,
        dead_ratio: if inputs.total_functions > 0 {
            inputs.dead_functions as f64 / inputs.total_functions as f64
        } else {
            0.0
        },
        hotspot_concentration: if inputs.total_fan_in > 0 {
            inputs.sum_hotspot_fan_in as f64 / inputs.total_fan_in as f64
        } else {
            0.0
        },
        max_depth: inputs.max_call_depth,
        tangle_index,
        avg_cyclomatic: inputs.avg_cyclomatic,
        avg_cohesion: inputs.avg_cohesion,
        avg_distance: inputs.avg_distance,
        temporal_coupling_score: if inputs.total_file_pairs_analyzed > 0 {
            inputs.hidden_coupling_pairs as f64 / inputs.total_file_pairs_analyzed as f64
        } else {
            0.0
        },
    }
}

/// Build a `PercentilesReport` by ranking `values` against a population database.
pub fn build_percentiles(
    values: &MetricValues,
    population: &[(String, PopulationRow)],
    population_version: &str,
    weights: &ScoreWeights,
) -> PercentilesReport {
    let n = population.len();

    let compute = |value: f64,
                   extract: fn(&PopulationRow) -> Option<f64>,
                   label: &str,
                   lower_better: bool|
     -> PercentileEntry {
        let mut valid_count = 0usize;
        let mut worse_count = 0usize;
        let mut equal_count = 0usize;
        for (_, row) in population {
            if let Some(pop_val) = extract(row) {
                valid_count += 1;
                if lower_better {
                    if pop_val > value {
                        worse_count += 1;
                    } else if (pop_val - value).abs() < 1e-12 {
                        equal_count += 1;
                    }
                } else if pop_val < value {
                    worse_count += 1;
                } else if (pop_val - value).abs() < 1e-12 {
                    equal_count += 1;
                }
            }
        }
        let pct = if valid_count == 0 {
            50
        } else {
            let midpoint = worse_count as f64 + (equal_count as f64 / 2.0);
            ((midpoint / valid_count as f64) * 100.0).round() as u32
        };
        let direction = if lower_better { "Lower" } else { "Higher" };
        PercentileEntry {
            value,
            percentile: pct.min(100),
            description: format!(
                "{direction} {label} than {pct}% of {valid_count} analyzed projects"
            ),
        }
    };

    let mut per_metric = BTreeMap::new();
    per_metric.insert(
        "cycle_ratio".into(),
        compute(
            values.cycle_ratio,
            |r| Some(r.cycle_ratio),
            "cycle ratio",
            true,
        ),
    );
    per_metric.insert(
        "avg_coupling".into(),
        compute(values.avg_coupling, |r| r.avg_coupling, "coupling", true),
    );
    per_metric.insert(
        "dead_ratio".into(),
        compute(
            values.dead_ratio,
            |r| Some(r.dead_ratio),
            "dead code ratio",
            true,
        ),
    );
    per_metric.insert(
        "hotspot_concentration".into(),
        compute(
            values.hotspot_concentration,
            |r| Some(r.hotspot_concentration),
            "hotspot concentration",
            true,
        ),
    );
    per_metric.insert(
        "max_depth".into(),
        compute(
            values.max_depth as f64,
            |r| Some(r.max_depth as f64),
            "call depth",
            true,
        ),
    );
    per_metric.insert(
        "tangle_index".into(),
        compute(
            values.tangle_index,
            |r| Some(r.tangle_index),
            "tangle index",
            true,
        ),
    );
    per_metric.insert(
        "avg_cyclomatic".into(),
        compute(
            values.avg_cyclomatic,
            |r| r.avg_cyclomatic,
            "cyclomatic complexity",
            true,
        ),
    );
    per_metric.insert(
        "avg_cohesion".into(),
        compute(values.avg_cohesion, |r| r.avg_cohesion, "cohesion", false),
    );
    per_metric.insert(
        "avg_distance".into(),
        compute(
            values.avg_distance,
            |r| r.avg_distance,
            "distance from main sequence",
            true,
        ),
    );
    per_metric.insert(
        "temporal_coupling_score".into(),
        compute(
            values.temporal_coupling_score,
            |r| r.temporal_coupling_score,
            "temporal coupling",
            true,
        ),
    );

    let metrics_for_composite = [
        "cycle_ratio",
        "avg_coupling",
        "hotspot_concentration",
        "dead_ratio",
        "max_depth",
        "avg_cyclomatic",
        "avg_cohesion",
        "avg_distance",
        "temporal_coupling_score",
    ];
    let metric_weights = [
        weights.cycle_severity,
        weights.coupling_health,
        weights.hotspot_concentration,
        weights.dead_code_ratio,
        weights.depth_complexity,
        weights.complexity,
        weights.cohesion,
        weights.distance,
        weights.temporal_coupling,
    ];
    let composite: f64 = metrics_for_composite
        .iter()
        .zip(metric_weights.iter())
        .map(|(key, w)| {
            per_metric
                .get(*key)
                .map(|e| e.percentile as f64)
                .unwrap_or(50.0)
                * w
        })
        .sum();

    PercentilesReport {
        population_size: n,
        population_version: population_version.to_string(),
        composite_percentile: composite.round() as u32,
        per_metric,
    }
}

/// A row from the population (norms) database.
#[derive(Debug, Clone)]
pub struct PopulationRow {
    pub cycle_ratio: f64,
    pub avg_coupling: Option<f64>,
    pub dead_ratio: f64,
    pub hotspot_concentration: f64,
    pub max_depth: usize,
    pub tangle_index: f64,
    pub avg_cyclomatic: Option<f64>,
    pub avg_cohesion: Option<f64>,
    pub avg_distance: Option<f64>,
    pub temporal_coupling_score: Option<f64>,
}

// ---------------------------------------------------------------------------
// Fallback formulas (simple linear, used only when no norms are available)
// ---------------------------------------------------------------------------

fn fallback_cycle(total_cycle_nodes: usize, total_nodes: usize) -> u32 {
    if total_nodes == 0 {
        return 100;
    }
    let ratio = total_cycle_nodes as f64 / total_nodes as f64;
    let raw = 100.0 - (ratio * 500.0).min(100.0);
    raw.round().max(0.0) as u32
}

/// Ecosystem-calibrated coupling score. Coupling at or below `baseline` scores 100.
/// Above baseline, a linear penalty applies: coupling of 1.0 scores 0.
fn fallback_coupling(avg_coupling: f64, baseline: f64) -> u32 {
    let c = avg_coupling.clamp(0.0, 1.0);
    let b = baseline.clamp(0.0, 0.95);
    if c <= b {
        return 100;
    }
    let headroom = 1.0 - b;
    if headroom <= 0.0 {
        return 0;
    }
    let excess = c - b;
    let raw = 100.0 * (1.0 - excess / headroom);
    raw.round().max(0.0) as u32
}

fn fallback_hotspot(sum_hotspot_fan_in: usize, total_fan_in: usize) -> u32 {
    if total_fan_in == 0 {
        return 100;
    }
    let ratio = (sum_hotspot_fan_in as f64 / total_fan_in as f64).clamp(0.0, 1.0);
    let raw = 100.0 * (1.0 - ratio);
    raw.round().max(0.0) as u32
}

fn fallback_dead_code(dead_functions: usize, total_functions: usize) -> u32 {
    if total_functions == 0 {
        return 100;
    }
    let ratio = (dead_functions as f64 / total_functions as f64).clamp(0.0, 1.0);
    let raw = 100.0 - (ratio * 300.0).min(100.0);
    raw.round().max(0.0) as u32
}

fn fallback_depth(max_call_depth: usize) -> u32 {
    let normalized = (max_call_depth as f64 / 30.0).clamp(0.0, 1.0);
    let raw = 100.0 * (1.0 - normalized);
    raw.round().max(0.0) as u32
}

/// Lower avg cyclomatic = better. 0 → 100, ≥25 → 0.
fn fallback_complexity(avg_cyclomatic: f64) -> u32 {
    if avg_cyclomatic <= 0.0 {
        return 100;
    }
    let normalized = (avg_cyclomatic / 25.0).clamp(0.0, 1.0);
    let raw = 100.0 * (1.0 - normalized);
    raw.round().max(0.0) as u32
}

/// Higher cohesion = better. 1.0 → 100, 0.0 → 0.
fn fallback_cohesion(avg_cohesion: f64) -> u32 {
    let c = avg_cohesion.clamp(0.0, 1.0);
    (c * 100.0).round() as u32
}

/// Lower distance = better. 0.0 → 100, 1.0 → 0.
fn fallback_distance(avg_distance: f64) -> u32 {
    let d = avg_distance.clamp(0.0, 1.0);
    let raw = 100.0 * (1.0 - d);
    raw.round().max(0.0) as u32
}

/// Fewer hidden coupling pairs = better. 0 hidden → 100, linear penalty.
fn fallback_temporal(hidden_coupling_pairs: usize, total_pairs: usize) -> u32 {
    if total_pairs == 0 || hidden_coupling_pairs == 0 {
        return 100;
    }
    let ratio = (hidden_coupling_pairs as f64 / total_pairs as f64).clamp(0.0, 1.0);
    let raw = 100.0 * (1.0 - ratio);
    raw.round().max(0.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_weights() -> ScoreWeights {
        ScoreWeights::default()
    }

    #[test]
    fn perfect_health_fallback() {
        let inputs = ScoreInputs {
            total_nodes: 100,
            total_cycle_nodes: 0,
            avg_coupling_score: 0.0,
            coupling_baseline: 0.35,
            sum_hotspot_fan_in: 0,
            total_fan_in: 100,
            dead_functions: 0,
            total_functions: 50,
            max_call_depth: 0,
            avg_cyclomatic: 0.0,
            avg_cohesion: 1.0,
            avg_distance: 0.0,
            hidden_coupling_pairs: 0,
            total_file_pairs_analyzed: 0,
        };
        let (score, _) = score_from_formulas(&default_weights(), &inputs);
        assert_eq!(score, 100);
    }

    #[test]
    fn terrible_health_fallback() {
        let inputs = ScoreInputs {
            total_nodes: 100,
            total_cycle_nodes: 30,
            avg_coupling_score: 0.95,
            coupling_baseline: 0.35,
            sum_hotspot_fan_in: 95,
            total_fan_in: 100,
            dead_functions: 45,
            total_functions: 50,
            max_call_depth: 28,
            avg_cyclomatic: 25.0,
            avg_cohesion: 0.0,
            avg_distance: 1.0,
            hidden_coupling_pairs: 20,
            total_file_pairs_analyzed: 20,
        };
        let (score, _) = score_from_formulas(&default_weights(), &inputs);
        assert!(score < 20, "Score should be very low: {score}");
    }

    #[test]
    fn empty_graph_fallback() {
        let inputs = ScoreInputs {
            total_nodes: 0,
            total_cycle_nodes: 0,
            avg_coupling_score: 0.0,
            coupling_baseline: 0.35,
            sum_hotspot_fan_in: 0,
            total_fan_in: 0,
            dead_functions: 0,
            total_functions: 0,
            max_call_depth: 0,
            avg_cyclomatic: 0.0,
            avg_cohesion: 1.0,
            avg_distance: 0.0,
            hidden_coupling_pairs: 0,
            total_file_pairs_analyzed: 0,
        };
        let (score, _) = score_from_formulas(&default_weights(), &inputs);
        assert_eq!(score, 100);
    }

    #[test]
    fn percentile_based_scoring() {
        let weights = default_weights();
        let mut per_metric = BTreeMap::new();
        per_metric.insert(
            "cycle_ratio".into(),
            PercentileEntry {
                value: 0.001,
                percentile: 80,
                description: String::new(),
            },
        );
        per_metric.insert(
            "avg_coupling".into(),
            PercentileEntry {
                value: 0.5,
                percentile: 60,
                description: String::new(),
            },
        );
        per_metric.insert(
            "hotspot_concentration".into(),
            PercentileEntry {
                value: 0.3,
                percentile: 70,
                description: String::new(),
            },
        );
        per_metric.insert(
            "dead_ratio".into(),
            PercentileEntry {
                value: 0.05,
                percentile: 75,
                description: String::new(),
            },
        );
        per_metric.insert(
            "max_depth".into(),
            PercentileEntry {
                value: 8.0,
                percentile: 65,
                description: String::new(),
            },
        );
        per_metric.insert(
            "avg_cyclomatic".into(),
            PercentileEntry {
                value: 5.0,
                percentile: 70,
                description: String::new(),
            },
        );
        per_metric.insert(
            "avg_cohesion".into(),
            PercentileEntry {
                value: 0.8,
                percentile: 60,
                description: String::new(),
            },
        );
        per_metric.insert(
            "avg_distance".into(),
            PercentileEntry {
                value: 0.3,
                percentile: 55,
                description: String::new(),
            },
        );
        per_metric.insert(
            "temporal_coupling_score".into(),
            PercentileEntry {
                value: 0.1,
                percentile: 50,
                description: String::new(),
            },
        );

        let report = PercentilesReport {
            population_size: 300,
            population_version: "test".into(),
            composite_percentile: 72,
            per_metric,
        };

        let (score, components) = score_from_percentiles(&weights, &report);
        assert_eq!(components.cycle_severity.score, 80);
        assert_eq!(components.coupling_health.score, 60);
        assert_eq!(components.complexity.score, 70);
        assert!(score > 50 && score < 80);
    }

    #[test]
    fn build_percentiles_midpoint_ties() {
        let population: Vec<(String, PopulationRow)> = (0..10)
            .map(|i| {
                (
                    format!("repo-{i}"),
                    PopulationRow {
                        cycle_ratio: i as f64 * 0.01,
                        avg_coupling: Some(i as f64 * 0.1),
                        dead_ratio: i as f64 * 0.05,
                        hotspot_concentration: i as f64 * 0.1,
                        max_depth: i * 2,
                        tangle_index: i as f64 * 0.01,
                        avg_cyclomatic: Some(i as f64 * 2.0),
                        avg_cohesion: Some(1.0 - i as f64 * 0.1),
                        avg_distance: Some(i as f64 * 0.1),
                        temporal_coupling_score: Some(i as f64 * 0.05),
                    },
                )
            })
            .collect();

        let values = MetricValues {
            cycle_ratio: 0.05,
            avg_coupling: 0.5,
            dead_ratio: 0.25,
            hotspot_concentration: 0.5,
            max_depth: 10,
            tangle_index: 0.05,
            avg_cyclomatic: 10.0,
            avg_cohesion: 0.5,
            avg_distance: 0.5,
            temporal_coupling_score: 0.25,
        };

        let report = build_percentiles(&values, &population, "test", &default_weights());
        assert_eq!(report.population_size, 10);
        for entry in report.per_metric.values() {
            assert!(entry.percentile <= 100);
        }
    }

    #[test]
    fn coupling_baseline_calibration() {
        // Rust ecosystem: baseline 0.75, avg coupling 0.875
        // Expected: 100 * (1 - (0.875-0.75)/(1-0.75)) = 100 * 0.50 = 50
        let inputs = ScoreInputs {
            total_nodes: 100,
            total_cycle_nodes: 0,
            avg_coupling_score: 0.875,
            coupling_baseline: 0.75,
            sum_hotspot_fan_in: 0,
            total_fan_in: 100,
            dead_functions: 0,
            total_functions: 50,
            max_call_depth: 0,
            avg_cyclomatic: 0.0,
            avg_cohesion: 1.0,
            avg_distance: 0.0,
            hidden_coupling_pairs: 0,
            total_file_pairs_analyzed: 0,
        };
        let (_, comps) = score_from_formulas(&default_weights(), &inputs);
        assert_eq!(comps.coupling_health.score, 50);

        // Same coupling with default baseline (0.35) — much harsher
        let inputs_default = ScoreInputs {
            coupling_baseline: 0.35,
            ..inputs
        };
        let (_, comps_default) = score_from_formulas(&default_weights(), &inputs_default);
        assert!(
            comps_default.coupling_health.score < comps.coupling_health.score,
            "Default baseline should score lower: {} vs {}",
            comps_default.coupling_health.score,
            comps.coupling_health.score,
        );

        // Coupling at baseline → 100
        let inputs_at_baseline = ScoreInputs {
            avg_coupling_score: 0.75,
            coupling_baseline: 0.75,
            ..inputs
        };
        let (_, comps_at) = score_from_formulas(&default_weights(), &inputs_at_baseline);
        assert_eq!(comps_at.coupling_health.score, 100);

        // Coupling below baseline → 100
        let inputs_below = ScoreInputs {
            avg_coupling_score: 0.50,
            coupling_baseline: 0.75,
            ..inputs
        };
        let (_, comps_below) = score_from_formulas(&default_weights(), &inputs_below);
        assert_eq!(comps_below.coupling_health.score, 100);
    }

    #[test]
    fn without_temporal_redistributes_weight() {
        let weights = ScoreWeights::default();
        let no_temporal = weights.without_temporal();
        assert_eq!(no_temporal.temporal_coupling, 0.0);
        let sum = no_temporal.cycle_severity
            + no_temporal.coupling_health
            + no_temporal.hotspot_concentration
            + no_temporal.dead_code_ratio
            + no_temporal.depth_complexity
            + no_temporal.complexity
            + no_temporal.cohesion
            + no_temporal.distance
            + no_temporal.temporal_coupling;
        assert!(
            (sum - 1.0).abs() < 0.001,
            "Redistributed weights should sum to 1.0, got {sum}"
        );
    }
}
