mod analyze_command;
mod context_command;
mod doctor_command;
mod drilldown_command;
mod feedback_command;
mod graph_command;
mod graph_queries;
mod history_command;
mod intent_router;
mod mcp_preflight;
mod progress;
mod render;
mod route_command;
mod scan_command;
mod scan_manifest;
mod setup;
#[cfg(feature = "trace-internal")]
mod setup_trace;
mod workspace_delta;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use graphengine_analysis::health::report::HealthReport;
use graphengine_diagnostic::priority;
use gridseak_engine_runner::{run_pipeline as run_engine_pipeline, RunPipelineConfig};
use gridseak_local_store::{BeginScanRecord, GitContext, LocalStorePaths, ProjectStore};
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::analyze_command::{run_analyze_background, AnalyzeArgs};
use crate::context_command::{run_context, ContextArgs};
use crate::doctor_command::run_doctor;
use crate::drilldown_command::{
    run_explain, run_findings, run_metrics, run_recommendations, run_report, ExplainArgs,
    FindingsArgs, MetricsCmdArgs, RecommendationsArgs, ReportArgs,
};
use crate::feedback_command::{run_feedback, FeedbackArgs};
use crate::graph_command::{run_graph, GraphArgs};
use crate::history_command::{
    run_compare, run_scan_latest, run_scans_list, run_trends, CompareArgs, ScanLatestArgs,
    ScansListArgs, TrendsArgs,
};
use crate::intent_router::{route, routing_table, RouteInput, RoutedTool};
use crate::mcp_preflight::{
    check_analysis_complete, check_stale_snapshot, enrich_envelope, load_analysis_readiness,
    symbol_file_path, AnalysisNotReadyError, StaleSnapshotError,
};
use crate::progress::{CliProgressSink, ProgressMode};
use crate::route_command::{run_route, RouteArgs};
use crate::scan_command::{run_scan_now, ScanArgs};
use crate::scan_manifest::write_scan_manifest;
use crate::setup::{run as run_setup, SetupArgs};
#[cfg(feature = "trace-internal")]
use crate::setup_trace::{run_setup as run_setup_trace, SetupArgs as SetupTraceArgs};
use crate::workspace_delta::{compute_workspace_delta, normalize_rel_path};

const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".nuxt",
    ".cache",
    "vendor",
    "Pods",
    ".gradle",
];

// `after_help` is what clap prints below the standard `Usage:` / `Options:`
// / `Commands:` blocks. We use it to surface the three commands every first-
// time user needs, in the order they need them. Without this, clap renders
// the subcommand list alphabetically and `scan` is buried behind
// `compare`, `context`, `doctor`, `explain`, `feedback`, `findings`, ...
// — a wall of options for a tool whose first command is almost always
// `gridseak scan .`. The after_help text is the same thing the install
// script prints after `done.`, so the docs converge on a single mental
// model.
const HELP_AFTER: &str = "\
Quick start:
  gridseak scan .              First-run structural health report
  gridseak setup               Wire the MCP server into your IDE(s)
  gridseak setup --verify      Confirm MCP + rule + binary are wired

Then drilldown:
  gridseak recommendations     Top deterministic suggestions, ranked
  gridseak metrics             All metrics with confidence flags
  gridseak graph blast-radius \"<symbol>\"
                              Who breaks if you change this symbol
  gridseak compare --previous  Delta vs your previous scan
  gridseak context --for-llm   Compact agent-ready context bundle

Docs:      https://gridseak.com/cli
Feedback:  gridseak feedback \"<text>\"   (stored locally, never uploaded)
";

#[derive(Parser)]
#[command(
    name = "gridseak",
    about = "GridSeak local project memory, scan history, and MCP server",
    version = env!("CARGO_PKG_VERSION"),
    after_help = HELP_AFTER,
)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    /// Accepted for script readability. Output is JSON by default for agent safety.
    #[arg(long, global = true, default_value_t = false)]
    json: bool,
    /// Progress rendering mode on stderr. `auto` (default) picks `fancy`
    /// for a TTY without `CI=true` and `plain` everywhere else.
    ///
    /// - `auto`: detect at startup.
    /// - `fancy`: in-place ANSI rendering. Requires a TTY.
    /// - `plain`: line-oriented, safe for logs and CI.
    /// - `off` / `silent` / `none`: emit nothing.
    #[arg(
        long,
        global = true,
        default_value = "auto",
        value_parser = parse_progress_mode,
    )]
    progress: ProgressMode,
    /// Suppress all progress UI on stderr. Equivalent to
    /// `--progress off`. Errors still print.
    #[arg(long, global = true, default_value_t = false)]
    quiet: bool,
    #[command(subcommand)]
    command: Commands,
}

fn parse_progress_mode(s: &str) -> Result<ProgressMode, String> {
    ProgressMode::from_str(s)
}

#[derive(Subcommand)]
enum Commands {
    /// Show how agents and humans should use this CLI.
    Discover {
        #[arg(default_value = ".")]
        project: String,
    },
    /// Compact current health/status for a project or current folder.
    Status {
        #[arg(default_value = ".")]
        project: String,
    },
    Projects {
        #[command(subcommand)]
        command: ProjectsCommand,
    },
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    Scans {
        #[command(subcommand)]
        command: ScansCommand,
    },
    /// Compare two scans (defaults to the two most recent).
    Compare(CompareArgs),
    /// Show a time-series for a metric across scans.
    Trends(TrendsArgs),
    /// Top Fix-First recommendations against the latest scan.
    Recommendations(RecommendationsArgs),
    /// Full filtered finding stream against the latest scan.
    Findings(FindingsArgs),
    /// Plain-prose breakdown of a single finding by id.
    Explain(ExplainArgs),
    /// Categorised metric tables (graph / function / module / temporal / classification / composite).
    Metrics(MetricsCmdArgs),
    /// Re-render the hero report from a persisted scan. `--full` dumps the raw HealthReport.
    Report(ReportArgs),
    /// Lightweight graph queries over the persisted scan artifact.
    Graph(GraphArgs),
    /// LLM-native context bundle: scan summary + metrics + priorities + graph slice.
    Context(ContextArgs),
    /// Deterministic symptom → MCP tool routing (debug / non-MCP agents).
    Route(RouteArgs),
    /// Wire the GridSeak MCP server into your IDE(s). Auto-writes for
    /// Cursor + Windsurf, prints instructions for Claude Code + Codex.
    /// Also writes the Cursor rule file that teaches the agent when to
    /// call each of the thirteen GridSeak MCP tools.
    Setup(SetupArgs),
    /// (Internal, `--features trace-internal` only.) Wire the visual
    /// view's MCP — kept private until the visual view is ready to
    /// ship publicly.
    #[cfg(feature = "trace-internal")]
    SetupTrace(SetupTraceArgs),
    /// Append a free-form note to the local feedback table. Everything
    /// stays on this machine until you decide to share it.
    Feedback(FeedbackArgs),
    /// Run a fresh parse + analysis on an absolute or relative repo path.
    Scan(ScanArgs),
    /// Background or one-shot analysis against the latest scan artifact.
    Analyze(AnalyzeArgs),
    Export {
        #[command(subcommand)]
        command: ExportCommand,
    },
    Mcp,
    /// Verify install consistency: probe each sidecar binary
    /// (`graphengine-parsing`, `ge-analyze`) and confirm its reported
    /// version matches this CLI's. Use this when scans behave oddly
    /// after an install/upgrade; a mismatch usually means a stale
    /// sidecar got left behind.
    Doctor,
}

#[derive(Subcommand)]
enum ProjectsCommand {
    List,
}

#[derive(Subcommand)]
enum ProjectCommand {
    Add {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    Show {
        #[arg(default_value = ".")]
        project: String,
    },
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum ScansCommand {
    /// List scans (newest first) with summary metrics.
    List(ScansListArgs),
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum ScanCommand {
    /// Render the most recent persisted scan without re-running.
    Latest(ScanLatestArgs),
    Recommendations {
        #[arg(default_value = ".")]
        project: String,
        #[arg(long, default_value_t = priority::DEFAULT_TOP_N)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        deep: bool,
    },
    Compare {
        #[arg(default_value = ".")]
        project: String,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },
    Rescan {
        #[arg(default_value = ".")]
        project: String,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long, value_delimiter = ',')]
        languages: Vec<String>,
    },
    Metrics {
        #[arg(default_value = ".")]
        project: String,
    },
    Findings {
        #[arg(default_value = ".")]
        project: String,
        #[arg(long)]
        severity: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    Report {
        #[arg(default_value = ".")]
        project: String,
        #[arg(long)]
        scan_id: Option<String>,
        #[arg(long, default_value_t = false)]
        full: bool,
    },
    Artifacts {
        #[arg(default_value = ".")]
        project: String,
    },
}

#[derive(Subcommand)]
enum ExportCommand {
    AiSummary {
        #[arg(default_value = ".")]
        project: String,
        #[arg(long, value_enum, default_value_t = SummaryFormat::Markdown)]
        format: SummaryFormat,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum SummaryFormat {
    Markdown,
    Json,
}

// CLI entry point. Three jobs, in order:
//
//   1. Parse argv (`Cli::parse`) and resolve where the local store
//      lives (`--data-dir` override, otherwise the platform default).
//   2. Reconcile progress UI: `--quiet` is hard-stop and overrides
//      whatever `--progress` selected, so "quiet" actually means quiet.
//   3. Dispatch the chosen subcommand. Every arm is a thin router into
//      a `*_command` module — no scan, render, or graph logic lives in
//      this function. The runtime is `tokio` because three arms are
//      async (`Doctor` probes sidecars, `Scan` drives the engine
//      pipeline, `Mcp` serves the rmcp transport).
//
// `Commands::Mcp` is the only long-running branch: it parks on the
// rmcp transport until shutdown. Every other branch runs one-shot and
// returns. Output is JSON by default (`print_json`) so agents can
// parse stdout without a flag; the `--json` flag is accepted for
// script readability and threaded into the few human-format renderers
// (`run_scans_list`, `run_compare`, `run_metrics`, …) that flip on it.
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = match cli.data_dir {
        Some(dir) => LocalStorePaths::from_data_dir(dir)?,
        None => LocalStorePaths::resolve_default()?,
    };
    // Effective progress mode: `--quiet` overrides whatever the user
    // (or `auto`) selected because the user has explicitly asked for
    // silence. Without this, `--quiet --progress fancy` would still
    // animate, which is a surprising violation of "quiet means quiet".
    let progress_mode = if cli.quiet {
        ProgressMode::Silent
    } else {
        cli.progress
    };
    let store = paths.open_store()?;

    match cli.command {
        Commands::Discover { project } => print_json(&discover(&store, &paths, &project)?)?,
        Commands::Status { project } => print_json(&status(&store, &project)?)?,
        Commands::Projects {
            command: ProjectsCommand::List,
        } => print_json(&store.list_projects()?)?,
        Commands::Project {
            command: ProjectCommand::Add { path },
        } => print_json(&store.create_project_for_folder(&path)?)?,
        Commands::Project {
            command: ProjectCommand::Show { project },
        } => print_json(&store.project_detail(&store.resolve_project(&project)?.id)?)?,
        Commands::Project {
            command: ProjectCommand::Search { query, limit },
        } => print_json(&store.search_projects(&query, limit)?)?,
        Commands::Scans {
            command: ScansCommand::List(args),
        } => run_scans_list(&store, args, cli.json)?,
        Commands::Compare(args) => run_compare(&store, args, cli.json)?,
        Commands::Trends(args) => run_trends(&store, args, cli.json)?,
        Commands::Recommendations(args) => run_recommendations(&store, args, cli.json)?,
        Commands::Findings(args) => run_findings(&store, args, cli.json)?,
        Commands::Explain(args) => run_explain(&store, args, cli.json)?,
        Commands::Metrics(args) => run_metrics(&store, args, cli.json)?,
        Commands::Report(args) => run_report(&store, args, cli.json)?,
        Commands::Graph(args) => run_graph(&store, args, cli.json)?,
        Commands::Context(args) => run_context(&store, args, cli.json)?,
        Commands::Route(args) => run_route(args)?,
        Commands::Setup(args) => run_setup(args)?,
        #[cfg(feature = "trace-internal")]
        Commands::SetupTrace(args) => run_setup_trace(args)?,
        Commands::Feedback(args) => {
            run_feedback(&store, args, env!("CARGO_PKG_VERSION"), cli.json)?
        }
        Commands::Doctor => run_doctor(cli.json).await?,
        Commands::Analyze(args) => run_analyze_background(&store, &paths, args)?,
        Commands::Scan(args) => match args.sub.clone() {
            None => {
                run_scan_now(&store, &paths, args, progress_mode, cli.json).await?;
            }
            Some(command) => match command {
                ScanCommand::Latest(args) => run_scan_latest(&store, args, cli.json)?,
                ScanCommand::Recommendations {
                    project,
                    limit,
                    deep,
                } => {
                    let recommendations = recommendations(&store, &project, limit, deep)?;
                    print_json(&recommendations)?
                }
                ScanCommand::Compare { project, from, to } => {
                    let _ = store.resolve_project(&project)?;
                    print_json(&compare_reports(&store, &from, &to)?)?
                }
                ScanCommand::Rescan {
                    project,
                    lang,
                    languages,
                } => {
                    rescan_project(
                        &store,
                        &paths,
                        &project,
                        scan_language_request(lang, languages),
                        "cli",
                        progress_mode,
                        // Legacy `Scan::Rescan` subcommand doesn't
                        // expose `--no-incremental`; preserve the
                        // shipping default of incremental ON. Users
                        // who need the bypass can run
                        // `gridseak scan --no-incremental`.
                        true,
                        false,
                    )
                    .await?;
                }
                ScanCommand::Metrics { project } => print_json(&scan_metrics(&store, &project)?)?,
                ScanCommand::Findings {
                    project,
                    severity,
                    limit,
                } => print_json(&scan_findings(
                    &store,
                    &project,
                    severity.as_deref(),
                    limit,
                )?)?,
                ScanCommand::Report {
                    project,
                    scan_id,
                    full,
                } => print_json(&scan_report(&store, &project, scan_id.as_deref(), full)?)?,
                ScanCommand::Artifacts { project } => {
                    print_json(&scan_artifacts(&store, &project)?)?
                }
            },
        },
        Commands::Export {
            command: ExportCommand::AiSummary { project, format },
        } => {
            let project = store.resolve_project(&project)?;
            let latest = project.latest_scan.context("project has no scans")?;
            match format {
                SummaryFormat::Json => print_json(&store.load_ai_summary(&latest.id)?)?,
                SummaryFormat::Markdown => {
                    let path = latest
                        .ai_summary_md_path
                        .context("latest scan has no markdown AI summary")?;
                    println!("{}", std::fs::read_to_string(path)?);
                }
            }
        }
        Commands::Mcp => serve_mcp(store, paths).await?,
    }

    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) enum LanguageRequest {
    Auto,
    Explicit(Vec<String>),
}

pub(crate) fn scan_language_request(
    lang: Option<String>,
    languages: Vec<String>,
) -> LanguageRequest {
    let mut values = languages;
    if let Some(lang) = lang {
        values.push(lang);
    }
    values.retain(|v| !v.trim().is_empty());
    values.sort();
    values.dedup();
    if values.is_empty() {
        LanguageRequest::Auto
    } else {
        LanguageRequest::Explicit(values)
    }
}

fn discover(
    store: &ProjectStore,
    paths: &LocalStorePaths,
    project_ref: &str,
) -> Result<serde_json::Value> {
    let project = store.resolve_project_lenient(project_ref).ok();
    Ok(serde_json::json!({
        "purpose": "GridSeak exposes local architectural truth over time for humans and AI agents.",
        "data_dir": paths.data_dir,
        "default_project_resolution": "Project arguments accept id, display name, partial name, folder path, or '.' for current directory.",
        "recommended_agent_flow": [
            "gridseak status .",
            "gridseak scan recommendations . --limit 10",
            "gridseak scan findings . --severity critical",
            "gridseak scan rescan .",
            "gridseak scan compare . --from <old_scan_id> --to <new_scan_id>"
        ],
        "power_commands": [
            "gridseak projects list",
            "gridseak project search <query>",
            "gridseak scan report . --full",
            "gridseak scan artifacts .",
            "gridseak export ai-summary . --format json",
            "gridseak mcp"
        ],
        "current_project": project,
        "mcp_tools": [
            "gridseak_context_for_llm",
            "gridseak_route",
            "gridseak_status",
            "gridseak_scan",
            "gridseak_get_recommendations",
            "gridseak_explain_finding",
            "gridseak_get_findings",
            "gridseak_graph_blast_radius",
            "gridseak_graph_file_blast_radius",
            "gridseak_graph_callers",
            "gridseak_graph_callees",
            "gridseak_graph_slice",
            "gridseak_graph_module_coupling",
            "gridseak_graph_cycles"
        ]
    }))
}

fn status(store: &ProjectStore, project_ref: &str) -> Result<serde_json::Value> {
    let project = store.resolve_project_lenient(project_ref)?;
    let latest = project.latest_scan.clone();
    let readiness = load_analysis_readiness(store, &project);

    let (workspace_delta, incremental_plan, last_analysis_provenance) =
        status_incremental_context(store, &project, &latest);

    let workspace_line = workspace_delta.as_ref().map_or_else(
        || "workspace: no scan".to_string(),
        |delta| {
            format!(
                "workspace: {} files changed since scan ({} unstaged, {} staged, {} untracked)",
                delta.dirty_paths.len(),
                delta.buckets.unstaged.len(),
                delta.buckets.staged.len(),
                delta.buckets.untracked.len(),
            )
        },
    );

    Ok(serde_json::json!({
        "project": {
            "id": project.id,
            "display_name": project.display_name,
            "roots": project.roots,
            "scan_count": project.scan_count,
            "storage_mode": project.storage_mode,
            "sync_status": project.sync_status,
        },
        "latest_scan": latest,
        "latest_metrics": project.latest_metrics,
        "analysis_readiness": readiness,
        "workspace_summary": workspace_line,
        "workspace_delta": workspace_delta,
        "incremental_plan": incremental_plan,
        "last_analysis_provenance": last_analysis_provenance,
        "next_best_commands": [
            "gridseak scan recommendations . --limit 10",
            "gridseak scan findings . --limit 20",
            "gridseak scan rescan ."
        ]
    }))
}

fn status_incremental_context(
    store: &ProjectStore,
    project: &gridseak_local_store::ProjectDto,
    latest: &Option<gridseak_local_store::ScanRunDto>,
) -> (
    Option<crate::workspace_delta::WorkspaceDelta>,
    Option<graphengine_analysis::health::pipeline::IncrementalPlan>,
    Option<serde_json::Value>,
) {
    let Some(scan) = latest.as_ref() else {
        return (None, None, None);
    };

    let last_analysis_provenance = store
        .load_report(&scan.id)
        .ok()
        .and_then(|report| report.get("analysis_provenance").cloned());

    let Some(root) = project.roots.first() else {
        return (None, None, last_analysis_provenance);
    };
    let Some(completed_at) = scan.completed_at.as_deref() else {
        return (None, None, last_analysis_provenance);
    };

    let workspace_delta =
        compute_workspace_delta(std::path::Path::new(&root.path), completed_at, None).ok();

    let incremental_plan = scan.graph_artifact_path.as_ref().and_then(|artifact| {
        let conn = rusqlite::Connection::open_with_flags(
            artifact,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .ok()?;
        let dirty = workspace_delta.as_ref().map(|d| d.dirty_paths.as_slice());
        graphengine_analysis::health::pipeline::predict_incremental_plan(&conn, dirty).ok()
    });

    (workspace_delta, incremental_plan, last_analysis_provenance)
}

fn recommendations(
    store: &ProjectStore,
    project_ref: &str,
    limit: usize,
    deep: bool,
) -> Result<serde_json::Value> {
    let project = store.resolve_project_lenient(project_ref)?;
    let latest = project
        .latest_scan
        .clone()
        .context("project has no scans")?;
    let report = store.load_report(&latest.id)?;
    let health: HealthReport = serde_json::from_value(report)?;
    let priorities = priority::compute_priorities(&health, limit);
    Ok(serde_json::json!({
        "project": project,
        "scan": latest,
        "top_recommendations": priorities,
        "full_report_included": deep,
        "report": if deep { Some(serde_json::to_value(&health)?) } else { None::<serde_json::Value> },
    }))
}

fn scan_metrics(store: &ProjectStore, project_ref: &str) -> Result<serde_json::Value> {
    let project = store.resolve_project_lenient(project_ref)?;
    Ok(serde_json::json!({
        "project": project.display_name,
        "latest_scan": project.latest_scan,
        "latest_metrics": project.latest_metrics,
    }))
}

fn scan_findings(
    store: &ProjectStore,
    project_ref: &str,
    severity: Option<&str>,
    limit: usize,
) -> Result<serde_json::Value> {
    let project = store.resolve_project_lenient(project_ref)?;
    let latest = project
        .latest_scan
        .clone()
        .context("project has no scans")?;
    let report: HealthReport = serde_json::from_value(store.load_report(&latest.id)?)?;
    let mut findings = report.findings;
    if let Some(severity) = severity {
        let needle = severity.to_lowercase();
        findings.retain(|f| format!("{:?}", f.severity).to_lowercase() == needle);
    }
    findings.truncate(limit);
    Ok(serde_json::json!({
        "project": project.display_name,
        "scan": latest,
        "count": findings.len(),
        "findings": findings,
    }))
}

fn scan_report(
    store: &ProjectStore,
    project_ref: &str,
    scan_id: Option<&str>,
    full: bool,
) -> Result<serde_json::Value> {
    let project = store.resolve_project_lenient(project_ref)?;
    let scan = match scan_id {
        Some(id) => id.to_string(),
        None => {
            project
                .latest_scan
                .clone()
                .context("project has no scans")?
                .id
        }
    };
    let report = store.load_report(&scan)?;
    if full {
        Ok(report)
    } else {
        Ok(serde_json::json!({
            "project": project.display_name,
            "scan_id": scan,
            "health_score": report.get("health_score"),
            "summary": report.get("summary"),
            "metrics": report.get("metrics"),
            "finding_count": report.get("findings").and_then(|v| v.as_array()).map(|a| a.len()),
            "use_full_report": "gridseak scan report <project> --full"
        }))
    }
}

fn scan_artifacts(store: &ProjectStore, project_ref: &str) -> Result<serde_json::Value> {
    let project = store.resolve_project_lenient(project_ref)?;
    let latest = project.latest_scan.context("project has no scans")?;
    Ok(serde_json::json!({
        "scan_id": latest.id,
        "report_path": latest.report_path,
        "graph_artifact_path": latest.graph_artifact_path,
        "ai_summary_json_path": latest.ai_summary_json_path,
        "ai_summary_md_path": latest.ai_summary_md_path,
    }))
}

fn compare_reports(store: &ProjectStore, from: &str, to: &str) -> Result<serde_json::Value> {
    let a: HealthReport = serde_json::from_value(store.load_report(from)?)?;
    let b: HealthReport = serde_json::from_value(store.load_report(to)?)?;
    let score_a = a.health_score.unwrap_or(0) as i64;
    let score_b = b.health_score.unwrap_or(0) as i64;
    Ok(serde_json::json!({
        "from_scan_id": from,
        "to_scan_id": to,
        "score_before": score_a,
        "score_after": score_b,
        "score_delta": score_b - score_a,
        "findings_before": a.findings.len(),
        "findings_after": b.findings.len(),
        "summary_before": a.summary,
        "summary_after": b.summary,
    }))
}

// Coherent rescan-invocation bundle forwarded to the engine runner; a params
// struct shared with run_scan_pipeline is the tracked post-launch cleanup.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn rescan_project(
    store: &ProjectStore,
    paths: &LocalStorePaths,
    project_ref: &str,
    language_request: LanguageRequest,
    trigger: &str,
    progress_mode: ProgressMode,
    incremental: bool,
    full_analysis: bool,
) -> Result<serde_json::Value> {
    let project = store.resolve_project_lenient(project_ref)?;
    let root = project
        .roots
        .first()
        .context("project has no saved root folder")?;
    let root_path = PathBuf::from(&root.path);
    let languages = resolve_scan_languages(&project, &root_path, language_request)?;
    let primary_language = languages.first().cloned();

    let scan_id = Uuid::new_v4();
    let started_at = chrono::Utc::now();
    store.begin_scan(BeginScanRecord {
        scan_id,
        project_id: project.id.clone(),
        root_id: Some(root.id.clone()),
        started_at,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        engine_version: graphengine_analysis::VERSION.to_string(),
        primary_language,
        scan_languages: languages.clone(),
        git: GitContext::from_path(&root_path).unwrap_or_default(),
        scan_trigger: trigger.to_string(),
        requested_by: Some(if trigger == "mcp" { "agent" } else { "human" }.to_string()),
    })?;

    // S1-\u{03b5}: route the parse DB to a stable per-project cache
    // path so incremental scanning's `file_cache` table survives
    // between scans. The runner refuses to pass `--clear` when this
    // path is set, so language pass 2..N append to the same DB,
    // matching the legacy ephemeral semantics modulo persistence.
    let persistent_parse_db = paths.parse_db_for_project(&project.id);

    match run_scan_pipeline(
        &paths.scratch_dir,
        Some(persistent_parse_db),
        scan_id,
        &root_path,
        &languages,
        progress_mode,
        incremental,
        full_analysis,
    )
    .await
    {
        Ok(output) => {
            // A3: pull the analyzer's File-node-majority answer out of
            // the report and pass it through so `scan_runs.primary_language`
            // reflects what the repo actually is, not whatever the
            // pre-scan language detector guessed.
            let primary_language_override = output
                .report
                .get("primary_language")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            store.complete_scan(
                scan_id,
                &project.id,
                &output.report,
                &output.report_path,
                &output.graph_path,
                primary_language_override.as_deref(),
            )?;
            let completed_at = chrono::Utc::now().to_rfc3339();
            if let Err(err) = write_scan_manifest(
                &scan_id.to_string(),
                &completed_at,
                &root_path,
                &output.graph_path,
            ) {
                eprintln!("[gridseak] warning: scan_manifest write failed: {err}");
            }
            Ok(serde_json::json!({
                "status": "ready",
                "scan_id": scan_id,
                "project": store.get_project(&project.id)?,
            }))
        }
        Err(err) => {
            let _ = store.fail_scan(scan_id, &err.to_string());
            Err(err)
        }
    }
}

struct ScanOutput {
    report: serde_json::Value,
    report_path: PathBuf,
    graph_path: PathBuf,
}

fn resolve_scan_languages(
    project: &gridseak_local_store::ProjectDto,
    root: &Path,
    request: LanguageRequest,
) -> Result<Vec<String>> {
    match request {
        LanguageRequest::Explicit(languages) => Ok(languages),
        LanguageRequest::Auto => {
            if let Some(scan) = &project.latest_scan {
                if !scan.scan_languages.is_empty() {
                    return Ok(scan.scan_languages.clone());
                }
                if let Some(language) = &scan.primary_language {
                    return Ok(vec![language.clone()]);
                }
            }
            let detected = detect_languages(root)?;
            if detected.is_empty() {
                anyhow::bail!(
                    "could not detect a parseable language. Pass --lang <language> or --languages a,b"
                );
            }
            Ok(detected.into_iter().map(|hit| hit.language).collect())
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct LanguageHit {
    language: String,
    file_count: usize,
}

fn detect_languages(root: &Path) -> Result<Vec<LanguageHit>> {
    if let Some(configs_dir) = resolve_configs_dir() {
        let _ = graphengine_parsing::infrastructure::config::set_configs_dir_override(configs_dir);
    }
    let mut ext_to_lang = std::collections::HashMap::<String, String>::new();
    let mut discovery_only = std::collections::HashSet::<String>::new();
    for language in graphengine_parsing::infrastructure::config::get_available_languages()? {
        let descriptor =
            graphengine_parsing::infrastructure::config::load_language_descriptor(&language)?;
        if descriptor.discovery_only {
            discovery_only.insert(descriptor.language.clone());
        }
        for ext in descriptor.file_extensions {
            ext_to_lang
                .entry(ext.trim_start_matches('.').to_ascii_lowercase())
                .or_insert(descriptor.language.clone());
        }
    }

    let mut counts = std::collections::HashMap::<String, usize>::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| {
            !entry
                .file_name()
                .to_str()
                .map(|name| IGNORED_DIRS.contains(&name))
                .unwrap_or(false)
        })
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(ext) = entry.path().extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if let Some(language) = ext_to_lang.get(&ext.to_ascii_lowercase()) {
            if !discovery_only.contains(language) {
                *counts.entry(language.clone()).or_default() += 1;
            }
        }
    }

    let mut hits = counts
        .into_iter()
        .map(|(language, file_count)| LanguageHit {
            language,
            file_count,
        })
        .collect::<Vec<_>>();
    hits.sort_by(|a, b| {
        b.file_count
            .cmp(&a.file_count)
            .then(a.language.cmp(&b.language))
    });
    Ok(hits)
}

/// Drive a scan via the shared `gridseak-engine-runner` pipeline.
///
/// This is intentionally a thin wrapper: every metric, exclusion default,
/// progress event, and error variant is the runner's. The CLI only resolves
/// the parser/analyzer binaries and the configs directory, then forwards.
/// Both surfaces (this CLI and the desktop shell) call the same runner so a
/// scan of the same repo emits the same numbers regardless of who invoked it.
#[allow(clippy::too_many_arguments)]
async fn run_scan_pipeline(
    scratch_dir: &Path,
    persistent_parse_db: Option<PathBuf>,
    scan_id: Uuid,
    root: &Path,
    languages: &[String],
    progress_mode: ProgressMode,
    incremental: bool,
    full_analysis: bool,
) -> Result<ScanOutput> {
    if languages.is_empty() {
        anyhow::bail!("no parseable languages selected");
    }
    std::fs::create_dir_all(scratch_dir)
        .with_context(|| format!("create scratch dir {}", scratch_dir.display()))?;

    let parser_bin = resolve_engine_bin("graphengine-parsing")?;
    let analyzer_bin = resolve_engine_bin("ge-analyze")?;
    let configs_dir = resolve_configs_dir().context(
        "could not locate graphengine-parsing/configs/. \
         Set GRAPHENGINE_CONFIGS_DIR or run from inside the workspace.",
    )?;

    let git_dir = root.join(".git");
    let git_dir = git_dir.exists().then_some(git_dir);

    // Resolve `Auto` here (after we've parsed argv and know whether
    // stderr is a TTY). Concrete modes pass through unchanged.
    let mut sink = CliProgressSink::new(progress_mode.into_renderer());
    // We can't move `sink` into the config below and still call
    // `sink.finish()` afterward, so wrap it in an Arc/Mutex. But the
    // runner requires `Box<dyn ProgressSink>` (exclusive ownership for
    // the pipeline's lifetime), and `Mutex` would force a re-borrow
    // through `lock()` on every event — too much overhead in the hot
    // path. Pragmatic shape: hand the runner a "forwarder" sink that
    // owns a sender into a channel; we keep the renderer locally and
    // drain on the receiver side.
    //
    // Actually simpler — since the runner takes the sink by value and
    // we don't need to call `finish()` until *after* the pipeline
    // returns, we can run the pipeline first and skip the explicit
    // finish on the renderer (renderers are designed to leave the
    // terminal in a clean state at the next newline anyway). For the
    // fancy renderer that means the in-place line lingers briefly,
    // but the very next stderr line — typically the JSON result on
    // stdout or a follow-up CLI log — pushes past it. Stage 2 will
    // make this fully clean by routing the sink through a channel
    // and having the CLI driver call `finish()` explicitly.
    let _ = &mut sink; // silence unused-mut warning if any
    let cfg = RunPipelineConfig {
        root: root.to_path_buf(),
        languages: languages.to_vec(),
        parser_bin,
        analyzer_bin,
        configs_dir,
        scratch_dir: scratch_dir.to_path_buf(),
        persistent_parse_db,
        scan_id,
        exclude_tests: true,
        exclude_generated: true,
        incremental,
        full_analysis,
        git_dir,
        progress: Box::new(sink),
        cancel: None,
    };

    let output = run_engine_pipeline(cfg)
        .await
        .map_err(|e| anyhow::anyhow!("scan pipeline failed: {e}"))?;

    // Emit a final newline so the fancy in-place line cleanly hands
    // off to whatever the caller prints next. Cheap insurance; the
    // renderer modes that don't paint (plain after its last line
    // already includes a newline; silent which never wrote) ignore
    // the extra blank.
    eprintln!();

    let report_value = serde_json::to_value(&output.report).context("serialize health report")?;

    Ok(ScanOutput {
        report: report_value,
        report_path: output.report_path,
        graph_path: output.db_path,
    })
}

fn resolve_configs_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("GRAPHENGINE_CONFIGS_DIR") {
        return Some(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors() {
            for rel in [
                "configs",
                "resources/configs",
                "graphengine-parsing/configs",
            ] {
                let candidate = ancestor.join(rel);
                if candidate.join("apex.yaml").exists() {
                    return Some(candidate);
                }
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors() {
            let candidate = ancestor.join("graphengine-parsing").join("configs");
            if candidate.join("apex.yaml").exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Resolve a sibling engine binary (`graphengine-parsing`, `ge-analyze`).
///
/// Resolution order, in order of precedence:
/// 1. `<NAME>_BIN` env override (e.g. `GRAPHENGINE_PARSING_BIN`) — useful for
///    tests and for cargo-installed binaries on a non-standard path.
/// 2. Sibling of the current executable (matches release installs where the
///    CLI and the engine binaries are co-located).
/// 3. `target/{debug,release}/` along the ancestor chain of either the
///    current exe or the cwd (covers `cargo run -p gridseak-cli` and
///    `cargo build --workspace` dev workflows).
///
/// Returns a typed error with all probed paths attached when nothing matches,
/// so the user (or the polyglot test) sees exactly where we looked. We
/// intentionally do *not* fall back to in-process parsing: that fallback is
/// what lets the CLI silently diverge from desktop on multi-language repos.
fn resolve_engine_bin(name: &str) -> Result<PathBuf> {
    let executable = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let mut probed: Vec<PathBuf> = Vec::new();
    let env_var = format!("{}_BIN", name.replace('-', "_").to_ascii_uppercase());

    if let Some(path) = std::env::var_os(&env_var) {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Ok(candidate);
        }
        probed.push(candidate);
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors() {
            for rel in [
                executable.as_str(),
                &format!("binaries/{executable}"),
                &format!("resources/{executable}"),
                &format!("target/debug/{executable}"),
                &format!("target/release/{executable}"),
            ] {
                let candidate = ancestor.join(rel);
                if candidate.exists() {
                    return Ok(candidate);
                }
                probed.push(candidate);
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors() {
            for rel in [
                format!("target/debug/{executable}"),
                format!("target/release/{executable}"),
            ] {
                let candidate = ancestor.join(rel);
                if candidate.exists() {
                    return Ok(candidate);
                }
                probed.push(candidate);
            }
        }
    }

    let probed_summary = probed
        .iter()
        .take(8)
        .map(|p| format!("  - {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let extra = if probed.len() > 8 {
        format!("\n  …and {} more", probed.len() - 8)
    } else {
        String::new()
    };
    anyhow::bail!(
        "could not locate engine binary `{name}`. Build it with \
         `cargo build -p {name}` (or set ${env_var} to its path).\n\
         Searched:\n{probed_summary}{extra}"
    )
}

// -------------------------------------------------------------------------
// MCP server
// -------------------------------------------------------------------------

#[derive(Clone)]
struct GridSeakMcp {
    store: ProjectStore,
    paths: LocalStorePaths,
    tool_router: ToolRouter<Self>,
}

impl GridSeakMcp {
    fn new(store: ProjectStore, paths: LocalStorePaths) -> Self {
        Self {
            store,
            paths,
            tool_router: Self::tool_router(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProjectRef {
    #[serde(default = "default_project_ref")]
    project: String,
}

/// Default value for the `project` parameter shared by every MCP
/// tool. Resolved through `ProjectStore::resolve_project_lenient`,
/// so callers can safely pass `"."`, `"./"`, `""`, or omit the field
/// entirely — the store will fall back to the cwd's project and
/// then to the most recently completed scan in the local store
/// before erroring.
fn default_project_ref() -> String {
    ".".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RecommendationsParams {
    #[serde(default = "default_project_ref")]
    project: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    deep: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FindingsParams {
    #[serde(default = "default_project_ref")]
    project: String,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// Stage 8: parameter structs for the new symmetry tools (graph + context +
// explain + scan wrapper).
//
// Each one mirrors a CLI driver in shape so an agent that has seen the CLI
// spec can call MCP with the same arguments. Defaults match the CLI defaults
// where possible so a tool-call without arguments behaves the same as the
// equivalent terminal invocation.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct GraphSymbolParams {
    #[serde(default = "default_project_ref")]
    project: String,
    /// Symbol to resolve. Exact FQN preferred; suffix/substring also works.
    symbol: String,
    /// Cap on returned rows; default 20.
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GraphBlastRadiusParams {
    #[serde(default = "default_project_ref")]
    project: String,
    symbol: String,
    #[serde(default)]
    depth: Option<usize>,
    #[serde(default)]
    cap: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GraphFileBlastRadiusParams {
    #[serde(default = "default_project_ref")]
    project: String,
    /// Repo-relative path of the file whose upstream blast radius
    /// you want, e.g. `gridseak-cli/src/main.rs`. Must match the
    /// path the parser recorded — typically POSIX-style and rooted
    /// at the project's primary repo root. Absolute paths are
    /// accepted but stripped of the project-root prefix before
    /// matching.
    file: String,
    #[serde(default)]
    depth: Option<usize>,
    /// Per-seed BFS cap. The total number of unique rows may be up
    /// to `cap * number_of_symbols_in_file` if there's no overlap,
    /// but in practice overlap is high and the result stays small.
    #[serde(default)]
    cap: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GraphLimitParams {
    #[serde(default = "default_project_ref")]
    project: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GraphCyclesParams {
    #[serde(default = "default_project_ref")]
    project: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    max_depth: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ContextForLlmParams {
    #[serde(default = "default_project_ref")]
    project: String,
    #[serde(default)]
    budget: Option<usize>,
    /// One of: `hotspots`, `coupling`, `cycles`, `deadcode`.
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    finding: Option<String>,
    #[serde(default)]
    changed_files: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExplainParams {
    #[serde(default = "default_project_ref")]
    project: String,
    finding_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ScanRunParams {
    /// Repository root to scan. Path that the local store will canonicalise.
    /// Required so an agent never accidentally scans its own cwd.
    path: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    languages: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RouteParams {
    /// Plain-language user question to route deterministically.
    question: String,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default = "default_project_ref")]
    project: String,
}

// ---------------------------------------------------------------------------
// MCP tool surface — agent-first, deterministic-local, 0 LLM tokens.
//
// Fourteen tools. Slimmed from ~26 by dropping legacy aliases that confused
// Cursor's planner (two names for the same thing → planner picks the wrong
// one half the time). Each kept tool's description is symptom-led: it tells
// the agent *when to call it* in the user's own words, not just what it
// returns. Every response is wrapped by `wrap_response()` so the envelope
// carries `evidence: "deterministic_local_analysis"` and a `tier_legend` —
// these are the bait that gets Cursor's token-cost-aware planner to prefer
// us over re-grepping the repo.
//
// Ordering below matches the cold-start flow:
//   1. context_for_llm     — first call on a new conversation (one-shot bundle)
//   2. status              — cheap probe if you only need health + counts
//   3. scan                — only when no scan exists; otherwise re-use
//   4. get_recommendations — "what should we refactor first?"
//   5. explain_finding     — drill into a priority by finding_id
//   6. get_findings        — raw unranked list, filterable by severity
//   7. graph_blast_radius  — "if I change X, what breaks?"
//   8. graph_callers       — "who calls X?"
//   9. graph_callees       — "what does X call?"
//   10. graph_slice        — full upstream+downstream neighborhood
//   11. graph_module_coupling — top tightly-coupled module pairs
//   12. graph_cycles       — call-graph cycles (a non-empty result is a smell)
// ---------------------------------------------------------------------------

#[tool_router]
impl GridSeakMcp {
    #[tool(
        description = "FIRST CALL on any new conversation about this codebase. One-shot bundle: project summary, metrics, top priorities, confidence caveats, artifact paths, and a `next_tool` array telling you what to call next. Replaces 4-5 separate tool calls. Deterministic, local, 0 LLM tokens. Trigger phrases: any first structural question in a fresh chat ('what's in this repo', 'help me understand this codebase', 'where should I start')."
    )]
    async fn gridseak_context_for_llm(
        &self,
        Parameters(params): Parameters<ContextForLlmParams>,
    ) -> Result<CallToolResult, McpError> {
        let view = context_for_llm_envelope(&self.store, &params).map_err(mcp_err)?;
        ok_json(&view)
    }

    #[tool(
        description = "Deterministic symptom router — maps plain-language questions to the correct GridSeak MCP tool with preconditions. Use when unsure which structural tool to call. Trigger phrases: any ambiguous structural question before committing to grep or read_file. Returns recommended_tool, preconditions (rescan_if_dirty, analysis_complete), and next_tool hints. Does NOT execute the recommended tool; call it separately."
    )]
    async fn gridseak_route(
        &self,
        Parameters(params): Parameters<RouteParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = self
            .store
            .resolve_project_lenient(&params.project)
            .map_err(mcp_err)?;
        let decision = route(RouteInput {
            question: &params.question,
            file_hint: params.file.as_deref(),
            symbol_hint: params.symbol.as_deref(),
        });
        let readiness = load_analysis_readiness(&self.store, &project);
        let inner = serde_json::json!({
            "question": params.question,
            "recommended_tool": decision.tool.mcp_name(),
            "matched_symptom": decision.matched_symptom,
            "preconditions": decision.preconditions,
            "routing_table": routing_table_payload(),
            "analysis_readiness": readiness,
            "recommended_first_call": recommended_first_call(&self.store, &project),
        });
        let ctx = McpEnvelopeContext {
            store: &self.store,
            project: &project,
            query_paths: params.file.map(|f| vec![normalize_rel_path(&f)]),
            routing_hint: Some(decision.tool),
        };
        ok_json(&wrap_response_ctx(
            inner,
            &[decision.tool.mcp_name()],
            Some(ctx),
        ))
    }

    #[tool(
        description = "Cheap probe: returns project health summary, latest scan metadata, and `next_tool` hints. Deterministic, local, 0 LLM tokens. Use when you only need a quick health check (not the full bundle from gridseak_context_for_llm). Trigger phrases: 'is this codebase healthy', 'when did we last scan', 'how big is this'."
    )]
    async fn gridseak_status(
        &self,
        Parameters(params): Parameters<ProjectRef>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(&wrap_response(
            status(&self.store, &params.project).map_err(mcp_err)?,
            &[
                "gridseak_get_recommendations",
                "gridseak_context_for_llm",
                "gridseak_scan",
            ],
        ))
    }

    #[tool(
        description = "Run a fresh parse + analysis on an absolute or relative repo path. Returns scan_id and full project state. Deterministic, local, 0 LLM tokens. Call ONLY when gridseak_status reports no recent scan; otherwise the existing scan is fresh enough. Trigger phrases: 'analyze this repo', 'scan this codebase', or when status shows no scans. `path` resolves relative to the agent's cwd; pass an absolute path to be safe."
    )]
    async fn gridseak_scan(
        &self,
        Parameters(params): Parameters<ScanRunParams>,
    ) -> Result<CallToolResult, McpError> {
        let path = PathBuf::from(&params.path)
            .canonicalize()
            .map_err(|e| mcp_err(format!("path does not exist: {} ({e})", params.path)))?;
        let project = self
            .store
            .create_project_for_folder(&path)
            .map_err(mcp_err)?;
        let outcome = rescan_project(
            &self.store,
            &self.paths,
            &project.id,
            scan_language_request(params.language, params.languages),
            "mcp",
            ProgressMode::Silent,
            // MCP-triggered scans run silently and trust the
            // incremental default. An agent that suspects cache
            // staleness can invoke `gridseak scan --no-incremental`
            // out-of-band; the MCP surface stays minimal.
            true,
            false,
        )
        .await
        .map_err(mcp_err)?;
        ok_json(&wrap_response(
            outcome,
            &[
                "gridseak_context_for_llm",
                "gridseak_get_recommendations",
                "gridseak_get_findings",
            ],
        ))
    }

    #[tool(
        description = "Use when the user asks 'what should we refactor first', 'where's risky', 'what's tightly coupled', or any prioritization question. Returns ranked deterministic priorities with `finding_id` (pass to gridseak_explain_finding for narrative + suggested action). Deterministic, local, 0 LLM tokens. Every priority carries a confidence caveat — quote it verbatim. Do not synthesise priorities yourself; the deterministic ranking is the value."
    )]
    async fn gridseak_get_recommendations(
        &self,
        Parameters(params): Parameters<RecommendationsParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = self
            .store
            .resolve_project_lenient(&params.project)
            .map_err(mcp_err)?;
        let readiness = load_analysis_readiness(&self.store, &project);
        check_analysis_complete(&readiness).map_err(analysis_incomplete_mcp_err)?;
        let inner = recommendations(
            &self.store,
            &params.project,
            params.limit.unwrap_or(priority::DEFAULT_TOP_N),
            params.deep.unwrap_or(false),
        )
        .map_err(mcp_err)?;
        let ctx = McpEnvelopeContext {
            store: &self.store,
            project: &project,
            query_paths: None,
            routing_hint: Some(RoutedTool::GridseakExplainFinding),
        };
        ok_json(&wrap_response_ctx(
            inner,
            &["gridseak_explain_finding", "gridseak_graph_blast_radius"],
            Some(ctx),
        ))
    }

    #[tool(
        description = "Use AFTER gridseak_get_recommendations to drill into a specific priority by `finding_id`. Returns the finding's severity, evidence, recommended action, and the affected symbols. Deterministic, local, 0 LLM tokens. Trigger phrases: 'why does that matter', 'what should I do about that', or when the user names a specific finding."
    )]
    async fn gridseak_explain_finding(
        &self,
        Parameters(params): Parameters<ExplainParams>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(
            &explain_finding_envelope(&self.store, &params.project, &params.finding_id)
                .map_err(mcp_err)?,
        )
    }

    #[tool(
        description = "Use when the user asks 'is there dead code?', 'show me all the dead code', \
                       'list every X-type finding', 'what are all the cycles?', 'show me every \
                       finding of severity Y', 'enumerate dead/risky/orphaned functions', or \
                       wants a complete (not ranked) list of structural findings. Returns the \
                       raw findings array from the latest scan as deterministic facts — DO NOT \
                       reach for grep/awk/shell to enumerate these; this tool is the source of \
                       truth and is 0 LLM tokens. \
                       \n\nParameters:\n  - `project`: project ref (path, name, or omit to use \
                       the current cwd's latest scan).\n  - `severity` (optional): filter by \
                       severity. Accepts `critical`, `high`, `warning`, `info` (case-insensitive). \
                       Omit to return all severities.\n  - `limit` (optional): max rows returned \
                       (default 20). Pass a higher value for an exhaustive enumeration, e.g. \
                       `limit: 1000` when the user asks 'show ALL dead code'. \
                       \n\nLower-level than `gridseak_get_recommendations` (which ranks and \
                       deduplicates); prefer `get_recommendations` when the user wants the \
                       Fix-First view, prefer `get_findings` when the user wants the \
                       comprehensive list or a typed enumeration. Each finding has a \
                       `finding_id` — pass it to `gridseak_explain_finding` for the narrative \
                       and suggested action."
    )]
    async fn gridseak_get_findings(
        &self,
        Parameters(params): Parameters<FindingsParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = self
            .store
            .resolve_project_lenient(&params.project)
            .map_err(mcp_err)?;
        let readiness = load_analysis_readiness(&self.store, &project);
        check_analysis_complete(&readiness).map_err(analysis_incomplete_mcp_err)?;
        let inner = scan_findings(
            &self.store,
            &params.project,
            params.severity.as_deref(),
            params.limit.unwrap_or(20),
        )
        .map_err(mcp_err)?;
        let ctx = McpEnvelopeContext {
            store: &self.store,
            project: &project,
            query_paths: None,
            routing_hint: Some(RoutedTool::GridseakExplainFinding),
        };
        ok_json(&wrap_response_ctx(
            inner,
            &["gridseak_explain_finding", "gridseak_get_recommendations"],
            Some(ctx),
        ))
    }

    #[tool(
        description = "Use when the user asks 'if I change X, what breaks?', 'what's the impact of refactoring X?', 'is X safe to remove?', or 'what depends on X?'. Returns the transitive **upstream** callers of a function symbol — reverse BFS through Call edges with depth cap (default 3). Deterministic, local, 0 LLM tokens. Symbol must be a function/method FQN (e.g. `health::graph::validate_schema`) — file paths are rejected with guidance, and module/file nodes return a clear error pointing you at `gridseak_graph_callers` on a specific symbol. Each result carries the evidence tier of the call edge that linked it in; Tier 0/3 hits are 'definitely affected', Tier 1 (grep heuristic) hits are 'possibly affected' and must be reported as such. Symbol resolution: pass a fully qualified path when possible — bare generic names (`load`, `init`, `new`, `get`, …) are refused with `OverlyGenericSymbol` because a substring scan would silently include unrelated functions from other modules. If the symbol DID resolve via a substring scan, the response carries a `resolution_warning` block that you must relay to the user verbatim before quoting any downstream finding. For the opposite direction ('what does X reach when it runs') use `gridseak_graph_slice` or `gridseak_graph_callees`. For **file-level** 'if I change file X, what breaks?', call `gridseak_graph_file_blast_radius` — one tool call replaces a 50-symbol fanout."
    )]
    async fn gridseak_graph_blast_radius(
        &self,
        Parameters(params): Parameters<GraphBlastRadiusParams>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(&graph_tool_blast_radius(&self.store, &params)?)
    }

    #[tool(
        description = "Use when the user asks 'if I change FILE X, what breaks?', 'what's the \
                       impact of refactoring this file?', 'who depends on this file?', or \
                       names a path (e.g. `gridseak-cli/src/main.rs`) instead of a symbol. \
                       Aggregates the upstream blast radius of EVERY callable symbol in the \
                       file into a single response with per-symbol attribution. \
                       \n\nDeterministic, local, 0 LLM tokens. **One MCP call replaces the \
                       50-symbol fanout pattern** an agent would otherwise have to perform \
                       by reading the file, enumerating its public surface, and calling \
                       `gridseak_graph_blast_radius` per-symbol — at agent-grade UI cost. \
                       \n\nParameters:\n  - `project`: project ref (path / name / omit).\n  \
                       - `file`: repo-relative path of the file. POSIX-style slashes, must \
                       match the path the parser recorded at scan time. Absolute paths are \
                       accepted but normalised.\n  - `depth` (optional, default 3): per-seed \
                       BFS depth cap.\n  - `cap` (optional, default 200): per-seed row cap. \
                       \n\nResponse shape:\n  - `file_path`: echoed back.\n  - `seeds`: \
                       every Function/Method node found in this file.\n  - `rows`: external \
                       upstream callers, one row per unique node, each with `via_seeds` \
                       naming which file symbols reach it and `edge_evidence_tier` carrying \
                       the best (highest-trust) tier across all paths that reach the row — \
                       `tier_3` (LSP-verified), `tier_0` (tree-sitter), `tier_1` (grep \
                       heuristic, may be noisy), or absent for legacy edges. Sorted by depth \
                       then FQN.\n  - `cap_hit`: true if the BFS was truncated.\n\nWhen \
                       `seeds` is empty, \
                       the file path didn't match anything — check the path or rerun \
                       `gridseak_scan` if the file is new since the last scan."
    )]
    async fn gridseak_graph_file_blast_radius(
        &self,
        Parameters(params): Parameters<GraphFileBlastRadiusParams>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(&graph_tool_file_blast_radius(&self.store, &params)?)
    }

    #[tool(
        description = "Use when the user asks 'who calls X?', 'who uses X?', or wants the immediate (one-hop) upstream callers of a function. Direct callers from the deterministic call graph. Deterministic, local, 0 LLM tokens. Edges are tier-tagged — quote the tier when stating a caller relationship. For *transitive* upstream callers (the full 'what breaks if X changes' answer), call `gridseak_graph_blast_radius` instead — same direction, BFS-extended."
    )]
    async fn gridseak_graph_callers(
        &self,
        Parameters(params): Parameters<GraphSymbolParams>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(&graph_tool_callers(&self.store, &params)?)
    }

    #[tool(
        description = "Use when the user asks 'what does X call?', 'what does X use?', or wants the immediate (one-hop) downstream callees of a function. Direct callees from the deterministic call graph. Deterministic, local, 0 LLM tokens. Tier-tagged edges. For *transitive* downstream reach ('everything X transitively touches'), use `gridseak_graph_slice` and filter on `direction = downstream` — NOT `gridseak_graph_blast_radius`, which walks the opposite (upstream/caller) direction."
    )]
    async fn gridseak_graph_callees(
        &self,
        Parameters(params): Parameters<GraphSymbolParams>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(&graph_tool_callees(&self.store, &params)?)
    }

    #[tool(
        description = "Use when the user wants the FULL neighborhood around a symbol — both upstream callers AND downstream callees, within `depth` hops (default 2). Heavier than callers/callees alone. Deterministic, local, 0 LLM tokens. Trigger phrases: 'show me everything connected to X', 'how does X fit into the codebase', 'I want to understand X end-to-end'."
    )]
    async fn gridseak_graph_slice(
        &self,
        Parameters(params): Parameters<GraphBlastRadiusParams>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(&graph_tool_slice(&self.store, &params)?)
    }

    #[tool(
        description = "Use when the user asks 'what modules are tightly coupled', 'where's the cross-module mess', 'where are the module boundaries weakest'. Top module pairs ranked by call-edge count. Deterministic, local, 0 LLM tokens. Pair with gridseak_get_findings (filter by severity) for concrete action items."
    )]
    async fn gridseak_graph_module_coupling(
        &self,
        Parameters(params): Parameters<GraphLimitParams>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(&graph_tool_module_coupling(&self.store, &params)?)
    }

    #[tool(
        description = "Use when the user asks 'are there cycles', 'is this codebase well-layered', or any dependency-direction question. Simple call-graph cycles, depth-bounded (default 8). Deterministic, local, 0 LLM tokens. A non-empty result is a layering smell to surface to the user immediately."
    )]
    async fn gridseak_graph_cycles(
        &self,
        Parameters(params): Parameters<GraphCyclesParams>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(&graph_tool_cycles(&self.store, &params)?)
    }
}

#[tool_handler]
impl ServerHandler for GridSeakMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "GridSeak — deterministic structural-knowledge layer for this repo. \
                 Fourteen tools, all local, all 0 LLM tokens. Start with \
                 `gridseak_context_for_llm` on any new conversation. Every response \
                 carries an `evidence: deterministic_local_analysis` marker and a \
                 `tier_legend` — quote the tier (0 tree-sitter, 1 grep, 3 LSP) when \
                 you state a structural fact, and never flatten tiers into a vague \
                 'GridSeak says…' attribution."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

async fn serve_mcp(store: ProjectStore, paths: LocalStorePaths) -> Result<()> {
    let service = GridSeakMcp::new(store, paths)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

fn mcp_err(error: impl std::fmt::Display) -> McpError {
    McpError {
        code: ErrorCode::INTERNAL_ERROR,
        message: error.to_string().into(),
        data: None,
    }
}

fn ok_json<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string_pretty(value).map_err(mcp_err)?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

// ---------------------------------------------------------------------------
// MCP response envelope.
//
// All graph + analysis helpers go through `crate::graph_queries` so the MCP
// path stays byte-identical with the CLI path. The MCP envelope adds three
// pieces of agent-facing metadata the CLI doesn't need:
//
//   - `evidence: "deterministic_local_analysis"` — the planner-bait. Cursor's
//     planner reads this field as "this tool is cheap and trustworthy"
//     and prefers it over re-grepping the repo.
//   - `tier_legend` — every graph edge or recommendation carries a tier (0
//     tree-sitter, 1 grep, 3 LSP). The legend teaches the agent to quote the
//     tier when stating a structural fact. Without this, agents flatten the
//     tiers and over-state grep-heuristic edges as if they were LSP-verified.
//   - `next_tool` — the suggested follow-up so the agent doesn't burn a
//     planning round-trip on discovery.
//
// Every kept tool emits this envelope. The single point of definition keeps
// the wire format consistent so the agent's prompt has stable structure.
// ---------------------------------------------------------------------------

struct McpEnvelopeContext<'a> {
    store: &'a ProjectStore,
    project: &'a gridseak_local_store::ProjectDto,
    query_paths: Option<Vec<String>>,
    routing_hint: Option<RoutedTool>,
}

fn wrap_response(value: serde_json::Value, next: &[&'static str]) -> serde_json::Value {
    wrap_response_ctx(value, next, None)
}

fn wrap_response_ctx(
    value: serde_json::Value,
    next: &[&'static str],
    ctx: Option<McpEnvelopeContext<'_>>,
) -> serde_json::Value {
    // Q7: every envelope carries scan provenance when we can derive it
    // from the inner value. `scan_age_seconds` is the load-bearing
    // datum — the cursor rule (`gridseak.mdc`) requires the agent to
    // surface it in a one-line preamble before any structural claim.
    // `scan_root_mtime` lets the agent detect "files changed since
    // the scan" without us shipping incremental scanning yet (S1).
    let provenance = extract_scan_provenance(&value);
    let mut envelope = serde_json::json!({
        "result": value,
        "next_tool": next,
        "evidence": "deterministic_local_analysis",
        "tier_legend": tier_legend(),
    });
    if let Some(prov) = provenance {
        envelope
            .as_object_mut()
            .expect("wrap_response envelope is always an object")
            .insert("scan_provenance".into(), prov);
    }
    if let Some(ctx) = ctx {
        envelope = enrich_envelope(
            envelope,
            ctx.project,
            ctx.store,
            ctx.query_paths.as_deref(),
            ctx.routing_hint,
        );
    }
    envelope
}

fn stale_snapshot_mcp_err(err: StaleSnapshotError) -> McpError {
    let body = serde_json::to_string_pretty(&err).unwrap_or_else(|_| err.message.clone());
    McpError {
        code: ErrorCode::INVALID_PARAMS,
        message: body.into(),
        data: None,
    }
}

fn analysis_incomplete_mcp_err(err: AnalysisNotReadyError) -> McpError {
    let body = serde_json::to_string_pretty(&err).unwrap_or_else(|_| err.message.clone());
    McpError {
        code: ErrorCode::INVALID_PARAMS,
        message: body.into(),
        data: None,
    }
}

/// Pull whatever scan-provenance information we can find from the
/// inner value of an MCP response, without re-querying the store.
/// Conventions: helpers like [`status`], [`scan_findings`], etc.
/// already embed the latest scan record at well-known paths
/// (`scan.completed_at`, `latest_scan.completed_at`, etc.) and the
/// project record at `project.roots[0].path`. We scrape both, parse
/// timestamps once, and emit a tiny provenance block — agents are
/// instructed by the cursor rule to surface this verbatim.
///
/// Returns `None` when the inner value doesn't carry a scan
/// timestamp (e.g. a freshly-failed scan envelope or an empty store
/// probe). Returning `None` is preferred over a half-populated block
/// — agents handle "no provenance available" better than a partial
/// one.
fn extract_scan_provenance(value: &serde_json::Value) -> Option<serde_json::Value> {
    use chrono::{DateTime, Utc};

    fn rfc3339(value: &serde_json::Value, path: &str) -> Option<DateTime<Utc>> {
        let raw = value.pointer(path)?.as_str()?;
        DateTime::parse_from_rfc3339(raw)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    }
    fn str_at(value: &serde_json::Value, path: &str) -> Option<String> {
        value
            .pointer(path)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    // Scan timestamp: prefer `completed_at` (the moment the scan
    // result became durable) over `started_at` (only relevant for
    // in-flight scans).
    let scan_ts = rfc3339(value, "/scan/completed_at")
        .or_else(|| rfc3339(value, "/scan/started_at"))
        .or_else(|| rfc3339(value, "/result/scan/completed_at"))
        .or_else(|| rfc3339(value, "/result/scan/started_at"))
        .or_else(|| rfc3339(value, "/latest_scan/completed_at"))
        .or_else(|| rfc3339(value, "/latest_scan/started_at"))
        .or_else(|| rfc3339(value, "/project/latest_scan/completed_at"))
        .or_else(|| rfc3339(value, "/project/latest_scan/started_at"));

    let scan_id = str_at(value, "/scan/id")
        .or_else(|| str_at(value, "/scan_id"))
        .or_else(|| str_at(value, "/latest_scan/id"))
        .or_else(|| str_at(value, "/project/latest_scan/id"));

    let scan_ts = scan_ts?;

    let scan_age_seconds = (Utc::now() - scan_ts).num_seconds().max(0);

    // Root path: stat the scanned root so the agent can warn the
    // user when the filesystem looks newer than the scan. We only
    // emit this when the path resolves AND we can read its metadata
    // — both checks are cheap (one stat) and silent failure keeps
    // the envelope honest.
    let root_path = str_at(value, "/project/roots/0/path")
        .or_else(|| str_at(value, "/result/project/roots/0/path"))
        .or_else(|| str_at(value, "/scan/root_path"));
    let scan_root_mtime = root_path.as_ref().and_then(|p| {
        let meta = std::fs::metadata(p).ok()?;
        let modified = meta.modified().ok()?;
        let mtime_chrono: DateTime<Utc> = modified.into();
        Some(mtime_chrono.to_rfc3339())
    });

    let mut block = serde_json::json!({
        "scan_id": scan_id,
        "scan_completed_at": scan_ts.to_rfc3339(),
        "scan_age_seconds": scan_age_seconds,
        "agent_directive": "Surface scan_age_seconds (rendered in human units like '12 min') in a one-line preamble before any structural claim. If scan_age_seconds is large or scan_root_mtime is newer than the scan, recommend re-running gridseak_scan before trusting derived facts.",
    });
    if let Some(mtime) = scan_root_mtime {
        block
            .as_object_mut()
            .expect("provenance block is always an object")
            .insert("scan_root_mtime".into(), serde_json::Value::String(mtime));
    }
    Some(block)
}

fn tier_legend() -> serde_json::Value {
    serde_json::json!({
        "tier_0": "tree-sitter parsed import / call site (deterministic, fast)",
        "tier_1": "filtered grep heuristic (may include false positives)",
        "tier_3": "LSP-verified via language server (deterministic, slower)",
        "agent_directive": "When you state a structural fact derived from this response, name the tier you're quoting. Quote `confidence_caveats` verbatim. Do not flatten tiers into 'GridSeak says…'.",
    })
}

fn resolve_project_and_graph(
    store: &ProjectStore,
    project_ref: &str,
) -> Result<(gridseak_local_store::ProjectDto, String, String), McpError> {
    let project = store
        .resolve_project_lenient(project_ref)
        .map_err(mcp_err)?;
    let scan = project
        .latest_scan
        .clone()
        .ok_or_else(|| mcp_err("project has no scans"))?;
    let artifact = scan
        .graph_artifact_path
        .clone()
        .ok_or_else(|| mcp_err("latest scan has no graph artifact recorded"))?;
    Ok((project, scan.id, artifact))
}

fn graph_tool_callers(
    store: &ProjectStore,
    params: &GraphSymbolParams,
) -> Result<serde_json::Value, McpError> {
    let (project, scan_id, artifact) = resolve_project_and_graph(store, &params.project)?;
    let conn = graph_queries::open_graph(std::path::Path::new(&artifact))
        .map_err(|e| mcp_err(e.to_string()))?;
    let mut query_paths = Vec::new();
    if let Some(file) = symbol_file_path(&conn, &params.symbol) {
        query_paths.push(file);
    }
    check_stale_snapshot(&project, &query_paths).map_err(stale_snapshot_mcp_err)?;
    let resolution = graph_queries::resolve_symbol_detailed(&conn, &params.symbol)
        .map_err(|e| mcp_err(e.to_string()))?;
    let target = resolution.node.clone();
    let rows = graph_queries::callers(&conn, &target.id).map_err(|e| mcp_err(e.to_string()))?;
    let limit = params.limit.unwrap_or(50);
    let truncated: Vec<_> = rows.into_iter().take(limit).collect();
    let mut inner = serde_json::json!({
        "project": project.display_name,
        "scan_id": scan_id,
        "symbol": target.fqn,
        "symbol_id": target.id,
        "callers": truncated,
    });
    attach_resolution_warning(&mut inner, &params.symbol, &resolution);
    let ctx = McpEnvelopeContext {
        store,
        project: &project,
        query_paths: Some(query_paths),
        routing_hint: Some(RoutedTool::GridseakGraphBlastRadius),
    };
    Ok(wrap_response_ctx(
        inner,
        &[
            "gridseak_graph_callees",
            "gridseak_graph_blast_radius",
            "gridseak_explain_finding",
        ],
        Some(ctx),
    ))
}

fn graph_tool_callees(
    store: &ProjectStore,
    params: &GraphSymbolParams,
) -> Result<serde_json::Value, McpError> {
    let (project, scan_id, artifact) = resolve_project_and_graph(store, &params.project)?;
    let conn = graph_queries::open_graph(std::path::Path::new(&artifact))
        .map_err(|e| mcp_err(e.to_string()))?;
    let mut query_paths = Vec::new();
    if let Some(file) = symbol_file_path(&conn, &params.symbol) {
        query_paths.push(file);
    }
    check_stale_snapshot(&project, &query_paths).map_err(stale_snapshot_mcp_err)?;
    let resolution = graph_queries::resolve_symbol_detailed(&conn, &params.symbol)
        .map_err(|e| mcp_err(e.to_string()))?;
    let target = resolution.node.clone();
    let rows = graph_queries::callees(&conn, &target.id).map_err(|e| mcp_err(e.to_string()))?;
    let limit = params.limit.unwrap_or(50);
    let truncated: Vec<_> = rows.into_iter().take(limit).collect();
    let mut inner = serde_json::json!({
        "project": project.display_name,
        "scan_id": scan_id,
        "symbol": target.fqn,
        "symbol_id": target.id,
        "callees": truncated,
    });
    attach_resolution_warning(&mut inner, &params.symbol, &resolution);
    let ctx = McpEnvelopeContext {
        store,
        project: &project,
        query_paths: Some(query_paths),
        routing_hint: Some(RoutedTool::GridseakGraphCallers),
    };
    Ok(wrap_response_ctx(
        inner,
        &["gridseak_graph_callers", "gridseak_graph_blast_radius"],
        Some(ctx),
    ))
}

/// Insert `resolution_method` + (when applicable) a
/// `resolution_warning` field onto an MCP envelope so the agent can
/// surface the caveat verbatim. The warning fires only on the
/// `SubstringUnique` path because that's the resolution method that
/// silently passed contaminated rows back to the agent in P2
/// dogfood. The other methods (exact / suffix-unique) are
/// deterministic enough to skip the noise. See Q3 in
/// `V0_1_0_RC1_FOLLOWUP_ISSUES.md`.
fn attach_resolution_warning(
    envelope: &mut serde_json::Value,
    raw_symbol: &str,
    resolution: &graph_queries::SymbolResolution,
) {
    let Some(map) = envelope.as_object_mut() else {
        return;
    };
    let method = serde_json::to_value(resolution.method).unwrap_or(serde_json::Value::Null);
    map.insert("resolution_method".to_string(), method);
    if resolution.method == graph_queries::ResolutionMethod::SubstringUnique {
        map.insert(
            "resolution_warning".to_string(),
            serde_json::json!({
                "kind": "substring_match",
                "message": format!(
                    "Input `{raw}` matched `{fqn}` only via a substring scan over node FQNs. \
                     This is the weakest resolution tier. Confirm with the user that `{fqn}` is the symbol they meant before reporting downstream findings.",
                    raw = raw_symbol,
                    fqn = resolution.node.fqn,
                ),
                "matched_symbol": resolution.node.fqn,
                "matched_id": resolution.node.id,
                "remediation": "Pass a fuller path (e.g. `module::symbol` or the canonical FQN) on the next call so resolution drops to suffix-unique or exact.",
            }),
        );
    }
}

fn graph_tool_blast_radius(
    store: &ProjectStore,
    params: &GraphBlastRadiusParams,
) -> Result<serde_json::Value, McpError> {
    let (project, scan_id, artifact) = resolve_project_and_graph(store, &params.project)?;
    let conn = graph_queries::open_graph(std::path::Path::new(&artifact))
        .map_err(|e| mcp_err(e.to_string()))?;
    if graph_queries::seed_looks_like_path(&params.symbol) {
        return Err(mcp_err(format!(
            "blast_radius expects a function/method symbol, not a file or path (got `{}`). \
             Resolve a specific public symbol in that file first — call \
             `gridseak_graph_callers` on it, or call `gridseak_get_recommendations` \
             for file-level risk.",
            params.symbol
        )));
    }
    let mut query_paths = Vec::new();
    if let Some(file) = symbol_file_path(&conn, &params.symbol) {
        query_paths.push(file);
    }
    check_stale_snapshot(&project, &query_paths).map_err(stale_snapshot_mcp_err)?;
    let resolution = graph_queries::resolve_symbol_detailed(&conn, &params.symbol)
        .map_err(|e| mcp_err(e.to_string()))?;
    let target = resolution.node.clone();
    let depth = params.depth.unwrap_or(3);
    let cap = params.cap.unwrap_or(200);
    let rows = graph_queries::blast_radius(&conn, &target.id, depth, cap)
        .map_err(|e| mcp_err(e.to_string()))?;
    let mut inner = serde_json::json!({
        "project": project.display_name,
        "scan_id": scan_id,
        "symbol": target.fqn,
        "symbol_id": target.id,
        "depth": depth,
        "direction": "upstream",
        "semantic": "transitive callers — what would have to be re-validated if the seed changes",
        "rows": rows,
    });
    attach_resolution_warning(&mut inner, &params.symbol, &resolution);
    let ctx = McpEnvelopeContext {
        store,
        project: &project,
        query_paths: Some(query_paths),
        routing_hint: Some(RoutedTool::GridseakGraphCallers),
    };
    Ok(wrap_response_ctx(
        inner,
        &[
            "gridseak_graph_callers",
            "gridseak_graph_slice",
            "gridseak_graph_file_blast_radius",
        ],
        Some(ctx),
    ))
}

fn graph_tool_file_blast_radius(
    store: &ProjectStore,
    params: &GraphFileBlastRadiusParams,
) -> Result<serde_json::Value, McpError> {
    let (project, scan_id, artifact) = resolve_project_and_graph(store, &params.project)?;
    let conn = graph_queries::open_graph(std::path::Path::new(&artifact))
        .map_err(|e| mcp_err(e.to_string()))?;

    // Normalise the file path: agents sometimes pass an absolute path
    // even when the parser recorded a repo-relative one. Strip a
    // matching project-root prefix to maximise the chance of a hit.
    // If the user gave us a relative path, we use it verbatim.
    let normalised = normalise_file_path_for_lookup(&project, &params.file);
    let query_paths = vec![normalize_rel_path(&normalised)];
    check_stale_snapshot(&project, &query_paths).map_err(stale_snapshot_mcp_err)?;

    let depth = params.depth.unwrap_or(3);
    let cap = params.cap.unwrap_or(200);
    let result = graph_queries::file_blast_radius(&conn, &normalised, depth, cap)
        .map_err(|e| mcp_err(e.to_string()))?;

    if result.seeds.is_empty() {
        return Err(mcp_err(format!(
            "no Function/Method nodes found in `{}` for project `{}` (resolved to scan {}). \
             Either the path is wrong (must be repo-relative — try `gridseak-cli/src/main.rs` \
             not `/Users/.../main.rs`) or the file is new since the last scan. Re-run \
             `gridseak_scan` if you've added the file recently.",
            params.file, project.display_name, scan_id
        )));
    }

    let inner = serde_json::json!({
        "project": project.display_name,
        "scan_id": scan_id,
        "depth": depth,
        "direction": "upstream",
        "semantic": "transitive callers of any callable symbol in the file — what would have to be re-validated if any symbol in the file changes",
        "result": result,
    });
    let ctx = McpEnvelopeContext {
        store,
        project: &project,
        query_paths: Some(query_paths),
        routing_hint: Some(RoutedTool::GridseakGraphBlastRadius),
    };
    Ok(wrap_response_ctx(
        inner,
        &[
            "gridseak_graph_blast_radius",
            "gridseak_graph_callers",
            "gridseak_get_recommendations",
        ],
        Some(ctx),
    ))
}

/// Strip a project-root prefix from `file` if it looks absolute and
/// shares a prefix with one of the project's roots. Falls through
/// unchanged for relative paths and for absolute paths outside any
/// known root.
fn normalise_file_path_for_lookup(
    project: &gridseak_local_store::ProjectDto,
    file: &str,
) -> String {
    use std::path::Path;
    let p = Path::new(file);
    if !p.is_absolute() {
        return file.to_string();
    }
    for root in &project.roots {
        let root_path = Path::new(&root.path);
        if let Ok(rel) = p.strip_prefix(root_path) {
            return rel.to_string_lossy().replace('\\', "/");
        }
    }
    file.to_string()
}

fn graph_tool_module_coupling(
    store: &ProjectStore,
    params: &GraphLimitParams,
) -> Result<serde_json::Value, McpError> {
    let (project, scan_id, artifact) = resolve_project_and_graph(store, &params.project)?;
    let conn = graph_queries::open_graph(std::path::Path::new(&artifact))
        .map_err(|e| mcp_err(e.to_string()))?;
    let limit = params.limit.unwrap_or(20);
    let rows = graph_queries::module_coupling(&conn, limit).map_err(|e| mcp_err(e.to_string()))?;
    let inner = serde_json::json!({
        "project": project.display_name,
        "scan_id": scan_id,
        "rows": rows,
    });
    let ctx = McpEnvelopeContext {
        store,
        project: &project,
        query_paths: None,
        routing_hint: Some(RoutedTool::GridseakGetFindings),
    };
    Ok(wrap_response_ctx(
        inner,
        &["gridseak_get_findings", "gridseak_get_recommendations"],
        Some(ctx),
    ))
}

fn graph_tool_cycles(
    store: &ProjectStore,
    params: &GraphCyclesParams,
) -> Result<serde_json::Value, McpError> {
    let (project, scan_id, artifact) = resolve_project_and_graph(store, &params.project)?;
    let conn = graph_queries::open_graph(std::path::Path::new(&artifact))
        .map_err(|e| mcp_err(e.to_string()))?;
    let limit = params.limit.unwrap_or(20);
    let max_depth = params.max_depth.unwrap_or(8);
    let rows =
        graph_queries::cycles(&conn, limit, max_depth).map_err(|e| mcp_err(e.to_string()))?;
    let inner = serde_json::json!({
        "project": project.display_name,
        "scan_id": scan_id,
        "max_depth": max_depth,
        "cycles": rows,
    });
    let ctx = McpEnvelopeContext {
        store,
        project: &project,
        query_paths: None,
        routing_hint: Some(RoutedTool::GridseakGetRecommendations),
    };
    Ok(wrap_response_ctx(
        inner,
        &["gridseak_get_recommendations", "gridseak_explain_finding"],
        Some(ctx),
    ))
}

fn graph_tool_slice(
    store: &ProjectStore,
    params: &GraphBlastRadiusParams,
) -> Result<serde_json::Value, McpError> {
    let (project, scan_id, artifact) = resolve_project_and_graph(store, &params.project)?;
    let conn = graph_queries::open_graph(std::path::Path::new(&artifact))
        .map_err(|e| mcp_err(e.to_string()))?;
    let mut query_paths = Vec::new();
    if let Some(file) = symbol_file_path(&conn, &params.symbol) {
        query_paths.push(file);
    }
    check_stale_snapshot(&project, &query_paths).map_err(stale_snapshot_mcp_err)?;
    let resolution = graph_queries::resolve_symbol_detailed(&conn, &params.symbol)
        .map_err(|e| mcp_err(e.to_string()))?;
    let target = resolution.node.clone();
    let depth = params.depth.unwrap_or(2);
    let cap = params.cap.unwrap_or(200);
    let rows =
        graph_queries::slice(&conn, &target.id, depth, cap).map_err(|e| mcp_err(e.to_string()))?;
    let mut inner = serde_json::json!({
        "project": project.display_name,
        "scan_id": scan_id,
        "symbol": target.fqn,
        "symbol_id": target.id,
        "depth": depth,
        "rows": rows,
    });
    attach_resolution_warning(&mut inner, &params.symbol, &resolution);
    let ctx = McpEnvelopeContext {
        store,
        project: &project,
        query_paths: Some(query_paths),
        routing_hint: Some(RoutedTool::GridseakGraphCallers),
    };
    Ok(wrap_response_ctx(
        inner,
        &["gridseak_graph_callers", "gridseak_explain_finding"],
        Some(ctx),
    ))
}

fn explain_finding_envelope(
    store: &ProjectStore,
    project_ref: &str,
    finding_id: &str,
) -> Result<serde_json::Value> {
    let project = store.resolve_project_lenient(project_ref)?;
    let scan = project
        .latest_scan
        .clone()
        .context("project has no scans")?;
    let report: HealthReport = serde_json::from_value(store.load_report(&scan.id)?)?;
    let finding = report
        .findings
        .iter()
        .find(|f| f.id == finding_id)
        .with_context(|| format!("finding `{finding_id}` not on latest scan"))?;
    let priority = render::recommendations::priority_for(&report, finding_id);
    let target = finding
        .primary_node_id
        .clone()
        .or_else(|| finding.node_ids.first().cloned())
        .unwrap_or_else(|| "—".into());
    let inner = serde_json::json!({
        "project": project.display_name,
        "scan_id": scan.id,
        "finding": finding,
        "priority": priority,
        "target": target,
    });
    Ok(wrap_response(
        inner,
        &[
            "gridseak_graph_blast_radius",
            "gridseak_graph_callers",
            "gridseak_get_findings",
        ],
    ))
}

fn context_for_llm_envelope(
    store: &ProjectStore,
    params: &ContextForLlmParams,
) -> Result<serde_json::Value> {
    let project = store.resolve_project_lenient(&params.project)?;
    let scan = project
        .latest_scan
        .clone()
        .context("project has no scans")?;
    let report: HealthReport = serde_json::from_value(store.load_report(&scan.id)?)?;
    let priorities = priority::compute_priorities(&report, 10);
    let readiness = load_analysis_readiness(store, &project);
    let summary = serde_json::json!({
        "project": project.display_name,
        "scan_id": scan.id,
        "branch": scan.git_branch,
        "commit": scan.git_commit,
        "dirty": scan.git_dirty,
        "languages": scan.scan_languages,
        "score": report.health_score,
        "total_findings": report.findings.len(),
    });
    let inner = serde_json::json!({
        "summary": summary,
        "metrics": project.latest_metrics,
        "top_recommendations": priorities,
        "report_path": scan.report_path,
        "graph_artifact_path": scan.graph_artifact_path,
        "budget_hint": params.budget.unwrap_or(4000),
        "focus": params.focus,
        "finding": params.finding,
        "changed_files_requested": params.changed_files.unwrap_or(false),
        "integrity_status": report.integrity_status,
        "routing_table": routing_table_payload(),
        "recommended_first_call": recommended_first_call(store, &project),
        "analysis_readiness": readiness,
    });
    let ctx = McpEnvelopeContext {
        store,
        project: &project,
        query_paths: None,
        routing_hint: Some(RoutedTool::GridseakGetRecommendations),
    };
    Ok(wrap_response_ctx(
        inner,
        &[
            "gridseak_graph_blast_radius",
            "gridseak_explain_finding",
            "gridseak_get_recommendations",
        ],
        Some(ctx),
    ))
}

fn routing_table_payload() -> serde_json::Value {
    serde_json::json!(routing_table()
        .into_iter()
        .map(|(symptom, tool, pre)| {
            serde_json::json!({
                "symptom": symptom,
                "tool": tool.mcp_name(),
                "precondition": format!("{pre:?}"),
            })
        })
        .collect::<Vec<_>>())
}

fn recommended_first_call(
    store: &ProjectStore,
    project: &gridseak_local_store::ProjectDto,
) -> &'static str {
    if project.latest_scan.is_none() {
        "gridseak_scan"
    } else {
        let readiness = load_analysis_readiness(store, project);
        if readiness.analysis_complete {
            "gridseak_get_recommendations"
        } else if readiness.graph_ready {
            "gridseak_graph_callers"
        } else {
            "gridseak_scan"
        }
    }
}
