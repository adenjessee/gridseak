//! Drivers for the Stage 5 drill-down commands:
//!
//! - `gridseak recommendations` — Fix-First-ordered top priorities
//! - `gridseak findings` — full finding stream with filters
//! - `gridseak explain <finding_id>` — single-finding deep dive
//! - `gridseak metrics` — categorised metric tables
//! - `gridseak report` — hero report against the latest scan (alias
//!   for `gridseak scan latest`; `--full` dumps the raw HealthReport)
//!
//! All five share the same format-resolution ladder as the history
//! commands (`--for-llm` > `--json` > `--format`) so the user never
//! has to learn a different escape sequence per command.

use std::io::Write;

use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use graphengine_analysis::health::report::HealthReport;
use graphengine_diagnostic::priority::DEFAULT_TOP_N;
use gridseak_local_store::ProjectStore;

use crate::render::{
    findings as findings_render,
    metrics::{self as metrics_render, MetricCategoryKind},
    recommendations as recommendations_render, render_hero,
    view::{self as view_mod, ScanReportView},
    width, HeroFormat,
};
use crate::scan_command::ScanOutputFormat;

// ---------------------------------------------------------------------------
// Shared format resolver
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Resolved {
    Table,
    Markdown,
    Json,
    ForLlm,
}

fn resolve_format(format: ScanOutputFormat, for_llm: bool, global_json: bool) -> Resolved {
    if for_llm {
        return Resolved::ForLlm;
    }
    if global_json {
        return Resolved::Json;
    }
    match format {
        ScanOutputFormat::Table => Resolved::Table,
        ScanOutputFormat::Markdown => Resolved::Markdown,
        ScanOutputFormat::Json => Resolved::Json,
    }
}

// ---------------------------------------------------------------------------
// `gridseak recommendations`
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Clone)]
pub struct RecommendationsArgs {
    #[arg(default_value = ".")]
    pub project: String,

    #[arg(long, default_value_t = DEFAULT_TOP_N)]
    pub limit: usize,

    #[arg(long)]
    pub severity: Option<String>,

    #[arg(long)]
    pub r#type: Option<String>,

    /// Include the raw HealthReport JSON in the response (only
    /// applies to `--format json` or `--json`).
    #[arg(long, default_value_t = false)]
    pub deep: bool,

    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table)]
    pub format: ScanOutputFormat,

    #[arg(long, default_value_t = false)]
    pub for_llm: bool,
}

pub fn run_recommendations(
    store: &ProjectStore,
    args: RecommendationsArgs,
    global_json: bool,
) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let scan = project
        .latest_scan
        .clone()
        .context("project has no scans — run `gridseak scan .` first")?;
    let report: HealthReport = serde_json::from_value(store.load_report(&scan.id)?)?;
    let view = recommendations_render::build_view(
        project.display_name.clone(),
        scan.id.clone(),
        &report,
        args.limit,
        args.severity.as_deref(),
        args.r#type.as_deref(),
        args.deep,
    );
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let format = match resolve_format(args.format, args.for_llm, global_json) {
        Resolved::Table => recommendations_render::RecommendationsFormat::Table {
            layout: width::detect(),
        },
        Resolved::Markdown => recommendations_render::RecommendationsFormat::Markdown,
        Resolved::Json => recommendations_render::RecommendationsFormat::Json,
        Resolved::ForLlm => recommendations_render::RecommendationsFormat::ForLlm,
    };
    recommendations_render::render(&format, &view, &mut handle)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `gridseak findings`
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Clone)]
pub struct FindingsArgs {
    #[arg(default_value = ".")]
    pub project: String,

    #[arg(long)]
    pub severity: Option<String>,

    #[arg(long)]
    pub r#type: Option<String>,

    #[arg(long, default_value_t = 50)]
    pub limit: usize,

    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table)]
    pub format: ScanOutputFormat,

    #[arg(long, default_value_t = false)]
    pub for_llm: bool,
}

pub fn run_findings(store: &ProjectStore, args: FindingsArgs, global_json: bool) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let scan = project
        .latest_scan
        .clone()
        .context("project has no scans — run `gridseak scan .` first")?;
    let report: HealthReport = serde_json::from_value(store.load_report(&scan.id)?)?;
    let view = findings_render::build_view(
        project.display_name.clone(),
        scan.id.clone(),
        &report,
        args.severity.as_deref(),
        args.r#type.as_deref(),
        Some(args.limit),
    );
    let format = match resolve_format(args.format, args.for_llm, global_json) {
        Resolved::Table => findings_render::FindingsFormat::Table {
            layout: width::detect(),
        },
        Resolved::Markdown => findings_render::FindingsFormat::Markdown,
        Resolved::Json => findings_render::FindingsFormat::Json,
        Resolved::ForLlm => findings_render::FindingsFormat::ForLlm,
    };
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    findings_render::render(&format, &view, &mut handle)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `gridseak explain`
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Clone)]
pub struct ExplainArgs {
    /// Finding id (from `gridseak recommendations` or `gridseak
    /// findings`).
    pub finding_id: String,

    #[arg(long, default_value = ".")]
    pub project: String,

    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table)]
    pub format: ScanOutputFormat,

    #[arg(long, default_value_t = false)]
    pub for_llm: bool,
}

pub fn run_explain(store: &ProjectStore, args: ExplainArgs, global_json: bool) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let scan = project
        .latest_scan
        .clone()
        .context("project has no scans — run `gridseak scan .` first")?;
    let report: HealthReport = serde_json::from_value(store.load_report(&scan.id)?)?;
    let finding = report
        .findings
        .iter()
        .find(|f| f.id == args.finding_id)
        .with_context(|| {
            format!(
                "finding id `{}` not present on the latest scan",
                args.finding_id
            )
        })?;
    let target = finding
        .primary_node_id
        .clone()
        .or_else(|| finding.node_ids.first().cloned())
        .unwrap_or_else(|| "—".into());
    let severity = view_mod::severity_display(finding.severity).to_string();
    let finding_type = view_mod::finding_type_display(finding.finding_type).to_string();
    let evidence = view_mod::evidence_for(finding);
    let confidence = finding
        .confidence
        .map(|c| view_mod::confidence_display(c).to_string());
    let priority = recommendations_render::priority_for(&report, &finding.id);

    let related_commands = build_related_commands(&target, &finding.id);

    // Render. Stage 5's explain text is plain-prose by design: no
    // table, no JSON-pretty-print, just sections labelled so a
    // human or LLM can navigate them.
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let format = resolve_format(args.format, args.for_llm, global_json);
    match format {
        Resolved::Json => {
            let envelope = serde_json::json!({
                "schema": "gridseak.explain.v1",
                "view": {
                    "repo_name": project.display_name,
                    "scan_id": scan.id,
                    "finding_id": finding.id,
                    "severity": severity,
                    "finding_type": finding_type,
                    "target": target,
                    "evidence": evidence,
                    "confidence": confidence,
                    "impact": finding.impact,
                    "recommendation": finding.recommendation,
                    "priority_score": priority.as_ref().map(|p| p.priority_score),
                    "risk_narrative": priority.as_ref().map(|p| p.risk_narrative.clone()),
                    "suggested_action": priority.as_ref().map(|p| p.suggested_action.clone()),
                    "related_commands": related_commands,
                },
            });
            serde_json::to_writer_pretty(&mut handle, &envelope)?;
            writeln!(handle)?;
        }
        Resolved::Markdown => {
            writeln!(handle, "# {} — {}", finding_type, target)?;
            writeln!(handle)?;
            writeln!(handle, "- **Finding id:** `{}`", finding.id)?;
            writeln!(handle, "- **Severity:** {severity}")?;
            writeln!(handle, "- **Evidence:** {evidence}")?;
            if let Some(c) = &confidence {
                writeln!(handle, "- **Confidence:** {c}")?;
            }
            if let Some(impact) = &finding.impact {
                writeln!(handle, "- **Impact:** {impact}")?;
            }
            if let Some(rec) = &finding.recommendation {
                writeln!(handle, "- **Recommendation:** {rec}")?;
            }
            if let Some(p) = &priority {
                writeln!(handle)?;
                writeln!(handle, "## Priority")?;
                writeln!(handle, "- **Score:** {:.2}", p.priority_score)?;
                writeln!(handle, "- **Risk:** {}", p.risk_narrative)?;
                writeln!(handle, "- **Action:** {}", p.suggested_action)?;
            }
            writeln!(handle)?;
            writeln!(handle, "## Related commands")?;
            for cmd in &related_commands {
                writeln!(handle, "- `{cmd}`")?;
            }
        }
        Resolved::ForLlm | Resolved::Table => {
            writeln!(handle, "{} — {}", finding_type, target)?;
            writeln!(handle, "id:         {}", finding.id)?;
            writeln!(handle, "severity:   {severity}")?;
            writeln!(handle, "evidence:   {evidence}")?;
            if let Some(c) = &confidence {
                writeln!(handle, "confidence: {c}")?;
            }
            if let Some(impact) = &finding.impact {
                writeln!(handle, "impact:     {impact}")?;
            }
            if let Some(rec) = &finding.recommendation {
                writeln!(handle, "rec:        {rec}")?;
            }
            if let Some(p) = &priority {
                writeln!(handle, "score:      {:.2}", p.priority_score)?;
                writeln!(handle, "risk:       {}", p.risk_narrative)?;
                writeln!(handle, "action:     {}", p.suggested_action)?;
            }
            writeln!(handle)?;
            writeln!(handle, "Related commands:")?;
            for cmd in &related_commands {
                writeln!(handle, "  {cmd}")?;
            }
        }
    }
    Ok(())
}

fn build_related_commands(target: &str, finding_id: &str) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!("gridseak graph callers {target}"));
    out.push(format!("gridseak graph blast-radius {target}"));
    out.push(format!("gridseak graph callees {target}"));
    out.push("gridseak findings . --severity critical".to_string());
    out.push(format!("gridseak explain {finding_id}"));
    out
}

// ---------------------------------------------------------------------------
// `gridseak metrics`
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Clone)]
pub struct MetricsCmdArgs {
    #[arg(default_value = ".")]
    pub project: String,

    /// Print every category. Equivalent to omitting `--category`.
    #[arg(long, default_value_t = false)]
    pub all: bool,

    /// Filter to a specific metric category. May be passed multiple
    /// times to include several categories.
    #[arg(long, value_enum)]
    pub category: Vec<MetricCategoryArg>,

    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table)]
    pub format: ScanOutputFormat,

    #[arg(long, default_value_t = false)]
    pub for_llm: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "lower")]
pub enum MetricCategoryArg {
    Graph,
    Function,
    Module,
    Temporal,
    Classification,
    Composite,
}

impl MetricCategoryArg {
    fn to_kind(self) -> MetricCategoryKind {
        match self {
            Self::Graph => MetricCategoryKind::Graph,
            Self::Function => MetricCategoryKind::Function,
            Self::Module => MetricCategoryKind::Module,
            Self::Temporal => MetricCategoryKind::Temporal,
            Self::Classification => MetricCategoryKind::Classification,
            Self::Composite => MetricCategoryKind::Composite,
        }
    }
}

pub fn run_metrics(store: &ProjectStore, args: MetricsCmdArgs, global_json: bool) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let scan = project
        .latest_scan
        .clone()
        .context("project has no scans — run `gridseak scan .` first")?;
    let report: HealthReport = serde_json::from_value(store.load_report(&scan.id)?)?;

    let categories: Option<Vec<MetricCategoryKind>> = if args.all || args.category.is_empty() {
        None
    } else {
        Some(args.category.iter().map(|c| c.to_kind()).collect())
    };

    let view = metrics_render::build_view(
        project.display_name.clone(),
        scan.id.clone(),
        &report,
        categories.as_deref(),
    );
    let format = match resolve_format(args.format, args.for_llm, global_json) {
        Resolved::Table => metrics_render::MetricsFormat::Table {
            layout: width::detect(),
        },
        Resolved::Markdown => metrics_render::MetricsFormat::Markdown,
        Resolved::Json => metrics_render::MetricsFormat::Json,
        Resolved::ForLlm => metrics_render::MetricsFormat::ForLlm,
    };
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    metrics_render::render(&format, &view, &mut handle)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `gridseak report`
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Clone)]
pub struct ReportArgs {
    #[arg(default_value = ".")]
    pub project: String,

    /// Pin to a specific scan id instead of the latest.
    #[arg(long)]
    pub scan_id: Option<String>,

    /// Dump the entire raw `HealthReport` JSON. Implies `--format
    /// json`.
    #[arg(long, default_value_t = false)]
    pub full: bool,

    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table)]
    pub format: ScanOutputFormat,

    #[arg(long, default_value_t = false)]
    pub for_llm: bool,

    #[arg(long)]
    pub budget: Option<usize>,
}

pub fn run_report(store: &ProjectStore, args: ReportArgs, global_json: bool) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let scan_id = match args.scan_id.clone() {
        Some(id) => id,
        None => project
            .latest_scan
            .as_ref()
            .map(|s| s.id.clone())
            .context("project has no scans — run `gridseak scan .` first")?,
    };

    let report_value = store.load_report(&scan_id)?;
    if args.full {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        serde_json::to_writer_pretty(&mut handle, &report_value)?;
        writeln!(handle)?;
        return Ok(());
    }

    let scan = store
        .list_scan_runs(&project.id)?
        .into_iter()
        .find(|s| s.id == scan_id)
        .with_context(|| format!("scan `{scan_id}` not in this project's history"))?;
    let report: HealthReport = serde_json::from_value(report_value)?;
    let view = ScanReportView::build(&report, &project, &scan);

    let format = match resolve_format(args.format, args.for_llm, global_json) {
        Resolved::Table => HeroFormat::Table {
            layout: width::detect(),
        },
        Resolved::Markdown => HeroFormat::Markdown,
        Resolved::Json => HeroFormat::Json,
        Resolved::ForLlm => HeroFormat::ForLlm {
            budget: args.budget,
        },
    };
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    render_hero(&format, &view, &mut handle)?;
    Ok(())
}
