//! View + renderers for `gridseak findings`.

use std::io::{self, Write};

use graphengine_analysis::health::report::{Finding, HealthReport};
use serde::Serialize;

use crate::render::view::{self, MetricRow as _MetricRow};
use crate::render::width::Layout;

#[derive(Debug, Clone, Serialize)]
pub struct FindingsView {
    pub repo_name: String,
    pub scan_id: String,
    pub total: usize,
    pub returned: usize,
    pub filters: FindingsFilters,
    pub findings: Vec<FindingRow>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct FindingsFilters {
    pub severity: Option<String>,
    pub finding_type: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindingRow {
    pub id: String,
    pub severity: String,
    pub finding_type: String,
    pub target: String,
    pub evidence: String,
    pub confidence: Option<String>,
}

impl FindingRow {
    pub fn from_finding(finding: &Finding) -> Self {
        let target = finding
            .primary_node_id
            .clone()
            .or_else(|| finding.node_ids.first().cloned())
            .unwrap_or_else(|| "—".into());
        Self {
            id: finding.id.clone(),
            severity: view::severity_display(finding.severity).into(),
            finding_type: view::finding_type_display(finding.finding_type).into(),
            target,
            evidence: view::evidence_for(finding),
            confidence: finding
                .confidence
                .map(|c| view::confidence_display(c).into()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum FindingsFormat {
    Table { layout: Layout },
    Markdown,
    Json,
    ForLlm,
}

pub fn render(format: &FindingsFormat, view: &FindingsView, out: &mut dyn Write) -> io::Result<()> {
    match format {
        FindingsFormat::Table { layout } => render_table(view, *layout, out),
        FindingsFormat::Markdown => render_markdown(view, out),
        FindingsFormat::Json => render_json(view, out),
        FindingsFormat::ForLlm => render_llm(view, out),
    }
}

fn render_table(view: &FindingsView, layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    writeln!(
        out,
        "Findings — {} (showing {} of {})",
        view.repo_name, view.returned, view.total
    )?;
    if view.findings.is_empty() {
        writeln!(out, "(no findings matched the filter set.)")?;
        return Ok(());
    }
    match layout {
        Layout::Wide => {
            let header = ["ID", "Severity", "Type", "Target", "Evidence"];
            let mut widths: Vec<usize> = header.iter().map(|c| c.len()).collect();
            let rows: Vec<Vec<String>> = view
                .findings
                .iter()
                .map(|f| {
                    vec![
                        f.id.clone(),
                        f.severity.clone(),
                        f.finding_type.clone(),
                        truncate(&f.target, 40),
                        truncate(&f.evidence, 36),
                    ]
                })
                .collect();
            for row in &rows {
                for (i, c) in row.iter().enumerate() {
                    widths[i] = widths[i].max(c.len());
                }
            }
            write_columns(
                out,
                &header.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                &widths,
            )?;
            for row in rows {
                write_columns(out, &row, &widths)?;
            }
        }
        Layout::Medium => {
            let header = ["Severity", "Type", "Target"];
            let mut widths: Vec<usize> = header.iter().map(|c| c.len()).collect();
            let rows: Vec<Vec<String>> = view
                .findings
                .iter()
                .map(|f| {
                    vec![
                        f.severity.clone(),
                        f.finding_type.clone(),
                        truncate(&f.target, 50),
                    ]
                })
                .collect();
            for row in &rows {
                for (i, c) in row.iter().enumerate() {
                    widths[i] = widths[i].max(c.len());
                }
            }
            write_columns(
                out,
                &header.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                &widths,
            )?;
            for row in rows {
                write_columns(out, &row, &widths)?;
            }
        }
        Layout::Narrow => {
            for (i, f) in view.findings.iter().enumerate() {
                if i > 0 {
                    writeln!(out)?;
                }
                writeln!(out, "[{}] {} — {}", f.severity, f.finding_type, f.target)?;
                writeln!(out, "  id:       {}", f.id)?;
                writeln!(out, "  evidence: {}", f.evidence)?;
                if let Some(c) = &f.confidence {
                    writeln!(out, "  confidence: {c}")?;
                }
            }
        }
        Layout::Plain => {
            for f in &view.findings {
                writeln!(
                    out,
                    "{id} [{sev}] {ty} :: {tgt} :: {ev}",
                    id = f.id,
                    sev = f.severity,
                    ty = f.finding_type,
                    tgt = f.target,
                    ev = f.evidence
                )?;
            }
        }
    }
    Ok(())
}

fn render_markdown(view: &FindingsView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "# Findings — {}", view.repo_name)?;
    writeln!(out)?;
    writeln!(out, "_{} of {} findings._", view.returned, view.total)?;
    writeln!(out)?;
    if view.findings.is_empty() {
        writeln!(out, "_No findings matched the filter set._")?;
        return Ok(());
    }
    writeln!(out, "| ID | Severity | Type | Target | Evidence |")?;
    writeln!(out, "|----|----------|------|--------|----------|")?;
    for f in &view.findings {
        writeln!(
            out,
            "| `{id}` | {sev} | {ty} | `{tgt}` | {ev} |",
            id = md_escape(&f.id),
            sev = md_escape(&f.severity),
            ty = md_escape(&f.finding_type),
            tgt = md_escape(&f.target),
            ev = md_escape(&f.evidence),
        )?;
    }
    Ok(())
}

fn render_json(view: &FindingsView, out: &mut dyn Write) -> io::Result<()> {
    let envelope = serde_json::json!({
        "schema": "gridseak.findings.v1",
        "view": view,
    });
    serde_json::to_writer_pretty(&mut *out, &envelope).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

fn render_llm(view: &FindingsView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(
        out,
        "[gridseak findings repo={} returned={} total={}]",
        view.repo_name, view.returned, view.total
    )?;
    for f in &view.findings {
        writeln!(
            out,
            "{id} [{sev}] {ty} :: {tgt} :: {ev}{conf}",
            id = f.id,
            sev = f.severity,
            ty = f.finding_type,
            tgt = f.target,
            ev = f.evidence,
            conf = f
                .confidence
                .as_deref()
                .map(|c| format!(" (confidence={c})"))
                .unwrap_or_default(),
        )?;
    }
    Ok(())
}

fn write_columns(out: &mut dyn Write, cells: &[String], widths: &[usize]) -> io::Result<()> {
    let mut first = true;
    for (cell, width) in cells.iter().zip(widths.iter()) {
        if !first {
            write!(out, "  ")?;
        }
        write!(out, "{:<w$}", cell, w = *width)?;
        first = false;
    }
    writeln!(out)?;
    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else if max_len <= 1 {
        "…".into()
    } else {
        let mut t: String = s.chars().take(max_len - 1).collect();
        t.push('…');
        t
    }
}

use super::util::md_escape;

/// Build the view from a persisted HealthReport, applying filters.
pub fn build_view(
    repo_name: String,
    scan_id: String,
    report: &HealthReport,
    severity: Option<&str>,
    finding_type: Option<&str>,
    limit: Option<usize>,
) -> FindingsView {
    let mut findings: Vec<&Finding> = report.findings.iter().collect();
    if let Some(sev) = severity {
        let needle = sev.to_ascii_lowercase();
        findings.retain(|f| view::severity_display(f.severity).to_ascii_lowercase() == needle);
    }
    if let Some(ty) = finding_type {
        let needle = ty.to_ascii_lowercase();
        findings.retain(|f| {
            view::finding_type_display(f.finding_type)
                .to_ascii_lowercase()
                .replace(' ', "_")
                == needle.replace(' ', "_")
        });
    }
    let total = report.findings.len();
    if let Some(limit) = limit {
        findings.truncate(limit);
    }
    let returned = findings.len();
    let rows: Vec<FindingRow> = findings
        .iter()
        .map(|f| FindingRow::from_finding(f))
        .collect();
    FindingsView {
        repo_name,
        scan_id,
        total,
        returned,
        filters: FindingsFilters {
            severity: severity.map(str::to_string),
            finding_type: finding_type.map(str::to_string),
            limit,
        },
        findings: rows,
    }
}

// Silence unused-import warning in this module's scope.
#[allow(dead_code)]
fn _silence(_: _MetricRow) {}

#[cfg(test)]
mod tests {
    use super::*;
    use graphengine_analysis::health::report::{
        FindingType, IntegrityStatus, MetricStatus, MetricsReport, Severity, Summary,
    };

    fn report_with_findings(findings: Vec<Finding>) -> HealthReport {
        HealthReport {
            version: "test".into(),
            generated_at: "2026-05-17T11:12:00Z".into(),
            analysis_duration_ms: 0,
            db_path: "".into(),
            health_score: Some(62),
            health_score_components: Default::default(),
            metrics: MetricsReport {
                cycles: graphengine_analysis::health::report::MetricDetail {
                    count: 0,
                    total: 0,
                    ratio: 0.0,
                    description: "".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                coupling: graphengine_analysis::health::report::CouplingMetricDetail {
                    modules_measured: 0,
                    modules_above_070: 0,
                    modules_above_050: 0,
                    avg_coupling: 0.0,
                    description: "".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                hotspot_concentration: graphengine_analysis::health::report::MetricDetail {
                    count: 0,
                    total: 0,
                    ratio: 0.0,
                    description: "".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                dead_code: graphengine_analysis::health::report::DeadCodeMetricDetail {
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
                depth: graphengine_analysis::health::report::DepthMetricDetail {
                    max_call_depth: 0,
                    description: "".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                tangle_index: graphengine_analysis::health::report::MetricDetail {
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

    fn finding(id: &str, sev: Severity, ty: FindingType, target: &str) -> Finding {
        Finding {
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
            blast_radius: Some(41),
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
    fn filters_apply_in_order() {
        let report = report_with_findings(vec![
            finding(
                "f1",
                Severity::Critical,
                FindingType::BlastRadiusHotspot,
                "a",
            ),
            finding("f2", Severity::High, FindingType::LowCohesion, "b"),
            finding(
                "f3",
                Severity::Critical,
                FindingType::CircularDependency,
                "c",
            ),
        ]);
        let v = build_view(
            "demo".into(),
            "scan".into(),
            &report,
            Some("critical"),
            None,
            None,
        );
        assert_eq!(v.returned, 2);
        assert_eq!(v.total, 3);
        let v = build_view(
            "demo".into(),
            "scan".into(),
            &report,
            None,
            Some("circular_dependency"),
            None,
        );
        assert_eq!(v.returned, 1);
    }

    #[test]
    fn snapshot_findings_table_wide() {
        let report = report_with_findings(vec![
            finding(
                "f1",
                Severity::Critical,
                FindingType::BlastRadiusHotspot,
                "auth/login",
            ),
            finding(
                "f2",
                Severity::High,
                FindingType::LowCohesion,
                "src/services",
            ),
        ]);
        let view = build_view("demo".into(), "scan_t".into(), &report, None, None, None);
        let mut buf = Vec::new();
        render(
            &FindingsFormat::Table {
                layout: Layout::Wide,
            },
            &view,
            &mut buf,
        )
        .unwrap();
        insta::assert_snapshot!("findings_table_wide", String::from_utf8(buf).unwrap());
    }

    #[test]
    fn snapshot_findings_markdown() {
        let report = report_with_findings(vec![finding(
            "f1",
            Severity::Critical,
            FindingType::BlastRadiusHotspot,
            "auth/login",
        )]);
        let view = build_view("demo".into(), "scan_t".into(), &report, None, None, None);
        let mut buf = Vec::new();
        render(&FindingsFormat::Markdown, &view, &mut buf).unwrap();
        insta::assert_snapshot!("findings_markdown", String::from_utf8(buf).unwrap());
    }
}
