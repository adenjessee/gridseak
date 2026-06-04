//! `gridseak scan [PATH]` — the spec's first-run command.
//!
//! Responsibilities (and why each lives here, not inside `main.rs`):
//!
//! 1. Parse the new positional + flag surface (`ScanArgs`). Kept here
//!    so main.rs's `Commands` enum doesn't drag in width/render
//!    types.
//! 2. Drive the upsert → detect → run → persist → render flow. The
//!    "run → persist" portion delegates to `crate::rescan_project`
//!    which is the same code path the legacy `scan rescan` subcommand
//!    drives; this stage's job is only to extend it with rendering.
//! 3. Pick the renderer based on the resolved format. Stage 3 owns
//!    the actual renderers (in `crate::render::*`); Stage 2 wires them
//!    so every documented flag does something visible.
//!
//! Anything that touches the engine pipeline, scratch dir, or
//! HealthReport persistence stays in `crate::main` (or, after Stage 3,
//! moves into its own dedicated module). This module is a *driver*,
//! not the engine.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use graphengine_analysis::health::report::HealthReport;
use gridseak_local_store::{LocalStorePaths, ProjectStore};

use crate::progress::ProgressMode;
use crate::render::{render_hero, view::ScanReportView, width, HeroFormat};

/// Args for the top-level `Scan` command.
///
/// `args_conflicts_with_subcommands = true` lets `gridseak scan
/// [PATH]` and `gridseak scan <subcommand>` coexist. Clap picks
/// the subcommand path whenever the first non-flag token matches a
/// known subcommand name (e.g. `latest`, `rescan`); everything else
/// (including bare `.`, `./path`, absolute paths) falls through to
/// the positional `path` field.
#[derive(Args, Debug, Clone)]
#[command(args_conflicts_with_subcommands = true)]
pub struct ScanArgs {
    /// Repository root to scan. Defaults to the current directory.
    pub path: Option<PathBuf>,

    /// Output shape for the report.
    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table)]
    pub format: ScanOutputFormat,

    /// Override `--format` and emit the LLM-friendly rendering. Wins
    /// over `--format` because LLM consumers should not have to
    /// remember to also pass `--format`.
    #[arg(long, default_value_t = false)]
    pub for_llm: bool,

    /// Token budget for `--for-llm`. Ignored otherwise. When set, the
    /// LLM renderer trims its lowest-priority sections until it
    /// estimates fitting under this token count.
    #[arg(long)]
    pub budget: Option<usize>,

    /// Restrict the scan to a single language (e.g. `--lang typescript`).
    #[arg(long)]
    pub lang: Option<String>,

    /// Comma-separated languages to include. Combines with `--lang`.
    #[arg(long, value_delimiter = ',')]
    pub languages: Vec<String>,

    /// Force progress UI off for this scan even if globals say otherwise.
    #[arg(long, default_value_t = false)]
    pub no_progress: bool,

    /// Verbose progress + diagnostic logging. Currently mirrors the
    /// default plain renderer; reserved so future stages can wire
    /// debug-level logging without changing the flag surface.
    #[arg(long, default_value_t = false)]
    pub verbose: bool,

    /// Bypass S1 incremental scanning for this run. Forces a full
    /// reparse of every discovered file regardless of the
    /// `file_cache` table's state. Useful when investigating
    /// suspected cache staleness or pinning a scan for a
    /// before/after comparison. The persistent parse DB is still
    /// reused (so node ids stay stable across scans); only the
    /// per-file cache lookup is skipped. Pair with `gridseak doctor`
    /// if you suspect a corrupted DB.
    #[arg(long, default_value_t = false)]
    pub no_incremental: bool,

    /// S2-γ: force L3 full analysis (skip segment-cache fast merge).
    #[arg(long, default_value_t = false)]
    pub full_analysis: bool,

    /// Legacy `scan` namespace subcommands. Without one of these,
    /// the driver runs the first-run flow on `path`.
    #[command(subcommand)]
    pub sub: Option<crate::ScanCommand>,
}

/// Output format selector for `gridseak scan [PATH]`.
#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "lower")]
pub enum ScanOutputFormat {
    /// Width-aware terminal table (default).
    Table,
    /// GitHub-flavored Markdown.
    Markdown,
    /// Stable JSON envelope around `ScanReportView`.
    Json,
}

impl ScanArgs {
    /// Resolve which renderer to use, given the overlay flags. Order
    /// of precedence (highest wins): `--for-llm` → global `--json` →
    /// explicit `--format`. `--for-llm` wins because the LLM
    /// renderer is the only one that takes a budget; the user who
    /// passed both `--for-llm` and a format almost certainly wanted
    /// the LLM output. `--json` wins over `--format` because it is
    /// a globally-recognised shorthand and matches how every
    /// modern CLI surfaces machine-readable output.
    fn hero_format(&self, global_json: bool) -> HeroFormat {
        if self.for_llm {
            return HeroFormat::ForLlm {
                budget: self.budget,
            };
        }
        if global_json {
            return HeroFormat::Json;
        }
        match self.format {
            ScanOutputFormat::Table => HeroFormat::Table {
                layout: width::detect(),
            },
            ScanOutputFormat::Markdown => HeroFormat::Markdown,
            ScanOutputFormat::Json => HeroFormat::Json,
        }
    }

    fn resolved_progress(&self, base: ProgressMode) -> ProgressMode {
        if self.no_progress {
            ProgressMode::Silent
        } else {
            base
        }
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Implementation of `gridseak scan [PATH]`.
///
/// Steps (each is a logged side-effect):
///
/// 1. Resolve `path` → absolute path. Bails with a typed error if
///    the folder does not exist; we never silently scan the user's
///    cwd when they pointed us elsewhere.
/// 2. Upsert the project via `ProjectStore::create_project_for_folder`.
///    The store's contract is "return existing project for this
///    canonical root path, else create one"; that is exactly the
///    "upsert" the spec asks for.
/// 3. Build the language request from `--lang` + `--languages`. If
///    both are empty, the underlying `rescan_project` flow falls
///    back to its standard auto-detect.
/// 4. Run the engine pipeline via `rescan_project`. The function
///    handles begin_scan / fail_scan / complete_scan for us.
/// 5. Load the freshly persisted `HealthReport` and the new
///    `ScanRunDto` via the store.
/// 6. Build the `ScanReportView` and dispatch to the requested
///    renderer.
pub async fn run_scan_now(
    store: &ProjectStore,
    paths: &LocalStorePaths,
    args: ScanArgs,
    base_progress_mode: ProgressMode,
    global_json: bool,
) -> Result<()> {
    let path = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let absolute = path
        .canonicalize()
        .with_context(|| format!("path does not exist: {}", path.display()))?;
    let metadata = std::fs::metadata(&absolute)
        .with_context(|| format!("cannot stat {}", absolute.display()))?;
    if !metadata.is_dir() {
        anyhow::bail!(
            "scan target is not a directory: {} (gridseak scan operates on a project root)",
            absolute.display()
        );
    }

    let project = store
        .create_project_for_folder(&absolute)
        .with_context(|| format!("upsert project for {}", absolute.display()))?;

    let language_request = crate::scan_language_request(args.lang.clone(), args.languages.clone());
    let progress_mode = args.resolved_progress(base_progress_mode);
    let incremental = !args.no_incremental;

    let _outcome = crate::rescan_project(
        store,
        paths,
        &project.id,
        language_request,
        "cli",
        progress_mode,
        incremental,
        args.full_analysis,
    )
    .await?;

    let refreshed = store
        .resolve_project(&project.id)
        .context("scan completed but project disappeared from store")?;
    let scan = refreshed
        .latest_scan
        .clone()
        .context("scan completed but no latest_scan was persisted")?;

    let report_value = store
        .load_report(&scan.id)
        .with_context(|| format!("load persisted report for scan {}", scan.id))?;
    let report: HealthReport = serde_json::from_value(report_value.clone())
        .context("persisted report failed to deserialize into HealthReport — schema drift?")?;

    let view = ScanReportView::build(&report, &refreshed, &scan);

    let format = args.hero_format(global_json);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    render_hero(&format, &view, &mut handle)?;
    Ok(())
}
