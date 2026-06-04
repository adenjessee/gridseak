//! View + renderers for `gridseak recommendations`.
//!
//! Output is Fix-First-ordered: the top N priorities ranked by the
//! diagnostic crate's composite score, with the risk narrative and
//! suggested action surfaced alongside the raw evidence.

use std::io::{self, Write};

use graphengine_analysis::health::report::HealthReport;
use graphengine_diagnostic::priority::{self, PriorityItem};
use serde::Serialize;

use crate::render::view::{self};
use crate::render::width::Layout;

#[derive(Debug, Clone, Serialize)]
pub struct RecommendationsView {
    pub repo_name: String,
    pub scan_id: String,
    pub returned: usize,
    pub deep: bool,
    pub recommendations: Vec<RecommendationRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecommendationRow {
    pub rank: usize,
    pub finding_id: String,
    pub severity: String,
    pub finding_type: String,
    pub target: String,
    pub evidence: String,
    pub priority_score: f64,
    pub risk_narrative: String,
    pub suggested_action: String,
    pub confidence: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RecommendationsFormat {
    Table { layout: Layout },
    Markdown,
    Json,
    ForLlm,
}

pub fn render(
    format: &RecommendationsFormat,
    view: &RecommendationsView,
    out: &mut dyn Write,
) -> io::Result<()> {
    match format {
        RecommendationsFormat::Table { layout } => render_table(view, *layout, out),
        RecommendationsFormat::Markdown => render_markdown(view, out),
        RecommendationsFormat::Json => render_json(view, out),
        RecommendationsFormat::ForLlm => render_llm(view, out),
    }
}

fn render_table(view: &RecommendationsView, layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "Top Recommendations — {}", view.repo_name)?;
    if view.recommendations.is_empty() {
        writeln!(out, "(no recommendations on this scan.)")?;
        return Ok(());
    }
    for (idx, row) in view.recommendations.iter().enumerate() {
        if idx > 0 {
            writeln!(out)?;
        }
        writeln!(
            out,
            "#{rank} [{sev}] {ty}",
            rank = row.rank,
            sev = row.severity,
            ty = row.finding_type
        )?;
        writeln!(out, "  target:     {}", row.target)?;
        writeln!(out, "  evidence:   {}", row.evidence)?;
        writeln!(out, "  risk:       {}", wrap(&row.risk_narrative, layout))?;
        writeln!(out, "  action:     {}", wrap(&row.suggested_action, layout))?;
        if let Some(c) = &row.confidence {
            writeln!(out, "  confidence: {c}")?;
        }
        writeln!(out, "  finding_id: {}", row.finding_id)?;
        writeln!(out, "  score:      {:.2}", row.priority_score)?;
    }
    Ok(())
}

fn render_markdown(view: &RecommendationsView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "# Top Recommendations — {}", view.repo_name)?;
    writeln!(out)?;
    if view.recommendations.is_empty() {
        writeln!(out, "_No recommendations on this scan._")?;
        return Ok(());
    }
    for row in &view.recommendations {
        writeln!(
            out,
            "## #{} [{}] {}",
            row.rank,
            row.severity,
            md_escape(&row.finding_type)
        )?;
        writeln!(out)?;
        writeln!(out, "- **Target:** `{}`", md_escape(&row.target))?;
        writeln!(out, "- **Evidence:** {}", md_escape(&row.evidence))?;
        writeln!(out, "- **Risk:** {}", md_escape(&row.risk_narrative))?;
        writeln!(out, "- **Action:** {}", md_escape(&row.suggested_action))?;
        if let Some(c) = &row.confidence {
            writeln!(out, "- **Confidence:** {c}")?;
        }
        writeln!(out, "- **Finding id:** `{}`", row.finding_id)?;
        writeln!(out, "- **Score:** {:.2}", row.priority_score)?;
        writeln!(out)?;
    }
    Ok(())
}

fn render_json(view: &RecommendationsView, out: &mut dyn Write) -> io::Result<()> {
    let envelope = serde_json::json!({
        "schema": "gridseak.recommendations.v1",
        "view": view,
    });
    serde_json::to_writer_pretty(&mut *out, &envelope).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

fn render_llm(view: &RecommendationsView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(
        out,
        "[gridseak recommendations repo={} returned={}]",
        view.repo_name, view.returned
    )?;
    for row in &view.recommendations {
        writeln!(
            out,
            "#{rank} [{sev}] {ty} :: {tgt} :: {evidence} score={score:.2} risk=\"{risk}\" action=\"{action}\" id={id}",
            rank = row.rank,
            sev = row.severity,
            ty = row.finding_type,
            tgt = row.target,
            evidence = row.evidence,
            score = row.priority_score,
            risk = row.risk_narrative.replace('"', "'"),
            action = row.suggested_action.replace('"', "'"),
            id = row.finding_id,
        )?;
    }
    Ok(())
}

fn wrap(text: &str, _layout: Layout) -> String {
    // Conservative: collapse runs of whitespace so the multi-line
    // narratives from the priority crate render as one tight line.
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

use super::util::md_escape;

/// Build the view from a HealthReport using the diagnostic crate's
/// `compute_priorities` for ordering.
pub fn build_view(
    repo_name: String,
    scan_id: String,
    report: &HealthReport,
    limit: usize,
    severity_filter: Option<&str>,
    type_filter: Option<&str>,
    deep: bool,
) -> RecommendationsView {
    let items = priority::compute_priorities(report, limit.max(1) * 4);
    let finding_lookup: std::collections::HashMap<
        &str,
        &graphengine_analysis::health::report::Finding,
    > = report.findings.iter().map(|f| (f.id.as_str(), f)).collect();

    let mut rows: Vec<RecommendationRow> = items
        .iter()
        .filter_map(|item| {
            let finding = finding_lookup.get(item.finding_id.as_str()).copied()?;
            let severity = view::severity_display(finding.severity);
            if let Some(sev) = severity_filter {
                if !severity.eq_ignore_ascii_case(sev) {
                    return None;
                }
            }
            let ftype = view::finding_type_display(finding.finding_type);
            if let Some(ty) = type_filter {
                if ftype.to_ascii_lowercase().replace(' ', "_")
                    != ty.to_ascii_lowercase().replace(' ', "_")
                {
                    return None;
                }
            }
            Some(RecommendationRow {
                rank: item.rank,
                finding_id: item.finding_id.clone(),
                severity: severity.to_string(),
                finding_type: ftype.to_string(),
                target: item.target.clone(),
                evidence: view::evidence_for(finding),
                priority_score: item.priority_score,
                risk_narrative: item.risk_narrative.clone(),
                suggested_action: item.suggested_action.clone(),
                confidence: item.confidence.map(|c| view::confidence_display(c).into()),
            })
        })
        .collect();

    rows.truncate(limit);
    // Re-rank after filtering so users see contiguous numbering.
    for (i, row) in rows.iter_mut().enumerate() {
        row.rank = i + 1;
    }

    RecommendationsView {
        repo_name,
        scan_id,
        returned: rows.len(),
        deep,
        recommendations: rows,
    }
}

/// Helper for `explain` — fetch a single `PriorityItem` by id from a
/// freshly recomputed priority list. Returns `None` when the id is
/// not present (either invalid or below the priority cutoff).
pub fn priority_for(report: &HealthReport, finding_id: &str) -> Option<PriorityItem> {
    let items = priority::compute_priorities(report, report.findings.len().max(1));
    items.into_iter().find(|i| i.finding_id == finding_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn priority_view_from(report: &HealthReport, repo: &str) -> RecommendationsView {
        build_view(repo.into(), "scan_t".into(), report, 3, None, None, false)
    }

    #[test]
    fn build_view_assigns_contiguous_ranks() {
        use graphengine_analysis::health::report::FindingType;

        let mut report = make_min_report();
        report.findings = vec![
            mk_finding(
                "f1",
                graphengine_analysis::health::report::Severity::Critical,
                FindingType::BlastRadiusHotspot,
                "a",
            ),
            mk_finding(
                "f2",
                graphengine_analysis::health::report::Severity::High,
                FindingType::LowCohesion,
                "b",
            ),
        ];

        let view = priority_view_from(&report, "demo");
        assert_eq!(view.recommendations.len(), 2);
        assert_eq!(view.recommendations[0].rank, 1);
        assert_eq!(view.recommendations[1].rank, 2);
    }

    #[test]
    fn snapshot_recommendations_markdown() {
        use graphengine_analysis::health::report::{FindingType, Severity};

        let mut report = make_min_report();
        report.findings = vec![mk_finding(
            "finding_a",
            Severity::Critical,
            FindingType::BlastRadiusHotspot,
            "auth/login",
        )];
        let view = build_view(
            "demo".into(),
            "scan_t".into(),
            &report,
            3,
            None,
            None,
            false,
        );
        let mut buf = Vec::new();
        render(&RecommendationsFormat::Markdown, &view, &mut buf).unwrap();
        insta::assert_snapshot!("recommendations_markdown", String::from_utf8(buf).unwrap());
    }

    fn make_min_report() -> HealthReport {
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
            findings: vec![],
            node_annotations: Default::default(),
            module_annotations: Default::default(),
            classifications: Default::default(),
            boundary_violations: vec![],
            resolution_quality: None,
            analysis_errors: vec![],
            integrity_status: IntegrityStatus::default(),
            git_signals: None,
            file_extraction_coverage: vec![],
            primary_language: None,
            analysis_provenance: None,
        }
    }

    fn mk_finding(
        id: &str,
        sev: graphengine_analysis::health::report::Severity,
        ty: graphengine_analysis::health::report::FindingType,
        target: &str,
    ) -> graphengine_analysis::health::report::Finding {
        graphengine_analysis::health::report::Finding {
            id: id.into(),
            finding_type: ty,
            severity: sev,
            description: "".into(),
            detail: None,
            node_ids: vec![target.into()],
            edge_ids: None,
            primary_node_id: Some(target.into()),
            metric_name: None,
            metric_value: None,
            impact: None,
            blast_radius: Some(10),
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
}
