//! `ScanReportView` — the format-agnostic projection of a persisted
//! scan into the shape the spec's hero report describes.
//!
//! Every numeric value rendered to the user is built once, here, from
//! the authoritative `HealthReport` + `ProjectDto` + `ScanRunDto`.
//! Renderers (`table`, `markdown`, `llm`, `json`) consume the view
//! and never re-interpret the underlying domain — that keeps the
//! "Score: 62/100" the user sees in their terminal byte-identical to
//! the "Score: 62/100" they paste into a GitHub issue via markdown.
//!
//! Design choices worth keeping when this module grows:
//! - Every text label here is the *display* string. Renderers may
//!   truncate or right-pad but MUST NOT re-derive labels from raw
//!   data — that is what lets stage 3's snapshot tests catch drift.
//! - Status strings (`"Risk"`, `"Moderate"`, …) come from
//!   [`metric_status_label`] which is the single source of truth for
//!   per-metric severity wording. Adding a metric means adding one
//!   line here and one line in the renderer; nothing else.
//! - `MetricStatus::InsufficientEdges` / `FrameworkInvisible` /
//!   `NotApplicable` / `ComputationFailed` are surfaced as their own
//!   status string so the user is never tricked into thinking
//!   "0 hotspots" means "no hotspots" when it actually means "we
//!   could not measure".

use std::collections::BTreeSet;

use graphengine_analysis::health::report::{Confidence, HealthReport, MetricStatus, Severity};
use graphengine_diagnostic::priority::{self, PriorityItem};
use gridseak_local_store::{ProjectDto, ScanRunDto};
use serde::Serialize;

use super::tier_signaling::TierSignal;

/// Default cap on hero priorities. Mirrors `priority::DEFAULT_TOP_N`
/// but capped lower for the hero report so the first-run output stays
/// readable; recommendations command surfaces the full list.
pub const HERO_PRIORITY_LIMIT: usize = 3;

/// Top-level view model rendered by every output format.
#[derive(Debug, Clone, Serialize)]
pub struct ScanReportView {
    pub repo_name: String,
    pub root_path: String,
    pub scan_id: String,
    pub scanned_at_display: String,
    pub branch: Option<String>,
    pub commit_short: Option<String>,
    pub dirty: Option<bool>,
    pub languages: Vec<String>,
    pub score: Option<u32>,
    pub score_band: ScoreBand,
    pub confidence_notes: Vec<ConfidenceNote>,
    pub priorities: Vec<PriorityRow>,
    pub metrics: Vec<MetricRow>,
    pub next_commands: Vec<String>,
    pub schema_caveats: Vec<String>,
    /// Free vs would-be-paid signaling block. Same content surfaces
    /// in `gridseak context --for-llm`; renderers append it as a
    /// footer so the report itself teaches the user what is included
    /// today and what would be paid later.
    pub tier_signal: TierSignal,
}

/// Coarse qualitative bucket for the composite health score.
///
/// Used by renderers to colour or label the headline number. The
/// thresholds are deliberately picked so the spec's example
/// (`Score: 62/100 → Moderate risk`) lands in `Moderate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreBand {
    Excellent,
    Good,
    Moderate,
    Risk,
    Critical,
    Unknown,
}

impl ScoreBand {
    pub fn from_score(score: Option<u32>) -> Self {
        match score {
            None => Self::Unknown,
            Some(s) if s >= 85 => Self::Excellent,
            Some(s) if s >= 70 => Self::Good,
            Some(s) if s >= 50 => Self::Moderate,
            Some(s) if s >= 30 => Self::Risk,
            Some(_) => Self::Critical,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Excellent => "Excellent",
            Self::Good => "Good",
            Self::Moderate => "Moderate risk",
            Self::Risk => "Elevated risk",
            Self::Critical => "Critical risk",
            Self::Unknown => "Unknown",
        }
    }
}

/// One row in the per-metric confidence summary printed under the
/// hero block. We aggregate metrics by their `Confidence` level
/// rather than emitting one note per metric — three lines reading
/// "high for coupling, depth, blast radius; medium for dead code"
/// is the spec's literal output.
#[derive(Debug, Clone, Serialize)]
pub struct ConfidenceNote {
    /// `"high"`, `"medium"`, `"low"` — lowercase by spec.
    pub level: String,
    /// Display names of the metrics that fell in this confidence band.
    pub metrics: Vec<String>,
}

/// One row in the Top Priorities table.
#[derive(Debug, Clone, Serialize)]
pub struct PriorityRow {
    pub rank: usize,
    pub severity: String,
    pub finding_id: String,
    pub finding: String,
    pub target: String,
    pub evidence: String,
    pub confidence: Option<String>,
}

/// One row in the Metrics table.
///
/// `confidence` carries the analyzer's `metric_confidence[key].level`
/// when the metric is low-confidence. Renderers may use this to mark
/// the row in a per-format way (the table prepends `Low-confidence · `
/// onto the status string); machine consumers can read the field
/// directly via JSON.
///
/// Additive by design: `skip_serializing_if = "Option::is_none"`
/// keeps every existing `hero_json.snap` byte-stable for rows that
/// are not low-confidence.
#[derive(Debug, Clone, Serialize)]
pub struct MetricRow {
    pub name: String,
    pub value: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
}

impl ScanReportView {
    /// Build a view from the canonical inputs. This is the *only*
    /// place renderers should look at the raw `HealthReport` — every
    /// downstream renderer takes a `&ScanReportView`.
    pub fn build(report: &HealthReport, project: &ProjectDto, scan: &ScanRunDto) -> Self {
        let repo_name = project.display_name.clone();
        let root_path = project
            .roots
            .first()
            .map(|r| r.path.clone())
            .unwrap_or_default();

        let scanned_at_display = format_started_at(&scan.started_at);
        let branch = scan.git_branch.clone();
        let commit_short = scan
            .git_commit
            .as_deref()
            .map(short_commit)
            .map(str::to_string);

        let languages = if scan.scan_languages.is_empty() {
            scan.primary_language.clone().into_iter().collect()
        } else {
            scan.scan_languages.clone()
        };

        let score = report.health_score;
        let score_band = ScoreBand::from_score(score);

        let confidence_notes = build_confidence_notes(report);
        let priorities = build_priorities(report);
        let metrics = build_metrics(report);
        let next_commands = build_next_commands(&priorities, project.scan_count);
        let schema_caveats = report.integrity_status.schema_caveats.clone();

        Self {
            repo_name,
            root_path,
            scan_id: scan.id.clone(),
            scanned_at_display,
            branch,
            commit_short,
            dirty: scan.git_dirty,
            languages,
            score,
            score_band,
            confidence_notes,
            priorities,
            metrics,
            next_commands,
            schema_caveats,
            tier_signal: TierSignal::default_v0(),
        }
    }
}

// ---------------------------------------------------------------------------
// Confidence notes
// ---------------------------------------------------------------------------

fn build_confidence_notes(report: &HealthReport) -> Vec<ConfidenceNote> {
    let Some(per_metric) = report.metrics.metric_confidence.as_ref() else {
        return Vec::new();
    };

    let mut high: Vec<String> = Vec::new();
    let mut medium: Vec<String> = Vec::new();
    let mut low: Vec<String> = Vec::new();
    for (metric_key, confidence) in per_metric {
        let label = humanise_metric_key(metric_key);
        match confidence.level {
            Confidence::High => high.push(label),
            Confidence::Medium => medium.push(label),
            Confidence::Low => low.push(label),
        }
    }
    for bucket in [&mut high, &mut medium, &mut low] {
        bucket.sort();
    }

    let mut notes = Vec::new();
    if !high.is_empty() {
        notes.push(ConfidenceNote {
            level: "high".into(),
            metrics: high,
        });
    }
    if !medium.is_empty() {
        notes.push(ConfidenceNote {
            level: "medium".into(),
            metrics: medium,
        });
    }
    if !low.is_empty() {
        notes.push(ConfidenceNote {
            level: "low".into(),
            metrics: low,
        });
    }
    notes
}

// ---------------------------------------------------------------------------
// Priority rows (Top Priorities table)
// ---------------------------------------------------------------------------

fn build_priorities(report: &HealthReport) -> Vec<PriorityRow> {
    let items = priority::compute_priorities(report, HERO_PRIORITY_LIMIT);

    let finding_lookup: std::collections::BTreeMap<
        &str,
        &graphengine_analysis::health::report::Finding,
    > = report.findings.iter().map(|f| (f.id.as_str(), f)).collect();

    items
        .into_iter()
        .map(|item| build_priority_row(&item, &finding_lookup))
        .collect()
}

fn build_priority_row(
    item: &PriorityItem,
    finding_lookup: &std::collections::BTreeMap<
        &str,
        &graphengine_analysis::health::report::Finding,
    >,
) -> PriorityRow {
    let finding = finding_lookup.get(item.finding_id.as_str()).copied();

    let severity = finding
        .map(|f| severity_label(f.severity).to_string())
        .unwrap_or_else(|| "—".to_string());

    let finding_label = finding
        .map(|f| finding_type_label(f.finding_type))
        .unwrap_or("Finding")
        .to_string();

    let evidence = finding
        .map(evidence_for_finding)
        .unwrap_or_else(|| item.risk_narrative.clone());

    let confidence = item.confidence.map(|c| confidence_label(c).to_string());

    PriorityRow {
        rank: item.rank,
        severity,
        finding_id: item.finding_id.clone(),
        finding: finding_label,
        target: item.target.clone(),
        evidence,
        confidence,
    }
}

pub(crate) fn evidence_for(finding: &graphengine_analysis::health::report::Finding) -> String {
    evidence_for_finding(finding)
}

fn evidence_for_finding(finding: &graphengine_analysis::health::report::Finding) -> String {
    use graphengine_analysis::health::report::FindingType;
    match finding.finding_type {
        FindingType::BlastRadiusHotspot => {
            let downstream = finding.blast_radius.unwrap_or(0);
            format!("{downstream} downstream nodes")
        }
        FindingType::CircularDependency => {
            let len = finding.cycle_length.unwrap_or(finding.node_ids.len());
            format!("cycle of {len} nodes")
        }
        FindingType::HighCoupling => match finding.coupling_score {
            Some(score) => format!("coupling {score:.2}"),
            None => "high coupling".into(),
        },
        FindingType::PotentiallyUnreachable => "no production callers".into(),
        FindingType::DeepCallChain => match finding.metric_value {
            Some(depth) => format!("depth {}", depth as u64),
            None => "deep call chain".into(),
        },
        FindingType::InformationFlowBottleneck => "bottleneck".into(),
        FindingType::HubNode => match finding.hub_score {
            Some(score) => format!("hub score {score:.2}"),
            None => "hub node".into(),
        },
        FindingType::LowEncapsulation => match (finding.internal_edges, finding.external_edges) {
            (Some(internal), Some(external)) => {
                format!("{external} external / {internal} internal")
            }
            _ => "low encapsulation".into(),
        },
        FindingType::LowCohesion => format!(
            "{} disconnected clusters",
            finding.count.unwrap_or(finding.node_ids.len().max(1))
        ),
        FindingType::ExcessiveComplexity => match finding.metric_value {
            Some(value) => format!("complexity {value:.1}"),
            None => "excessive complexity".into(),
        },
        FindingType::TemporalCoupling => match finding.temporal_coupling_score {
            Some(score) => format!("temporal {score:.2}"),
            None => "temporal coupling".into(),
        },
        FindingType::GodFunction => "god function".into(),
        FindingType::EntryPoint => "entry point".into(),
        FindingType::ZoneOfPain => "stable + concrete".into(),
        FindingType::ZoneOfUselessness => "abstract + unstable".into(),
        FindingType::BoundaryViolation => "boundary violation".into(),
        FindingType::LayerViolation => "layer violation".into(),
        FindingType::ResolutionDegraded => "resolution degraded".into(),
    }
}

fn severity_label(s: Severity) -> &'static str {
    severity_display(s)
}

pub(crate) fn severity_display(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "Critical",
        Severity::High => "High",
        Severity::Warning => "Warning",
        Severity::Info => "Info",
    }
}

fn confidence_label(c: Confidence) -> &'static str {
    confidence_display(c)
}

pub(crate) fn confidence_display(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

fn finding_type_label(t: graphengine_analysis::health::report::FindingType) -> &'static str {
    finding_type_display(t)
}

pub(crate) fn finding_type_display(
    t: graphengine_analysis::health::report::FindingType,
) -> &'static str {
    use graphengine_analysis::health::report::FindingType::*;
    match t {
        CircularDependency => "Circular dependency",
        BlastRadiusHotspot => "Blast radius hotspot",
        HighCoupling => "High coupling",
        PotentiallyUnreachable => "Dead code",
        DeepCallChain => "Deep call chain",
        InformationFlowBottleneck => "Bottleneck",
        HubNode => "Hub node",
        LowEncapsulation => "Low encapsulation",
        BoundaryViolation => "Boundary violation",
        ExcessiveComplexity => "Excessive complexity",
        TemporalCoupling => "Temporal coupling",
        LowCohesion => "Low cohesion",
        LayerViolation => "Layer violation",
        GodFunction => "God function",
        EntryPoint => "Entry point",
        ZoneOfPain => "Zone of pain",
        ZoneOfUselessness => "Zone of uselessness",
        ResolutionDegraded => "Resolution degraded",
    }
}

// ---------------------------------------------------------------------------
// Metric rows (Metrics table)
// ---------------------------------------------------------------------------

fn build_metrics(report: &HealthReport) -> Vec<MetricRow> {
    let mut rows = Vec::new();
    let metrics = &report.metrics;

    // Health score — hero row.
    rows.push(MetricRow {
        name: "Health score".into(),
        value: match report.health_score {
            Some(s) => format!("{s}/100"),
            None => "—".into(),
        },
        status: ScoreBand::from_score(report.health_score)
            .label()
            .to_string(),
        confidence: None,
    });

    rows.push(MetricRow {
        name: "Cycles".into(),
        value: metrics.cycles.count.to_string(),
        status: status_label_with_count(metrics.cycles.status, metrics.cycles.count),
        confidence: None,
    });

    rows.push(MetricRow {
        name: "Tangle index".into(),
        value: format_percent(metrics.tangle_index.ratio),
        status: status_label_with_threshold(
            metrics.tangle_index.status,
            metrics.tangle_index.ratio,
            0.02,
            0.05,
        ),
        confidence: None,
    });

    rows.push(MetricRow {
        name: "Hotspots".into(),
        value: metrics.hotspot_concentration.count.to_string(),
        status: status_label_with_count(
            metrics.hotspot_concentration.status,
            metrics.hotspot_concentration.count,
        ),
        confidence: None,
    });

    rows.push(MetricRow {
        name: "Dead functions".into(),
        value: metrics.dead_code.count.to_string(),
        status: dead_code_status_label(metrics.dead_code.status, metrics.dead_code.count),
        confidence: None,
    });

    rows.push(MetricRow {
        name: "Avg coupling".into(),
        value: format!("{:.2}", metrics.coupling.avg_coupling),
        status: coupling_status_label(metrics.coupling.status, metrics.coupling.avg_coupling),
        confidence: None,
    });

    rows.push(MetricRow {
        name: "Max call depth".into(),
        value: metrics.depth.max_call_depth.to_string(),
        status: depth_status_label(metrics.depth.status, metrics.depth.max_call_depth),
        confidence: None,
    });

    if let Some(cohesion) = metrics.cohesion.as_ref() {
        rows.push(MetricRow {
            name: "Avg cohesion".into(),
            value: format!("{:.2}", cohesion.avg_cohesion),
            status: cohesion_status_label(cohesion.avg_cohesion, cohesion.low_cohesion_modules),
            confidence: None,
        });
    }

    if let Some(complexity) = metrics.complexity.as_ref() {
        rows.push(MetricRow {
            name: "Max cyclomatic".into(),
            value: complexity.max_cyclomatic.to_string(),
            status: complexity_status_label(complexity.max_cyclomatic),
            confidence: None,
        });
    }

    apply_confidence_overrides(&mut rows, report);
    rows
}

/// Map a hero `MetricRow.name` back to the key the analyzer uses in
/// `HealthReport.metrics.metric_confidence`. This is the inverse of
/// [`humanise_metric_key`] for the subset of metrics that actually
/// have a confidence signal today.
///
/// Note `"Hotspots"` intentionally points at the `"dead_code"` key:
/// the analyzer ([graphengine-analysis/src/health/mod.rs] near the
/// `hotspot_concentration.status = dead_code_status_value` assignment)
/// computes the hotspot signal from the same edge-resolution pool as
/// dead code, so low confidence in dead code implies low confidence
/// in hotspots. If the analyzer ever publishes a `"hotspot_concentration"`
/// key directly, prefer that instead.
fn confidence_key_for_row(name: &str) -> Option<&'static str> {
    match name {
        "Dead functions" => Some("dead_code"),
        "Hotspots" => Some("dead_code"),
        "Avg coupling" => Some("coupling"),
        "Max call depth" => Some("depth"),
        _ => None,
    }
}

/// Walk `rows` and, for every row whose analyzer-side confidence is
/// `Low`, prepend `Low-confidence · ` to its status string and record
/// the level on `MetricRow.confidence`. We keep the numeric severity
/// (`Risk` / `Moderate` / `OK`) so the reader still sees both pieces
/// of information; the prefix makes the caveat impossible to miss.
fn apply_confidence_overrides(rows: &mut [MetricRow], report: &HealthReport) {
    use graphengine_analysis::health::report::Confidence;
    let Some(per_metric) = report.metrics.metric_confidence.as_ref() else {
        return;
    };
    for row in rows.iter_mut() {
        let Some(key) = confidence_key_for_row(&row.name) else {
            continue;
        };
        let Some(c) = per_metric.get(key) else {
            continue;
        };
        match c.level {
            Confidence::Low => {
                row.confidence = Some("low".into());
                row.status = format!("Low-confidence · {}", row.status);
            }
            Confidence::Medium => {
                row.confidence = Some("medium".into());
            }
            Confidence::High => {
                // No annotation: the metric is trustworthy at face value.
            }
        }
    }
}

fn status_label_with_count(status: MetricStatus, count: usize) -> String {
    if let Some(badge) = badge_for_non_ok_status(status) {
        return badge.into();
    }
    if count == 0 {
        "OK".into()
    } else {
        "Risk".into()
    }
}

fn status_label_with_threshold(
    status: MetricStatus,
    value: f64,
    moderate_at: f64,
    risk_at: f64,
) -> String {
    if let Some(badge) = badge_for_non_ok_status(status) {
        return badge.into();
    }
    if value >= risk_at {
        "Risk".into()
    } else if value >= moderate_at {
        "Moderate".into()
    } else {
        "OK".into()
    }
}

fn dead_code_status_label(status: MetricStatus, count: usize) -> String {
    if let Some(badge) = badge_for_non_ok_status(status) {
        return badge.into();
    }
    // Dead-code numbers are noisier than cycle/hotspot counts because
    // framework-invisible callers can inflate them. We treat any
    // non-zero count up to 50 as "Review" (worth a look but not
    // alarming) and only flip to "Risk" above that. Matches the
    // spec's example (18 → Review).
    if count == 0 {
        "OK".into()
    } else if count < 50 {
        "Review".into()
    } else {
        "Risk".into()
    }
}

fn coupling_status_label(status: MetricStatus, avg_coupling: f64) -> String {
    if let Some(badge) = badge_for_non_ok_status(status) {
        return badge.into();
    }
    if avg_coupling >= 0.6 {
        "Risk".into()
    } else if avg_coupling >= 0.3 {
        "Moderate".into()
    } else {
        "OK".into()
    }
}

fn depth_status_label(status: MetricStatus, max_depth: usize) -> String {
    if let Some(badge) = badge_for_non_ok_status(status) {
        return badge.into();
    }
    // Spec example: max_depth 14 → "Deep". Any call chain longer
    // than ~8 frames is hard to reason about so we surface "Deep"
    // there; "Moderate" covers the noisy middle band.
    if max_depth >= 8 {
        "Deep".into()
    } else if max_depth >= 4 {
        "Moderate".into()
    } else {
        "OK".into()
    }
}

fn cohesion_status_label(avg_cohesion: f64, low_cohesion_modules: usize) -> String {
    if avg_cohesion >= 0.7 {
        "OK".into()
    } else if low_cohesion_modules == 0 {
        "Moderate".into()
    } else {
        "Risk".into()
    }
}

fn complexity_status_label(max_cyclomatic: u32) -> String {
    if max_cyclomatic >= 25 {
        "Risk".into()
    } else if max_cyclomatic >= 15 {
        "Moderate".into()
    } else {
        "OK".into()
    }
}

/// Map a non-`Ok` `MetricStatus` to the user-facing badge it should
/// be rendered as. Returns `None` for `Ok` because a numeric status
/// derived from the metric value applies in that case.
fn badge_for_non_ok_status(status: MetricStatus) -> Option<&'static str> {
    match status {
        MetricStatus::Ok => None,
        MetricStatus::InsufficientEdges => Some("Not measured"),
        MetricStatus::FrameworkInvisible => Some("Framework-invisible"),
        MetricStatus::NotApplicable => Some("N/A"),
        MetricStatus::ComputationFailed => Some("Failed"),
    }
}

fn format_percent(ratio: f64) -> String {
    format!("{:.1}%", ratio * 100.0)
}

// ---------------------------------------------------------------------------
// Next commands
// ---------------------------------------------------------------------------

/// Build the "Next" block at the bottom of the hero report. Pull
/// from the top-ranked priority so the suggestions actually point at
/// something the report just surfaced; if no priorities, fall back
/// to safe generic next steps.
///
/// `scan_count` is the number of completed scans on this project
/// (including the one we just rendered). When `scan_count <= 1` we
/// suggest re-running the scan instead of `gridseak compare --previous`,
/// because the compare command refuses to run with fewer than two
/// scans ([gridseak-cli/src/history_command.rs::run_compare]).
fn build_next_commands(priorities: &[PriorityRow], scan_count: u32) -> Vec<String> {
    let mut commands = Vec::new();
    if let Some(top) = priorities.first() {
        let target = &top.target;
        commands.push(format!("gridseak explain {}", top.finding_id));
        commands.push(format!("gridseak graph callers {target}"));
        commands.push(format!("gridseak graph blast-radius {target}"));
    }
    if scan_count > 1 {
        commands.push("gridseak compare --previous".into());
    } else {
        // First scan: invite the user to run a second so `compare` has
        // something to diff against. Phrased as a concrete command so
        // it composes with the rest of the bullet list.
        commands.push("gridseak scan . (run again to build trend history)".into());
    }

    let mut seen = BTreeSet::new();
    commands.retain(|cmd| seen.insert(cmd.clone()));
    commands
}

// ---------------------------------------------------------------------------
// Display formatting helpers
// ---------------------------------------------------------------------------

fn format_started_at(started_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(started_at)
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| started_at.to_string())
}

fn short_commit(commit: &str) -> &str {
    if commit.len() > 7 {
        &commit[..7]
    } else {
        commit
    }
}

fn humanise_metric_key(key: &str) -> String {
    match key {
        "cycles" => "cycles".into(),
        "coupling" => "coupling".into(),
        "hotspot_concentration" | "hotspots" => "hotspots".into(),
        "dead_code" => "dead code".into(),
        "depth" => "depth".into(),
        "tangle_index" => "tangle index".into(),
        "complexity" => "complexity".into(),
        "cohesion" => "cohesion".into(),
        "distance_from_main_sequence" => "distance from main sequence".into(),
        "temporal_coupling" => "temporal coupling".into(),
        "blast_radius" => "blast radius".into(),
        other => other.replace('_', " "),
    }
}

// ---------------------------------------------------------------------------
// Shared test fixture
// ---------------------------------------------------------------------------
//
// Every renderer's snapshot test shares this view so a regression in
// any one renderer shows up as a contained, easy-to-diff snapshot
// failure rather than four tests drifting against four slightly-
// different inputs. Keep it in sync with the spec's "Example Output"
// at `docs/02-strategy/CLI_SHADOW_MODE_DISTRIBUTION_SPEC.md` (lines
// 146-178) so the snapshots double as living documentation.

#[cfg(test)]
pub(crate) fn fixture_spec_example() -> ScanReportView {
    ScanReportView {
        repo_name: "my-service".into(),
        root_path: "/repos/my-service".into(),
        scan_id: "scan_test".into(),
        scanned_at_display: "2026-05-17 11:12".into(),
        branch: Some("main".into()),
        commit_short: Some("9f3c2a1".into()),
        dirty: Some(false),
        languages: vec!["typescript".into(), "javascript".into()],
        score: Some(62),
        score_band: ScoreBand::Moderate,
        confidence_notes: vec![
            ConfidenceNote {
                level: "high".into(),
                metrics: vec!["coupling".into(), "depth".into(), "blast radius".into()],
            },
            ConfidenceNote {
                level: "medium".into(),
                metrics: vec!["dead code".into()],
            },
        ],
        priorities: vec![
            PriorityRow {
                rank: 1,
                severity: "Critical".into(),
                finding_id: "finding_001".into(),
                finding: "Blast radius hotspot".into(),
                target: "auth/createSession".into(),
                evidence: "41 downstream nodes".into(),
                confidence: Some("high".into()),
            },
            PriorityRow {
                rank: 2,
                severity: "High".into(),
                finding_id: "finding_002".into(),
                finding: "Low cohesion".into(),
                target: "src/services".into(),
                evidence: "9 disconnected clusters".into(),
                confidence: Some("medium".into()),
            },
            PriorityRow {
                rank: 3,
                severity: "High".into(),
                finding_id: "finding_003".into(),
                finding: "Dead code".into(),
                target: "billing/legacyApplyDiscount".into(),
                evidence: "no production callers".into(),
                confidence: Some("medium".into()),
            },
        ],
        metrics: vec![
            MetricRow {
                name: "Health score".into(),
                value: "62/100".into(),
                status: "Moderate risk".into(),
                confidence: None,
            },
            MetricRow {
                name: "Cycles".into(),
                value: "3".into(),
                status: "Risk".into(),
                confidence: None,
            },
            MetricRow {
                name: "Tangle index".into(),
                value: "4.2%".into(),
                status: "Risk".into(),
                confidence: None,
            },
            MetricRow {
                name: "Hotspots".into(),
                value: "7".into(),
                status: "Risk".into(),
                confidence: None,
            },
            MetricRow {
                name: "Dead functions".into(),
                value: "18".into(),
                status: "Review".into(),
                confidence: None,
            },
            MetricRow {
                name: "Avg coupling".into(),
                value: "0.42".into(),
                status: "Moderate".into(),
                confidence: None,
            },
            MetricRow {
                name: "Max call depth".into(),
                value: "14".into(),
                status: "Deep".into(),
                confidence: None,
            },
        ],
        next_commands: vec![
            "gridseak explain finding_001".into(),
            "gridseak graph callers auth/createSession".into(),
            "gridseak graph blast-radius auth/createSession".into(),
            "gridseak compare --previous".into(),
        ],
        schema_caveats: Vec::new(),
        tier_signal: TierSignal::default_v0(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_band_matches_spec_example() {
        // Spec example: Score 62 → "Moderate risk".
        assert_eq!(ScoreBand::from_score(Some(62)), ScoreBand::Moderate);
        assert_eq!(ScoreBand::from_score(Some(62)).label(), "Moderate risk");
    }

    #[test]
    fn score_band_unknown_when_no_score() {
        assert_eq!(ScoreBand::from_score(None), ScoreBand::Unknown);
    }

    #[test]
    fn short_commit_returns_7_chars() {
        assert_eq!(short_commit("9f3c2a1abcdef"), "9f3c2a1");
        assert_eq!(short_commit("9f3c2a1"), "9f3c2a1");
        assert_eq!(short_commit("ab"), "ab");
    }

    #[test]
    fn percent_format_matches_spec_example() {
        // Spec example: tangle 4.2%.
        assert_eq!(format_percent(0.042), "4.2%");
    }

    #[test]
    fn humanise_metric_key_translates_known_keys() {
        assert_eq!(humanise_metric_key("dead_code"), "dead code");
        assert_eq!(humanise_metric_key("hotspots"), "hotspots");
        assert_eq!(humanise_metric_key("hotspot_concentration"), "hotspots");
        assert_eq!(humanise_metric_key("unknown_metric"), "unknown metric");
    }

    // -----------------------------------------------------------------
    // A2 — confidence-aware metric status
    // -----------------------------------------------------------------

    #[test]
    fn confidence_key_for_row_covers_the_four_analyzer_keys() {
        // Mirrors the runtime population set in
        // graphengine-analysis/src/health/mod.rs near
        // `metric_confidence: Some(metric_confidence)`. Hotspots
        // intentionally inherits dead_code's confidence (see analyzer
        // `hotspot_concentration.status = dead_code_status_value`).
        assert_eq!(confidence_key_for_row("Dead functions"), Some("dead_code"));
        assert_eq!(confidence_key_for_row("Hotspots"), Some("dead_code"));
        assert_eq!(confidence_key_for_row("Avg coupling"), Some("coupling"));
        assert_eq!(confidence_key_for_row("Max call depth"), Some("depth"));
        assert_eq!(confidence_key_for_row("Cycles"), None);
        assert_eq!(confidence_key_for_row("Tangle index"), None);
    }

    #[test]
    fn apply_confidence_overrides_marks_dead_code_low_confidence() {
        use graphengine_analysis::health::report::{Confidence, MetricConfidence};
        use std::collections::BTreeMap;

        let mut rows = vec![
            MetricRow {
                name: "Dead functions".into(),
                value: "129".into(),
                status: "Risk".into(),
                confidence: None,
            },
            MetricRow {
                name: "Hotspots".into(),
                value: "256".into(),
                status: "Risk".into(),
                confidence: None,
            },
            MetricRow {
                name: "Cycles".into(),
                value: "0".into(),
                status: "OK".into(),
                confidence: None,
            },
        ];
        let mut report = make_report_with_confidence(BTreeMap::from([(
            "dead_code".into(),
            MetricConfidence {
                level: Confidence::Low,
                reason: "no import edges".into(),
            },
        )]));
        // ensure no other key disturbs the assertion
        report.metrics.metric_confidence = Some(BTreeMap::from([(
            "dead_code".into(),
            MetricConfidence {
                level: Confidence::Low,
                reason: "no import edges".into(),
            },
        )]));

        apply_confidence_overrides(&mut rows, &report);

        assert_eq!(rows[0].status, "Low-confidence · Risk");
        assert_eq!(rows[0].confidence.as_deref(), Some("low"));
        assert_eq!(rows[1].status, "Low-confidence · Risk");
        assert_eq!(rows[1].confidence.as_deref(), Some("low"));
        // unrelated row untouched
        assert_eq!(rows[2].status, "OK");
        assert!(rows[2].confidence.is_none());
    }

    #[test]
    fn apply_confidence_overrides_high_is_no_op() {
        use graphengine_analysis::health::report::{Confidence, MetricConfidence};
        use std::collections::BTreeMap;

        let mut rows = vec![MetricRow {
            name: "Max call depth".into(),
            value: "12".into(),
            status: "Deep".into(),
            confidence: None,
        }];
        let report = make_report_with_confidence(BTreeMap::from([(
            "depth".into(),
            MetricConfidence {
                level: Confidence::High,
                reason: "lots of edges".into(),
            },
        )]));

        apply_confidence_overrides(&mut rows, &report);
        assert_eq!(rows[0].status, "Deep");
        assert!(rows[0].confidence.is_none());
    }

    // -----------------------------------------------------------------
    // A7 — first-scan gating on `gridseak compare --previous`
    // -----------------------------------------------------------------

    #[test]
    fn build_next_commands_first_scan_suggests_scan_again_not_compare() {
        let priorities = sample_priorities();
        let cmds = build_next_commands(&priorities, 1);
        assert!(
            !cmds.iter().any(|c| c == "gridseak compare --previous"),
            "first scan should not suggest compare --previous; got {cmds:?}"
        );
        assert!(
            cmds.iter().any(|c| c.starts_with("gridseak scan .")),
            "first scan should suggest running again; got {cmds:?}"
        );
    }

    #[test]
    fn build_next_commands_second_scan_suggests_compare() {
        let priorities = sample_priorities();
        let cmds = build_next_commands(&priorities, 2);
        assert!(
            cmds.iter().any(|c| c == "gridseak compare --previous"),
            "scan_count=2 should suggest compare --previous; got {cmds:?}"
        );
        assert!(
            !cmds.iter().any(|c| c.contains("run again")),
            "scan_count=2 should not suggest re-running; got {cmds:?}"
        );
    }

    // -----------------------------------------------------------------
    // Test helpers (kept private to this module)
    // -----------------------------------------------------------------

    fn sample_priorities() -> Vec<PriorityRow> {
        vec![PriorityRow {
            rank: 1,
            severity: "Critical".into(),
            finding_id: "f1".into(),
            finding: "Hotspot".into(),
            target: "auth/login".into(),
            evidence: "5 downstream".into(),
            confidence: Some("high".into()),
        }]
    }

    fn make_report_with_confidence(
        map: std::collections::BTreeMap<
            String,
            graphengine_analysis::health::report::MetricConfidence,
        >,
    ) -> HealthReport {
        use graphengine_analysis::health::report::{
            CouplingMetricDetail, DeadCodeMetricDetail, DepthMetricDetail, IntegrityStatus,
            MetricDetail, MetricStatus, MetricsReport, Summary,
        };
        HealthReport {
            version: "test".into(),
            generated_at: "2026-05-17T11:12:00Z".into(),
            analysis_duration_ms: 0,
            db_path: "".into(),
            health_score: Some(60),
            health_score_components: Default::default(),
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
                    reason_breakdown: Default::default(),
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
                metric_confidence: Some(map),
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
            findings: Vec::new(),
            node_annotations: Default::default(),
            module_annotations: Default::default(),
            classifications: Default::default(),
            boundary_violations: Vec::new(),
            resolution_quality: None,
            analysis_errors: Vec::new(),
            integrity_status: IntegrityStatus::default(),
            git_signals: None,
            file_extraction_coverage: Vec::new(),
            primary_language: None,
            analysis_provenance: None,
        }
    }
}
