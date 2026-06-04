//! Baseline delta between two `HealthReport`s for the same project.
//!
//! Implements `docs/02-strategy/DIAGNOSTIC_PRODUCT_SPEC.md` §3. Findings are matched
//! heuristically via `(finding_type, primary_node_id)`-first, falling back to
//! `(finding_type, sorted node_ids)` for multi-node findings like cycles.

use std::collections::{BTreeMap, BTreeSet};

use graphengine_analysis::health::report::{
    Finding, FindingType, HealthReport, ModuleAnnotation, Severity,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Improved,
    Worsened,
    Stable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricDelta {
    pub metric: String,
    pub baseline_value: f64,
    pub current_value: f64,
    pub delta: f64,
    pub direction: TrendDirection,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FindingDelta {
    pub finding_id: String,
    pub finding_type: FindingType,
    pub target: String,
    pub baseline_severity: Severity,
    pub current_severity: Severity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModuleDelta {
    pub module: String,
    pub coupling_delta: f64,
    pub new_violations: usize,
    pub new_cycle_membership: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrendReport {
    pub baseline_date: String,
    pub current_date: String,
    pub baseline_score: Option<u32>,
    pub current_score: Option<u32>,
    pub score_delta: Option<i32>,
    pub metric_deltas: BTreeMap<String, MetricDelta>,
    pub new_findings: Vec<String>,
    pub resolved_findings: Vec<String>,
    pub worsened_findings: Vec<FindingDelta>,
    pub modules_worsening: Vec<ModuleDelta>,
}

/// Compute the full trend delta between `baseline` and `current`.
pub fn diff(baseline: &HealthReport, current: &HealthReport) -> TrendReport {
    let score_delta = match (baseline.health_score, current.health_score) {
        (Some(b), Some(c)) => Some(c as i32 - b as i32),
        _ => None,
    };

    TrendReport {
        baseline_date: baseline.generated_at.clone(),
        current_date: current.generated_at.clone(),
        baseline_score: baseline.health_score,
        current_score: current.health_score,
        score_delta,
        metric_deltas: metric_diffs(baseline, current),
        new_findings: new_finding_ids(baseline, current),
        resolved_findings: resolved_finding_ids(baseline, current),
        worsened_findings: worsened(baseline, current),
        modules_worsening: module_deltas(baseline, current),
    }
}

fn finding_key(f: &Finding) -> (FindingType, String) {
    // Prefer `primary_node_id`; fall back to sorted `node_ids` join for
    // multi-node findings (cycles, temporal coupling pairs).
    let key = f.primary_node_id.clone().unwrap_or_else(|| {
        let mut ids = f.node_ids.clone();
        ids.sort();
        ids.join("|")
    });
    (f.finding_type, key)
}

fn new_finding_ids(baseline: &HealthReport, current: &HealthReport) -> Vec<String> {
    let baseline_keys: BTreeSet<_> = baseline.findings.iter().map(finding_key).collect();
    current
        .findings
        .iter()
        .filter(|f| !baseline_keys.contains(&finding_key(f)))
        .map(|f| f.id.clone())
        .collect()
}

fn resolved_finding_ids(baseline: &HealthReport, current: &HealthReport) -> Vec<String> {
    let current_keys: BTreeSet<_> = current.findings.iter().map(finding_key).collect();
    baseline
        .findings
        .iter()
        .filter(|f| !current_keys.contains(&finding_key(f)))
        .map(|f| f.id.clone())
        .collect()
}

fn worsened(baseline: &HealthReport, current: &HealthReport) -> Vec<FindingDelta> {
    let baseline_map: BTreeMap<_, _> = baseline
        .findings
        .iter()
        .map(|f| (finding_key(f), f))
        .collect();

    current
        .findings
        .iter()
        .filter_map(|cur| {
            let bl = baseline_map.get(&finding_key(cur))?;
            if cur.severity.rank() > bl.severity.rank() {
                Some(FindingDelta {
                    finding_id: cur.id.clone(),
                    finding_type: cur.finding_type,
                    target: cur
                        .primary_node_id
                        .clone()
                        .or_else(|| cur.node_ids.first().cloned())
                        .unwrap_or_default(),
                    baseline_severity: bl.severity,
                    current_severity: cur.severity,
                })
            } else {
                None
            }
        })
        .collect()
}

fn metric_diffs(baseline: &HealthReport, current: &HealthReport) -> BTreeMap<String, MetricDelta> {
    let mut out = BTreeMap::new();

    // Only the first-class scalar metrics produce stable deltas that the UI
    // can render as a sentence. Counts-with-totals (cycles, dead_code, etc.)
    // would need their ratios, which already are in these structs.
    push_scalar(
        &mut out,
        "avg_coupling",
        baseline.metrics.coupling.avg_coupling,
        current.metrics.coupling.avg_coupling,
        Direction::LowerIsBetter,
    );
    push_scalar(
        &mut out,
        "cycle_ratio",
        baseline.metrics.cycles.ratio,
        current.metrics.cycles.ratio,
        Direction::LowerIsBetter,
    );
    push_scalar(
        &mut out,
        "dead_code_ratio",
        baseline.metrics.dead_code.ratio,
        current.metrics.dead_code.ratio,
        Direction::LowerIsBetter,
    );
    push_scalar(
        &mut out,
        "hotspot_concentration",
        baseline.metrics.hotspot_concentration.ratio,
        current.metrics.hotspot_concentration.ratio,
        Direction::LowerIsBetter,
    );
    push_scalar(
        &mut out,
        "tangle_index",
        baseline.metrics.tangle_index.ratio,
        current.metrics.tangle_index.ratio,
        Direction::LowerIsBetter,
    );

    if let (Some(b), Some(c)) = (&baseline.metrics.complexity, &current.metrics.complexity) {
        push_scalar(
            &mut out,
            "avg_cyclomatic",
            b.avg_cyclomatic,
            c.avg_cyclomatic,
            Direction::LowerIsBetter,
        );
    }
    if let (Some(b), Some(c)) = (&baseline.metrics.cohesion, &current.metrics.cohesion) {
        push_scalar(
            &mut out,
            "avg_cohesion",
            b.avg_cohesion,
            c.avg_cohesion,
            Direction::HigherIsBetter,
        );
    }

    out
}

enum Direction {
    LowerIsBetter,
    HigherIsBetter,
}

fn push_scalar(
    out: &mut BTreeMap<String, MetricDelta>,
    name: &str,
    baseline: f64,
    current: f64,
    dir: Direction,
) {
    let delta = current - baseline;
    let pct = if baseline.abs() > f64::EPSILON {
        (delta / baseline) * 100.0
    } else {
        0.0
    };
    let direction = classify(delta, &dir);
    let direction_word = match direction {
        TrendDirection::Improved => "improved",
        TrendDirection::Worsened => "worsened",
        TrendDirection::Stable => "is stable",
    };
    let description = format!(
        "{} {} from {:.3} to {:.3} ({:+.1}%)",
        name, direction_word, baseline, current, pct
    );
    out.insert(
        name.to_string(),
        MetricDelta {
            metric: name.to_string(),
            baseline_value: baseline,
            current_value: current,
            delta,
            direction,
            description,
        },
    );
}

fn classify(delta: f64, dir: &Direction) -> TrendDirection {
    if delta.abs() < 1e-6 {
        return TrendDirection::Stable;
    }
    match dir {
        Direction::LowerIsBetter => {
            if delta < 0.0 {
                TrendDirection::Improved
            } else {
                TrendDirection::Worsened
            }
        }
        Direction::HigherIsBetter => {
            if delta > 0.0 {
                TrendDirection::Improved
            } else {
                TrendDirection::Worsened
            }
        }
    }
}

fn module_deltas(baseline: &HealthReport, current: &HealthReport) -> Vec<ModuleDelta> {
    let mut out: Vec<ModuleDelta> = Vec::new();

    for (name, cur) in &current.module_annotations {
        let baseline_mod: Option<&ModuleAnnotation> = baseline.module_annotations.get(name);
        let (baseline_coupling, baseline_violations, baseline_cycle) = baseline_mod
            .map(|m| {
                (
                    m.coupling_score,
                    m.layer_violation_count,
                    cycle_member_in(baseline, name),
                )
            })
            .unwrap_or((0.0, 0, false));

        let coupling_delta = cur.coupling_score - baseline_coupling;
        let new_violations = cur
            .layer_violation_count
            .saturating_sub(baseline_violations);
        let new_cycle_membership = !baseline_cycle && cycle_member_in(current, name);

        // Filter down to modules that actually got worse on at least one axis.
        let is_worsening = coupling_delta > 0.05 || new_violations > 0 || new_cycle_membership;

        if is_worsening {
            out.push(ModuleDelta {
                module: name.clone(),
                coupling_delta,
                new_violations,
                new_cycle_membership,
            });
        }
    }

    out.sort_by(|a, b| {
        b.coupling_delta
            .partial_cmp(&a.coupling_delta)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.module.cmp(&b.module))
    });
    out
}

fn cycle_member_in(report: &HealthReport, module_name: &str) -> bool {
    report.findings.iter().any(|f| {
        matches!(f.finding_type, FindingType::CircularDependency)
            && f.node_ids.iter().any(|n| n.starts_with(module_name))
    })
}
