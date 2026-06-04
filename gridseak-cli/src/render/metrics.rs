//! View + renderers for `gridseak metrics`.
//!
//! Categories follow `docs/03-specs/METRICS_REFERENCE.md`:
//!
//! - `graph`: cycles, tangle index, max call depth
//! - `function`: hotspots, dead code (and complexity when present)
//! - `module`: coupling, cohesion, distance from main sequence
//! - `temporal`: temporal coupling (when git signals ran)
//! - `classification`: per-metric confidence + resolution quality
//! - `composite`: health score breakdown
//!
//! Each category is a `MetricCategory` with its own rows. Callers
//! filter to one or all; rendering produces one table per category.

use std::io::{self, Write};

use graphengine_analysis::health::report::{HealthReport, MetricStatus};
use serde::Serialize;

use crate::render::view::ScoreBand;
use crate::render::width::Layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricCategoryKind {
    Graph,
    Function,
    Module,
    Temporal,
    Classification,
    Composite,
}

impl MetricCategoryKind {
    pub fn all() -> &'static [Self] {
        &[
            Self::Graph,
            Self::Function,
            Self::Module,
            Self::Temporal,
            Self::Classification,
            Self::Composite,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Graph => "Graph",
            Self::Function => "Function",
            Self::Module => "Module",
            Self::Temporal => "Temporal",
            Self::Classification => "Classification",
            Self::Composite => "Composite",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricRow {
    pub name: String,
    pub value: String,
    pub status: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricCategory {
    pub kind: MetricCategoryKind,
    pub rows: Vec<MetricRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsView {
    pub repo_name: String,
    pub scan_id: String,
    pub categories: Vec<MetricCategory>,
}

#[derive(Debug, Clone)]
pub enum MetricsFormat {
    Table { layout: Layout },
    Markdown,
    Json,
    ForLlm,
}

pub fn render(format: &MetricsFormat, view: &MetricsView, out: &mut dyn Write) -> io::Result<()> {
    match format {
        MetricsFormat::Table { layout } => render_table(view, *layout, out),
        MetricsFormat::Markdown => render_markdown(view, out),
        MetricsFormat::Json => render_json(view, out),
        MetricsFormat::ForLlm => render_llm(view, out),
    }
}

fn render_table(view: &MetricsView, _layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "Metrics — {}", view.repo_name)?;
    for cat in &view.categories {
        if cat.rows.is_empty() {
            continue;
        }
        writeln!(out)?;
        writeln!(out, "[{}]", cat.kind.label())?;
        let header = ["Metric", "Value", "Status"];
        let mut widths = [header[0].len(), header[1].len(), header[2].len()];
        for row in &cat.rows {
            widths[0] = widths[0].max(row.name.len());
            widths[1] = widths[1].max(row.value.len());
            widths[2] = widths[2].max(row.status.len());
        }
        writeln!(
            out,
            "{:<n0$}  {:<n1$}  {:<n2$}",
            header[0],
            header[1],
            header[2],
            n0 = widths[0],
            n1 = widths[1],
            n2 = widths[2],
        )?;
        for row in &cat.rows {
            writeln!(
                out,
                "{:<n0$}  {:<n1$}  {:<n2$}",
                row.name,
                row.value,
                row.status,
                n0 = widths[0],
                n1 = widths[1],
                n2 = widths[2],
            )?;
        }
    }
    Ok(())
}

fn render_markdown(view: &MetricsView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "# Metrics — {}", view.repo_name)?;
    for cat in &view.categories {
        if cat.rows.is_empty() {
            continue;
        }
        writeln!(out)?;
        writeln!(out, "## {}", cat.kind.label())?;
        writeln!(out)?;
        writeln!(out, "| Metric | Value | Status |")?;
        writeln!(out, "|--------|-------|--------|")?;
        for row in &cat.rows {
            writeln!(
                out,
                "| {} | {} | {} |",
                md_escape(&row.name),
                md_escape(&row.value),
                md_escape(&row.status),
            )?;
        }
    }
    Ok(())
}

fn render_json(view: &MetricsView, out: &mut dyn Write) -> io::Result<()> {
    let envelope = serde_json::json!({
        "schema": "gridseak.metrics.v1",
        "view": view,
    });
    serde_json::to_writer_pretty(&mut *out, &envelope).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

fn render_llm(view: &MetricsView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "[gridseak metrics repo={}]", view.repo_name)?;
    for cat in &view.categories {
        if cat.rows.is_empty() {
            continue;
        }
        writeln!(out, "## {}", cat.kind.label().to_ascii_lowercase())?;
        for row in &cat.rows {
            writeln!(out, "{} = {} ({})", row.name, row.value, row.status)?;
        }
    }
    Ok(())
}

use super::util::md_escape;

/// Build the view for `gridseak metrics`. `categories` is the filter
/// set; `None` or empty means all categories.
pub fn build_view(
    repo_name: String,
    scan_id: String,
    report: &HealthReport,
    categories: Option<&[MetricCategoryKind]>,
) -> MetricsView {
    let want: Vec<MetricCategoryKind> = match categories {
        Some(slice) if !slice.is_empty() => slice.to_vec(),
        _ => MetricCategoryKind::all().to_vec(),
    };
    let mut cats = Vec::new();
    for kind in want {
        let rows = match kind {
            MetricCategoryKind::Graph => graph_rows(report),
            MetricCategoryKind::Function => function_rows(report),
            MetricCategoryKind::Module => module_rows(report),
            MetricCategoryKind::Temporal => temporal_rows(report),
            MetricCategoryKind::Classification => classification_rows(report),
            MetricCategoryKind::Composite => composite_rows(report),
        };
        cats.push(MetricCategory { kind, rows });
    }
    MetricsView {
        repo_name,
        scan_id,
        categories: cats,
    }
}

// ---------------------------------------------------------------------------
// Per-category row builders
// ---------------------------------------------------------------------------

fn graph_rows(report: &HealthReport) -> Vec<MetricRow> {
    let m = &report.metrics;
    let mut rows = Vec::new();
    rows.push(MetricRow {
        name: "Cycles".into(),
        value: m.cycles.count.to_string(),
        status: count_status(m.cycles.status, m.cycles.count),
        description: Some(m.cycles.description.clone()),
    });
    rows.push(MetricRow {
        name: "Tangle index".into(),
        value: format!("{:.1}%", m.tangle_index.ratio * 100.0),
        status: ratio_status(m.tangle_index.status, m.tangle_index.ratio, 0.02, 0.05),
        description: Some(m.tangle_index.description.clone()),
    });
    rows.push(MetricRow {
        name: "Max call depth".into(),
        value: m.depth.max_call_depth.to_string(),
        status: depth_status(m.depth.status, m.depth.max_call_depth),
        description: Some(m.depth.description.clone()),
    });
    rows
}

fn function_rows(report: &HealthReport) -> Vec<MetricRow> {
    let m = &report.metrics;
    let mut rows = vec![
        MetricRow {
            name: "Hotspots".into(),
            value: m.hotspot_concentration.count.to_string(),
            status: count_status(
                m.hotspot_concentration.status,
                m.hotspot_concentration.count,
            ),
            description: Some(m.hotspot_concentration.description.clone()),
        },
        MetricRow {
            name: "Dead functions".into(),
            value: m.dead_code.count.to_string(),
            status: dead_status(m.dead_code.status, m.dead_code.count),
            description: Some(m.dead_code.description.clone()),
        },
    ];
    if let Some(c) = m.complexity.as_ref() {
        rows.push(MetricRow {
            name: "Avg cyclomatic".into(),
            value: format!("{:.2}", c.avg_cyclomatic),
            status: complexity_status(c.max_cyclomatic),
            description: Some(c.description.clone()),
        });
        rows.push(MetricRow {
            name: "Max cyclomatic".into(),
            value: c.max_cyclomatic.to_string(),
            status: complexity_status(c.max_cyclomatic),
            description: None,
        });
        rows.push(MetricRow {
            name: "Avg cognitive".into(),
            value: format!("{:.2}", c.avg_cognitive),
            status: cognitive_status(c.max_cognitive),
            description: None,
        });
        rows.push(MetricRow {
            name: "Functions above threshold".into(),
            value: c.functions_above_threshold.to_string(),
            status: count_status(MetricStatus::Ok, c.functions_above_threshold),
            description: None,
        });
    }
    rows.push(MetricRow {
        name: "Avg fan-in".into(),
        value: format!("{:.2}", report.summary.avg_fan_in),
        status: "—".into(),
        description: None,
    });
    rows.push(MetricRow {
        name: "Avg fan-out".into(),
        value: format!("{:.2}", report.summary.avg_fan_out),
        status: "—".into(),
        description: None,
    });
    rows
}

fn module_rows(report: &HealthReport) -> Vec<MetricRow> {
    let m = &report.metrics;
    let mut rows = vec![MetricRow {
        name: "Avg coupling".into(),
        value: format!("{:.2}", m.coupling.avg_coupling),
        status: coupling_status(m.coupling.status, m.coupling.avg_coupling),
        description: Some(m.coupling.description.clone()),
    }];
    if let Some(cohesion) = m.cohesion.as_ref() {
        rows.push(MetricRow {
            name: "Avg cohesion".into(),
            value: format!("{:.2}", cohesion.avg_cohesion),
            status: cohesion_status(cohesion.avg_cohesion),
            description: Some(cohesion.description.clone()),
        });
        rows.push(MetricRow {
            name: "Low cohesion modules".into(),
            value: cohesion.low_cohesion_modules.to_string(),
            status: count_status(MetricStatus::Ok, cohesion.low_cohesion_modules),
            description: None,
        });
    }
    if let Some(dist) = m.distance_from_main_sequence.as_ref() {
        rows.push(MetricRow {
            name: "Avg distance from main sequence".into(),
            value: format!("{:.2}", dist.avg_distance),
            status: "—".into(),
            description: Some(dist.description.clone()),
        });
        rows.push(MetricRow {
            name: "Zone of pain modules".into(),
            value: dist.zone_of_pain_modules.to_string(),
            status: count_status(MetricStatus::Ok, dist.zone_of_pain_modules),
            description: None,
        });
        rows.push(MetricRow {
            name: "Zone of uselessness modules".into(),
            value: dist.zone_of_uselessness_modules.to_string(),
            status: count_status(MetricStatus::Ok, dist.zone_of_uselessness_modules),
            description: None,
        });
    }
    rows.push(MetricRow {
        name: "High-coupling modules".into(),
        value: report.summary.high_coupling_modules.to_string(),
        status: count_status(MetricStatus::Ok, report.summary.high_coupling_modules),
        description: None,
    });
    rows
}

fn temporal_rows(report: &HealthReport) -> Vec<MetricRow> {
    let Some(tc) = report.metrics.temporal_coupling.as_ref() else {
        return Vec::new();
    };
    vec![
        MetricRow {
            name: "High-coupling pairs".into(),
            value: tc.high_coupling_pairs.to_string(),
            status: count_status(MetricStatus::Ok, tc.high_coupling_pairs),
            description: Some(tc.description.clone()),
        },
        MetricRow {
            name: "Hidden-coupling pairs".into(),
            value: tc.hidden_coupling_pairs.to_string(),
            status: count_status(MetricStatus::Ok, tc.hidden_coupling_pairs),
            description: None,
        },
    ]
}

fn classification_rows(report: &HealthReport) -> Vec<MetricRow> {
    let mut rows = Vec::new();
    if let Some(rq) = report.resolution_quality.as_ref() {
        rows.push(MetricRow {
            name: "Resolution tier".into(),
            value: format!("{:?}", rq.resolution_tier).to_ascii_lowercase(),
            status: "—".into(),
            description: None,
        });
        rows.push(MetricRow {
            name: "Import edges total".into(),
            value: rq.import_edges_total.to_string(),
            status: "—".into(),
            description: None,
        });
    }
    if let Some(per_metric) = report.metrics.metric_confidence.as_ref() {
        for (key, conf) in per_metric {
            rows.push(MetricRow {
                name: format!("Confidence: {}", key.replace('_', " ")),
                value: format!("{:?}", conf.level).to_ascii_lowercase(),
                status: "—".into(),
                description: Some(conf.reason.clone()),
            });
        }
    }
    rows
}

fn composite_rows(report: &HealthReport) -> Vec<MetricRow> {
    let mut rows = Vec::new();
    rows.push(MetricRow {
        name: "Health score".into(),
        value: report
            .health_score
            .map(|s| format!("{s}/100"))
            .unwrap_or_else(|| "—".into()),
        status: ScoreBand::from_score(report.health_score).label().into(),
        description: None,
    });
    rows.push(MetricRow {
        name: "Findings (total)".into(),
        value: report.findings.len().to_string(),
        status: "—".into(),
        description: None,
    });
    let critical = report
        .findings
        .iter()
        .filter(|f| {
            matches!(
                f.severity,
                graphengine_analysis::health::report::Severity::Critical
            )
        })
        .count();
    let high = report
        .findings
        .iter()
        .filter(|f| {
            matches!(
                f.severity,
                graphengine_analysis::health::report::Severity::High
            )
        })
        .count();
    rows.push(MetricRow {
        name: "Findings (critical)".into(),
        value: critical.to_string(),
        status: count_status(MetricStatus::Ok, critical),
        description: None,
    });
    rows.push(MetricRow {
        name: "Findings (high)".into(),
        value: high.to_string(),
        status: count_status(MetricStatus::Ok, high),
        description: None,
    });
    rows
}

// ---------------------------------------------------------------------------
// Status helpers
// ---------------------------------------------------------------------------

fn count_status(status: MetricStatus, count: usize) -> String {
    if let Some(badge) = non_ok_badge(status) {
        return badge.into();
    }
    if count == 0 {
        "OK".into()
    } else {
        "Risk".into()
    }
}

fn ratio_status(status: MetricStatus, value: f64, mod_thr: f64, risk_thr: f64) -> String {
    if let Some(badge) = non_ok_badge(status) {
        return badge.into();
    }
    if value >= risk_thr {
        "Risk".into()
    } else if value >= mod_thr {
        "Moderate".into()
    } else {
        "OK".into()
    }
}

fn dead_status(status: MetricStatus, count: usize) -> String {
    if let Some(badge) = non_ok_badge(status) {
        return badge.into();
    }
    if count == 0 {
        "OK".into()
    } else if count < 50 {
        "Review".into()
    } else {
        "Risk".into()
    }
}

fn coupling_status(status: MetricStatus, avg: f64) -> String {
    if let Some(badge) = non_ok_badge(status) {
        return badge.into();
    }
    if avg >= 0.6 {
        "Risk".into()
    } else if avg >= 0.3 {
        "Moderate".into()
    } else {
        "OK".into()
    }
}

fn depth_status(status: MetricStatus, depth: usize) -> String {
    if let Some(badge) = non_ok_badge(status) {
        return badge.into();
    }
    if depth >= 8 {
        "Deep".into()
    } else if depth >= 4 {
        "Moderate".into()
    } else {
        "OK".into()
    }
}

fn cohesion_status(avg: f64) -> String {
    if avg >= 0.7 {
        "OK".into()
    } else if avg >= 0.4 {
        "Moderate".into()
    } else {
        "Risk".into()
    }
}

fn complexity_status(max_cyclomatic: u32) -> String {
    if max_cyclomatic >= 25 {
        "Risk".into()
    } else if max_cyclomatic >= 15 {
        "Moderate".into()
    } else {
        "OK".into()
    }
}

fn cognitive_status(max_cognitive: u32) -> String {
    if max_cognitive >= 25 {
        "Risk".into()
    } else if max_cognitive >= 15 {
        "Moderate".into()
    } else {
        "OK".into()
    }
}

fn non_ok_badge(status: MetricStatus) -> Option<&'static str> {
    match status {
        MetricStatus::Ok => None,
        MetricStatus::InsufficientEdges => Some("Not measured"),
        MetricStatus::FrameworkInvisible => Some("Framework-invisible"),
        MetricStatus::NotApplicable => Some("N/A"),
        MetricStatus::ComputationFailed => Some("Failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_report() -> HealthReport {
        use graphengine_analysis::health::report::{
            CouplingMetricDetail, DeadCodeMetricDetail, DepthMetricDetail, IntegrityStatus,
            MetricDetail, MetricsReport, Summary,
        };
        HealthReport {
            version: "test".into(),
            generated_at: "2026-05-17T11:12:00Z".into(),
            analysis_duration_ms: 0,
            db_path: "".into(),
            health_score: Some(62),
            health_score_components: Default::default(),
            metrics: MetricsReport {
                cycles: MetricDetail {
                    count: 3,
                    total: 0,
                    ratio: 0.0,
                    description: "cycle count".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                coupling: CouplingMetricDetail {
                    modules_measured: 10,
                    modules_above_070: 1,
                    modules_above_050: 3,
                    avg_coupling: 0.42,
                    description: "coupling".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                hotspot_concentration: MetricDetail {
                    count: 7,
                    total: 100,
                    ratio: 0.07,
                    description: "hotspots".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                dead_code: DeadCodeMetricDetail {
                    count: 18,
                    total: 100,
                    ratio: 0.18,
                    description: "dead code".into(),
                    status: MetricStatus::Ok,
                    reason_breakdown: Default::default(),
                    reason_breakdown_caveats: None,
                    fidelity: None,
                    no_callers_total: None,
                    no_callers_high_confidence: None,
                },
                depth: DepthMetricDetail {
                    max_call_depth: 14,
                    description: "depth".into(),
                    status: MetricStatus::Ok,
                    fidelity: None,
                },
                tangle_index: MetricDetail {
                    count: 0,
                    total: 0,
                    ratio: 0.042,
                    description: "tangle".into(),
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
                cycles_found: 3,
                cycle_total_nodes: 0,
                hotspot_count: 7,
                hotspot_threshold_fan_in: 0,
                high_coupling_modules: 0,
                dead_functions: 18,
                max_call_depth: 14,
                tangle_index: 0.042,
                avg_module_coupling: 0.42,
                avg_fan_in: 2.1,
                avg_fan_out: 2.4,
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

    #[test]
    fn graph_category_lists_three_rows() {
        let view = build_view(
            "demo".into(),
            "scan_t".into(),
            &empty_report(),
            Some(&[MetricCategoryKind::Graph]),
        );
        assert_eq!(view.categories.len(), 1);
        assert_eq!(view.categories[0].rows.len(), 3);
        assert_eq!(view.categories[0].rows[0].name, "Cycles");
    }

    #[test]
    fn snapshot_metrics_table() {
        let view = build_view("demo".into(), "scan_t".into(), &empty_report(), None);
        let mut buf = Vec::new();
        render(
            &MetricsFormat::Table {
                layout: Layout::Wide,
            },
            &view,
            &mut buf,
        )
        .unwrap();
        insta::assert_snapshot!("metrics_table_all", String::from_utf8(buf).unwrap());
    }

    #[test]
    fn snapshot_metrics_markdown() {
        let view = build_view("demo".into(), "scan_t".into(), &empty_report(), None);
        let mut buf = Vec::new();
        render(&MetricsFormat::Markdown, &view, &mut buf).unwrap();
        insta::assert_snapshot!("metrics_markdown_all", String::from_utf8(buf).unwrap());
    }
}
