//! Rebuild `HealthReport` from per-segment cache rows (S2-γ).

use std::collections::{BTreeSet, HashSet};

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::super::report::{Finding, FindingType, HealthReport};
use super::cache::read_segment_cache;
use super::segments::{all_segment_ids, AnalysisSegment};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentMetricsSnapshot {
    pub segment_id: String,
    pub cycle_count: Option<usize>,
    pub finding_count: Option<usize>,
    pub health_score: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPrepPayload {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CyclesPayload {
    pub cycle_count: usize,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanMetricsPayload {
    pub sum_hotspot_fan_in: usize,
    pub total_fan_in: usize,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleCouplingPayload {
    pub avg_coupling: f64,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodePayload {
    pub dead_functions: usize,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadiusPayload {
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityPayload {
    pub avg_cyclomatic: f64,
    pub avg_cognitive: f64,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuxiliaryMetricsPayload {
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingsAssemblyPayload {
    pub findings: Vec<Finding>,
}

/// Full report payload stored under the HealthScore segment when a run completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthScoreSegmentPayload {
    pub report: HealthReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "segment", content = "payload")]
pub enum SegmentPayload {
    GraphPrep(GraphPrepPayload),
    Cycles(CyclesPayload),
    FanMetrics(FanMetricsPayload),
    ModuleCoupling(ModuleCouplingPayload),
    DeadCode(DeadCodePayload),
    BlastRadius(BlastRadiusPayload),
    Complexity(ComplexityPayload),
    AuxiliaryMetrics(AuxiliaryMetricsPayload),
    FindingsAssembly(FindingsAssemblyPayload),
    // Boxed: HealthScoreSegmentPayload wraps a full HealthReport, far larger
    // than the other variants. Boxing keeps SegmentPayload small to move/cache.
    // Box<T> is serde-transparent, so the cache wire format is unchanged.
    HealthScore(Box<HealthScoreSegmentPayload>),
}

pub fn extract_segment_snapshots(report: &HealthReport) -> Vec<SegmentMetricsSnapshot> {
    let cycle_count = Some(report.metrics.cycles.count);
    let finding_count = Some(report.findings.len());
    let health_score = report.health_score.map(|s| s as u8);

    all_segment_ids()
        .into_iter()
        .map(|seg| SegmentMetricsSnapshot {
            segment_id: seg.as_str().to_string(),
            cycle_count: if seg == AnalysisSegment::Cycles {
                cycle_count
            } else {
                None
            },
            finding_count: if seg == AnalysisSegment::FindingsAssembly {
                finding_count
            } else {
                None
            },
            health_score: if seg == AnalysisSegment::HealthScore {
                health_score
            } else {
                None
            },
        })
        .collect()
}

fn findings_of_type(report: &HealthReport, finding_type: FindingType) -> Vec<Finding> {
    report
        .findings
        .iter()
        .filter(|f| f.finding_type == finding_type)
        .cloned()
        .collect()
}

fn findings_of_types(report: &HealthReport, types: &[FindingType]) -> Vec<Finding> {
    report
        .findings
        .iter()
        .filter(|f| types.contains(&f.finding_type))
        .cloned()
        .collect()
}

fn fan_metrics_payload(report: &HealthReport, fan_findings: Vec<Finding>) -> FanMetricsPayload {
    let hc = &report.metrics.hotspot_concentration;
    // Prefer metrics.hotspot_concentration when HealthScore populated it; count tracks
    // hotspot functions. True sum_hotspot_fan_in is not persisted on HealthReport —
    // summary.hotspot_count is the same hotspot-function proxy used by health_score inputs.
    let sum_hotspot_fan_in = if hc.count > 0 {
        hc.count
    } else {
        report.summary.hotspot_count
    };
    let total_fan_in = hc.total.max(report.summary.total_functions).max(1);
    FanMetricsPayload {
        sum_hotspot_fan_in,
        total_fan_in,
        findings: fan_findings,
    }
}

pub fn extract_typed_segment_payloads(
    report: &HealthReport,
) -> Vec<(AnalysisSegment, SegmentPayload)> {
    let cycle_findings = findings_of_type(report, FindingType::CircularDependency);
    let fan_findings = findings_of_type(report, FindingType::BlastRadiusHotspot);
    let coupling_findings = findings_of_type(report, FindingType::HighCoupling);
    let dead_findings = findings_of_types(
        report,
        &[FindingType::PotentiallyUnreachable, FindingType::EntryPoint],
    );
    let blast_findings = findings_of_types(
        report,
        &[
            FindingType::BlastRadiusHotspot,
            FindingType::DeepCallChain,
            FindingType::LayerViolation,
        ],
    );
    let complexity_findings = findings_of_types(
        report,
        &[
            FindingType::ExcessiveComplexity,
            FindingType::GodFunction,
            FindingType::LowCohesion,
        ],
    );
    let auxiliary_findings = findings_of_types(
        report,
        &[
            FindingType::ZoneOfPain,
            FindingType::ZoneOfUselessness,
            FindingType::InformationFlowBottleneck,
            FindingType::HubNode,
            FindingType::LowEncapsulation,
            FindingType::TemporalCoupling,
        ],
    );
    let assembly_findings = findings_of_type(report, FindingType::ResolutionDegraded);

    let mut out = vec![
        (
            AnalysisSegment::GraphPrep,
            SegmentPayload::GraphPrep(GraphPrepPayload {}),
        ),
        (
            AnalysisSegment::Cycles,
            SegmentPayload::Cycles(CyclesPayload {
                cycle_count: report.metrics.cycles.count,
                findings: cycle_findings,
            }),
        ),
        (
            AnalysisSegment::FanMetrics,
            SegmentPayload::FanMetrics(fan_metrics_payload(report, fan_findings)),
        ),
        (
            AnalysisSegment::ModuleCoupling,
            SegmentPayload::ModuleCoupling(ModuleCouplingPayload {
                avg_coupling: report.metrics.coupling.avg_coupling,
                findings: coupling_findings,
            }),
        ),
        (
            AnalysisSegment::DeadCode,
            SegmentPayload::DeadCode(DeadCodePayload {
                dead_functions: report.summary.dead_functions,
                findings: dead_findings,
            }),
        ),
        (
            AnalysisSegment::BlastRadius,
            SegmentPayload::BlastRadius(BlastRadiusPayload {
                findings: blast_findings,
            }),
        ),
        (
            AnalysisSegment::Complexity,
            SegmentPayload::Complexity(ComplexityPayload {
                avg_cyclomatic: report
                    .metrics
                    .complexity
                    .as_ref()
                    .map(|c| c.avg_cyclomatic)
                    .unwrap_or(0.0),
                avg_cognitive: report
                    .metrics
                    .complexity
                    .as_ref()
                    .map(|c| c.avg_cognitive)
                    .unwrap_or(0.0),
                findings: complexity_findings,
            }),
        ),
        (
            AnalysisSegment::AuxiliaryMetrics,
            SegmentPayload::AuxiliaryMetrics(AuxiliaryMetricsPayload {
                findings: auxiliary_findings,
            }),
        ),
        (
            AnalysisSegment::FindingsAssembly,
            SegmentPayload::FindingsAssembly(FindingsAssemblyPayload {
                findings: assembly_findings,
            }),
        ),
        (
            AnalysisSegment::HealthScore,
            SegmentPayload::HealthScore(Box::new(HealthScoreSegmentPayload {
                report: report.clone(),
            })),
        ),
    ];
    out.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    out
}

pub fn read_segment_payload(
    conn: &Connection,
    segment: AnalysisSegment,
    fp: &str,
) -> Result<Option<SegmentPayload>> {
    let Some(raw) = read_segment_cache(conn, segment.as_str(), fp)? else {
        return Ok(None);
    };
    if segment == AnalysisSegment::HealthScore {
        let payload: HealthScoreSegmentPayload =
            serde_json::from_str(&raw).context("deserialize HealthScore segment cache")?;
        return Ok(Some(SegmentPayload::HealthScore(Box::new(payload))));
    }
    let payload: SegmentPayload =
        serde_json::from_str(&raw).context("deserialize segment cache payload")?;
    Ok(Some(payload))
}

pub fn read_health_score_cache(
    conn: &Connection,
    structure_fingerprint: &str,
) -> Result<Option<HealthReport>> {
    let Some(raw) = read_segment_cache(
        conn,
        AnalysisSegment::HealthScore.as_str(),
        structure_fingerprint,
    )?
    else {
        return Ok(None);
    };
    let payload: HealthScoreSegmentPayload =
        serde_json::from_str(&raw).context("deserialize HealthScore segment cache")?;
    Ok(Some(payload.report))
}

pub fn finding_types_for_segment(segment: AnalysisSegment) -> &'static [FindingType] {
    use AnalysisSegment::*;
    use FindingType::*;
    match segment {
        Cycles => &[CircularDependency],
        FanMetrics => &[BlastRadiusHotspot],
        ModuleCoupling => &[HighCoupling],
        DeadCode => &[PotentiallyUnreachable, EntryPoint],
        BlastRadius => &[BlastRadiusHotspot, DeepCallChain, LayerViolation],
        Complexity => &[ExcessiveComplexity, GodFunction, LowCohesion],
        AuxiliaryMetrics => &[
            ZoneOfPain,
            ZoneOfUselessness,
            InformationFlowBottleneck,
            HubNode,
            LowEncapsulation,
            TemporalCoupling,
        ],
        FindingsAssembly => &[ResolutionDegraded],
        GraphPrep | HealthScore => &[],
    }
}

/// L2 partial merge: wiring segments come from `partial`; reused segments keep prior findings.
/// Health score and score components always come from `partial` (HealthScore segment rerun).
pub fn merge_l2_report(
    prior: &HealthReport,
    partial: HealthReport,
    reused_segments: &HashSet<AnalysisSegment>,
) -> HealthReport {
    if reused_segments.is_empty() {
        return partial;
    }

    let mut reuse_types: BTreeSet<FindingType> = BTreeSet::new();
    for seg in reused_segments {
        for ft in finding_types_for_segment(*seg) {
            reuse_types.insert(*ft);
        }
    }

    let mut merged = partial;
    merged
        .findings
        .retain(|f| !reuse_types.contains(&f.finding_type));
    for f in &prior.findings {
        if reuse_types.contains(&f.finding_type) {
            merged.findings.push(f.clone());
        }
    }
    merged.findings.sort_by(|a, b| a.id.cmp(&b.id));

    if reused_segments.contains(&AnalysisSegment::AuxiliaryMetrics) {
        merged.metrics.cohesion = prior.metrics.cohesion.clone();
        merged.metrics.distance_from_main_sequence =
            prior.metrics.distance_from_main_sequence.clone();
        merged.metrics.temporal_coupling = prior.metrics.temporal_coupling.clone();
    }

    merged
}

pub fn merge_report(report: HealthReport) -> HealthReport {
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::config::AnalysisConfig;
    use crate::health::report::Severity;

    fn minimal_report(db_path: &str) -> HealthReport {
        let weights = AnalysisConfig::default().score_weights;
        crate::health::empty_report(db_path, 0, &weights)
    }

    fn finding(id: &str, finding_type: FindingType) -> Finding {
        Finding {
            id: id.into(),
            finding_type,
            severity: Severity::Warning,
            description: id.into(),
            detail: None,
            node_ids: vec![],
            edge_ids: None,
            primary_node_id: None,
            metric_name: None,
            metric_value: None,
            impact: None,
            blast_radius: None,
            recommendation: None,
            confidence: None,
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
        }
    }

    #[test]
    fn merge_l2_report_keeps_prior_auxiliary_findings_and_partial_health_score() {
        let mut prior = minimal_report("/tmp/prior.db");
        prior.findings.push(finding("aux-1", FindingType::HubNode));
        prior
            .findings
            .push(finding("wire-1", FindingType::CircularDependency));
        prior.health_score = Some(42);

        let mut partial = minimal_report("/tmp/partial.db");
        partial
            .findings
            .push(finding("wire-2", FindingType::HighCoupling));
        partial.health_score = Some(77);

        let reused = HashSet::from([AnalysisSegment::AuxiliaryMetrics]);
        let merged = merge_l2_report(&prior, partial, &reused);

        assert_eq!(merged.health_score, Some(77));
        assert!(merged.findings.iter().any(|f| f.id == "aux-1"));
        assert!(merged.findings.iter().any(|f| f.id == "wire-2"));
        assert!(!merged.findings.iter().any(|f| f.id == "wire-1"));
    }

    #[test]
    fn fan_metrics_payload_prefers_metrics_hotspot_count() {
        let mut report = minimal_report("/tmp/fan.db");
        report.summary.hotspot_count = 3;
        report.metrics.hotspot_concentration.count = 7;
        report.metrics.hotspot_concentration.total = 100;
        let payload = fan_metrics_payload(&report, vec![]);
        assert_eq!(payload.sum_hotspot_fan_in, 7);
        assert_eq!(payload.total_fan_in, 100);
    }
}
