//! Fix-First composite priority score.
//!
//! Implements `docs/02-strategy/DIAGNOSTIC_PRODUCT_SPEC.md` §1 exactly.
//!
//! ```text
//! priority_score = severity_weight
//!               × blast_radius_factor
//!               × temporal_factor
//!               × confidence_multiplier
//!               × change_proximity_bonus
//! ```
//!
//! All inputs come from fields that the analysis crate already populates on
//! [`Finding`] and [`ModuleAnnotation`]. No new engine work required.

use std::collections::BTreeSet;

use std::collections::BTreeMap;

use graphengine_analysis::health::report::{
    Confidence, Finding, FindingType, HealthReport, ModuleAnnotation, NodeAnnotation, Severity,
};
use serde::{Deserialize, Serialize};

use crate::narratives;

/// Default depth of the returned Fix-First list. Ten keeps the action surface
/// small enough to act on in a single engineering sprint.
pub const DEFAULT_TOP_N: usize = 10;

/// A single Fix-First entry, ready to render in Layer 3 of the report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PriorityItem {
    pub rank: usize,
    pub finding_id: String,
    pub finding_type: FindingType,
    pub target: String,
    pub priority_score: f64,
    pub risk_narrative: String,
    pub suggested_action: String,
    pub confidence: Option<Confidence>,
}

/// Compute a priority score for a single finding. Deterministic.
///
/// Multiplicative composition: any factor at `1.0` is a no-op; anything lower
/// than `1.0` (low confidence) dampens, anything higher (blast-radius log,
/// temporal bonus, proximity bonus) lifts. Saturation is handled by the
/// log-scale on blast radius, preventing single-function outliers from
/// dominating the ranking.
pub fn score_finding(finding: &Finding, worsening_modules: &BTreeSet<String>) -> f64 {
    let severity_weight = match finding.severity {
        Severity::Critical => 4.0,
        Severity::High => 3.0,
        Severity::Warning => 2.0,
        Severity::Info => 1.0,
    };

    let blast = finding.blast_radius.unwrap_or(0) as f64;
    let blast_radius_factor = 1.0 + (blast + 1.0).log2();

    let temporal_factor = match finding.finding_type {
        FindingType::TemporalCoupling => 1.5,
        _ => 1.0,
    };

    let confidence_multiplier = match finding.confidence {
        Some(Confidence::High) | None => 1.0,
        Some(Confidence::Medium) => 0.7,
        Some(Confidence::Low) => 0.4,
    };

    let change_proximity_bonus = if finding_touches_worsening_module(finding, worsening_modules) {
        1.3
    } else {
        1.0
    };

    severity_weight
        * blast_radius_factor
        * temporal_factor
        * confidence_multiplier
        * change_proximity_bonus
}

fn finding_touches_worsening_module(
    finding: &Finding,
    worsening_modules: &BTreeSet<String>,
) -> bool {
    if worsening_modules.is_empty() {
        return false;
    }
    finding
        .node_ids
        .iter()
        .any(|n| worsening_modules.iter().any(|m| n.starts_with(m)))
}

/// Identify modules that deserve a change-proximity bonus.
///
/// A module is "worsening" per the spec if it has any layer violations or
/// participates in a cycle. Both fields are already computed by the engine.
pub fn worsening_modules(report: &HealthReport) -> BTreeSet<String> {
    report
        .module_annotations
        .iter()
        .filter(|(_, m)| module_is_worsening(m))
        .map(|(k, _)| k.clone())
        .collect()
}

fn module_is_worsening(m: &ModuleAnnotation) -> bool {
    m.layer_violation_count > 0
        || matches!(
            m.risk_level,
            graphengine_analysis::health::report::RiskLevel::Critical
                | graphengine_analysis::health::report::RiskLevel::High
        )
}

/// Resolve the human-visible `target` label for a finding.
///
/// Order of preference:
/// 1. `primary_node_id` resolved through `node_annotations` → display name,
/// 2. first `node_ids` resolved through `node_annotations` → display name,
/// 3. `primary_node_id` / `node_ids[0]` verbatim (engines that use FQNs as
///    ids — modules typically do),
/// 4. `metric_name`,
/// 5. literal `"unknown"`.
///
/// This is the canonical way to derive the text the UI shows in the Top Risks
/// card heading and the Fix-First priority list. Callers must not duplicate
/// the fallback logic.
pub fn finding_target(finding: &Finding, annotations: &BTreeMap<String, NodeAnnotation>) -> String {
    if let Some(id) = finding
        .primary_node_id
        .as_deref()
        .or_else(|| finding.node_ids.first().map(|s| s.as_str()))
    {
        let label = narratives::display_name_for(id, annotations);
        if !label.is_empty() {
            return label;
        }
    }
    if let Some(m) = finding.metric_name.clone() {
        return m;
    }
    "unknown".to_string()
}

/// Enrich every finding in the report with `priority_score` (stored in
/// `metric_value` is NOT done — the spec calls for dedicated fields, but
/// those fields don't exist on the canonical `Finding` struct today).
///
/// Until the analysis crate is extended with `priority_score` / `priority_rank`
/// fields (a future small PR), the scores are carried out-of-band through the
/// returned `Vec<(finding_id, score)>`. This keeps the diagnostic crate
/// strictly additive to the analysis crate contract.
pub fn enrich_findings(report: &HealthReport) -> Vec<(String, f64)> {
    let worsening = worsening_modules(report);
    report
        .findings
        .iter()
        .map(|f| (f.id.clone(), score_finding(f, &worsening)))
        .collect()
}

/// Compute the top-N Fix-First priority list.
///
/// Deterministic: ties broken by (severity rank desc, finding_id lex asc) so
/// snapshot tests can pin output byte-for-byte.
pub fn compute_priorities(report: &HealthReport, top_n: usize) -> Vec<PriorityItem> {
    let worsening = worsening_modules(report);

    let mut scored: Vec<(f64, &Finding)> = report
        .findings
        .iter()
        .map(|f| (score_finding(f, &worsening), f))
        .collect();

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.severity.rank().cmp(&a.1.severity.rank()))
            .then_with(|| a.1.id.cmp(&b.1.id))
    });

    scored
        .into_iter()
        .take(top_n)
        .enumerate()
        .map(|(i, (score, finding))| PriorityItem {
            rank: i + 1,
            finding_id: finding.id.clone(),
            finding_type: finding.finding_type,
            target: finding_target(finding, &report.node_annotations),
            priority_score: score,
            risk_narrative: narratives::risk_narrative(finding, &report.node_annotations),
            suggested_action: narratives::suggested_action(finding, &report.node_annotations),
            confidence: finding.confidence,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphengine_analysis::health::report::*;
    use std::collections::BTreeMap;

    fn f(id: &str, sev: Severity, blast: usize, conf: Option<Confidence>) -> Finding {
        Finding {
            id: id.to_string(),
            finding_type: FindingType::BlastRadiusHotspot,
            severity: sev,
            description: "test".into(),
            detail: None,
            node_ids: vec!["mod::fn".into()],
            edge_ids: None,
            primary_node_id: Some("mod::fn".into()),
            metric_name: None,
            metric_value: None,
            impact: None,
            blast_radius: Some(blast),
            recommendation: None,
            confidence: conf,
            cycle_length: None,
            fan_in: Some(5),
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
        }
    }

    fn empty_report(findings: Vec<Finding>) -> HealthReport {
        HealthReport {
            version: "1".into(),
            generated_at: "t".into(),
            analysis_duration_ms: 0,
            db_path: "".into(),
            health_score: Some(70),
            health_score_components: HealthScoreComponents {
                cycle_severity: ScoreComponent {
                    score: 80,
                    weight: 0.1,
                },
                coupling_health: ScoreComponent {
                    score: 80,
                    weight: 0.1,
                },
                hotspot_concentration: ScoreComponent {
                    score: 80,
                    weight: 0.1,
                },
                dead_code_ratio: ScoreComponent {
                    score: 80,
                    weight: 0.1,
                },
                depth_complexity: ScoreComponent {
                    score: 80,
                    weight: 0.1,
                },
                complexity: ScoreComponent {
                    score: 80,
                    weight: 0.1,
                },
                cohesion: ScoreComponent {
                    score: 80,
                    weight: 0.1,
                },
                distance: ScoreComponent {
                    score: 80,
                    weight: 0.1,
                },
                temporal_coupling: ScoreComponent {
                    score: 80,
                    weight: 0.2,
                },
            },
            metrics: MetricsReport {
                cycles: MetricDetail {
                    count: 0,
                    total: 0,
                    ratio: 0.0,
                    description: "".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                coupling: CouplingMetricDetail {
                    modules_measured: 0,
                    modules_above_070: 0,
                    modules_above_050: 0,
                    avg_coupling: 0.0,
                    description: "".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                hotspot_concentration: MetricDetail {
                    count: 0,
                    total: 0,
                    ratio: 0.0,
                    description: "".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                dead_code: DeadCodeMetricDetail {
                    count: 0,
                    total: 0,
                    ratio: 0.0,
                    description: "".into(),
                    status: MetricStatus::Ok,
                    reason_breakdown: BTreeMap::new(),
                    reason_breakdown_caveats: None,
                    fidelity: None,
                    no_callers_total: None,
                    no_callers_high_confidence: None,
                },
                depth: DepthMetricDetail {
                    max_call_depth: 0,
                    description: "".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                tangle_index: MetricDetail {
                    count: 0,
                    total: 0,
                    ratio: 0.0,
                    description: "".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                complexity: None,
                cohesion: None,
                distance_from_main_sequence: None,
                temporal_coupling: None,
                metric_confidence: None,
            },
            percentiles: None,
            summary: Summary {
                total_nodes: 0,
                total_edges: 0,
                total_functions: 0,
                total_modules: 0,
                cycles_found: 0,
                cycle_total_nodes: 0,
                hotspot_count: 0,
                hotspot_threshold_fan_in: 0,
                high_coupling_modules: 0,
                dead_functions: 0,
                max_call_depth: 0,
                tangle_index: 0.0,
                avg_module_coupling: 0.0,
                avg_fan_in: 0.0,
                avg_fan_out: 0.0,
            },
            findings,
            node_annotations: BTreeMap::new(),
            module_annotations: BTreeMap::new(),
            classifications: BTreeMap::new(),
            boundary_violations: vec![],
            resolution_quality: None,
            analysis_errors: vec![],
            integrity_status: IntegrityStatus::default(),
            git_signals: None,
            file_extraction_coverage: Vec::new(),
            primary_language: None,
            analysis_provenance: None,
        }
    }

    #[test]
    fn critical_outranks_warning_at_same_blast_radius() {
        let r = empty_report(vec![
            f("a", Severity::Warning, 10, Some(Confidence::High)),
            f("b", Severity::Critical, 10, Some(Confidence::High)),
        ]);
        let top = compute_priorities(&r, 10);
        assert_eq!(top[0].finding_id, "b");
    }

    #[test]
    fn low_confidence_is_dampened() {
        let r = empty_report(vec![
            f("a", Severity::Critical, 10, Some(Confidence::Low)),
            f("b", Severity::High, 10, Some(Confidence::High)),
        ]);
        let top = compute_priorities(&r, 10);
        // Critical-but-low (4 * log_factor * 0.4) vs High-and-high (3 * log_factor * 1.0)
        // -> high-and-high wins because 1.6 < 3.0 after factoring.
        assert_eq!(top[0].finding_id, "b");
    }

    #[test]
    fn deterministic_tie_breaking() {
        let r = empty_report(vec![
            f("zzz", Severity::High, 5, Some(Confidence::High)),
            f("aaa", Severity::High, 5, Some(Confidence::High)),
        ]);
        let top = compute_priorities(&r, 10);
        assert_eq!(top[0].finding_id, "aaa");
        assert_eq!(top[1].finding_id, "zzz");
    }
}
