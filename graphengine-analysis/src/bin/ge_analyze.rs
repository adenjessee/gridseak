//! ge-analyze CLI entry point.
//!
//! Usage: ge-analyze --db <path-to-sqlite> --output <path-to-health-json> [--config <path.toml>]
//!
//! Exit codes:
//!   0 — Success
//!   1 — Database not found or unreadable
//!   2 — Database schema invalid
//!   3 — Output path not writable
//!   4 — Analysis error (internal)
//!   5 — Invalid config file
//!   6 — Invalid overrides file

use std::path::PathBuf;
use std::process;

use clap::Parser;
use graphengine_analysis::health::config::{self, AnalysisConfig};

#[derive(Parser)]
#[command(
    name = "ge-analyze",
    about = "Structural health analysis for parsed codebase graphs",
    // Version is mandatory: `gridseak-engine-runner::version_check` and
    // `gridseak doctor` both spawn `ge-analyze --version` to detect when
    // the runner and its sidecar binaries have drifted (e.g. the
    // installer copied a new `gridseak` over an existing prefix but did
    // not refresh the analyzer). The expected format is the clap
    // default `name <version>` on stdout; do not change it without
    // updating `gridseak_engine_runner::version_check::parse_version_line`.
    version = env!("CARGO_PKG_VERSION"),
)]
struct Cli {
    /// Path to SQLite database created by graphengine-parsing
    #[arg(long)]
    db: PathBuf,

    /// Path where the health JSON report will be written
    #[arg(long)]
    output: PathBuf,

    /// Output format
    #[arg(long, default_value = "json-pretty")]
    format: String,

    /// Path to TOML configuration file (overrides ecosystem profile defaults)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Explicitly set the ecosystem (overrides auto-detection)
    #[arg(long)]
    ecosystem: Option<String>,

    /// Path to population (norms) SQLite database for percentile-based scoring.
    /// When provided, the report includes a `percentiles` block and the health
    /// score is derived from the composite percentile rank.
    #[arg(long)]
    norms: Option<PathBuf>,

    /// Path to .git directory for temporal coupling analysis.
    /// When provided, git log is parsed to detect files that frequently change together.
    #[arg(long)]
    git_dir: Option<PathBuf>,

    /// Exclude test files from all analysis metrics and findings.
    /// Test files are still loaded but filtered from production graph.
    #[arg(long, default_value_t = false)]
    exclude_tests: bool,

    /// Exclude generated/vendor/build-output files from all analysis.
    #[arg(long, default_value_t = false)]
    exclude_generated: bool,

    /// Emit the pre-analysis validation payload to stdout and exit.
    /// Does not run analysis — just queries the parsed DB for uncertain
    /// classifications, dead code candidates, module boundaries, and repo type.
    #[arg(long, default_value_t = false)]
    emit_validation: bool,

    /// Path to a JSON overrides file from user validation.
    /// Applied after loading the graph but before running analysis.
    #[arg(long)]
    overrides: Option<PathBuf>,

    /// Path to the repository working-tree root for T7 Layer 0 git
    /// signal extraction. When provided, `ge-analyze` runs
    /// `graphengine-git-signals` and attaches the resulting
    /// `GitSignalReport` to `HealthReport.git_signals`. When omitted,
    /// no Layer 0 signals are collected and the field is `None` —
    /// downstream consumers MUST NOT interpret absence as "no
    /// churn" (see `CAVEAT_LAYER0_GIT_SIGNALS_V1`).
    ///
    /// Note: this is intentionally separate from `--git-dir`. The
    /// `--git-dir` flag points at a `.git/` directory for commit-
    /// message-based temporal coupling (pre-T7), which consumes raw
    /// `git log` output. `--repo-root` points at the *working tree*
    /// and is consumed by the `gix`-based signal extractor. Passing
    /// the same repository to both is fine; they read different
    /// things.
    #[arg(long)]
    repo_root: Option<PathBuf>,

    /// Opt out of T7 Layer 0 git signal collection even when
    /// `--repo-root` (or its default) points at a valid repository.
    /// Use this on CI jobs where the signal cost is unacceptable
    /// or when you specifically want a report shape identical to
    /// the pre-T7 engine (e.g. for regression comparisons).
    #[arg(long, default_value_t = false)]
    no_git_signals: bool,

    /// Emit machine-readable progress events as JSON Lines (JSONL) to stderr.
    ///
    /// Each major analyzer detector boundary (cycle detection, fan
    /// metrics, dead code, complexity, etc.) emits one
    /// `EngineEvent::Progress` line. `gridseak-engine-runner` already
    /// drains stderr and parses each line through
    /// `graphengine_progress::try_parse_line`; structured events
    /// surface in `gridseak-cli` and the desktop shell as live
    /// percentage updates. Without this flag the analyzer remains
    /// `eprintln!`-only, which is what every pre-Stage-1 caller
    /// expects.
    #[arg(long, default_value_t = false)]
    progress_json: bool,

    /// S2: reuse cached analysis on warm incremental rescans when the
    /// parse-layer delta is below threshold. Refreshes cache after a
    /// full analysis run.
    #[arg(long, default_value_t = false)]
    incremental_analysis: bool,

    /// S2-γ: force L3 full analysis (skip segment cache / L1 fast merge).
    #[arg(long, default_value_t = false)]
    full_analysis: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.progress_json {
        // One-time process-global toggle. Flipped before any detector
        // runs so the structured stage-start event for `loading` lands
        // on the wire. Never flipped back — this process is one-shot.
        graphengine_analysis::health::progress::enable();
    }

    if !cli.db.exists() {
        eprintln!(
            "[ge-analyze] Error: database not found: {}",
            cli.db.display()
        );
        process::exit(1);
    }

    let db_str = cli.db.to_string_lossy().to_string();
    {
        let conn = match rusqlite::Connection::open_with_flags(
            &cli.db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[ge-analyze] Error: cannot open database: {e}");
                process::exit(1);
            }
        };

        if let Err(e) = graphengine_analysis::health::graph::validate_schema(&conn) {
            eprintln!("[ge-analyze] Error: invalid database schema: {e}");
            process::exit(2);
        }
    }

    // Handle --emit-validation: output validation payload and exit
    if cli.emit_validation {
        match graphengine_analysis::validation::emit_validation_payload(&db_str) {
            Ok(payload) => {
                let json = match cli.format.as_str() {
                    "json" => serde_json::to_string(&payload),
                    _ => serde_json::to_string_pretty(&payload),
                };
                match json {
                    Ok(j) => {
                        println!("{j}");
                        // Also write to output if specified
                        let _ = std::fs::write(&cli.output, &j);
                    }
                    Err(e) => {
                        eprintln!(
                            "[ge-analyze] Error: failed to serialize validation payload: {e}"
                        );
                        process::exit(4);
                    }
                }
            }
            Err(e) => {
                eprintln!("[ge-analyze] Error: validation payload generation failed: {e}");
                process::exit(4);
            }
        }
        return;
    }

    if let Some(parent) = cli.output.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            eprintln!(
                "[ge-analyze] Error: output directory does not exist: {}",
                parent.display()
            );
            process::exit(3);
        }
    }

    // Load overrides if provided
    let overrides = cli.overrides.as_ref().map(|path| {
        let path_str = path.to_string_lossy().to_string();
        match graphengine_analysis::validation::overrides::load_overrides(&path_str) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("[ge-analyze] Error: {e}");
                process::exit(6);
            }
        }
    });

    // Build analysis config
    let analysis_config = build_config(&cli);
    let norms_str = cli.norms.as_ref().map(|p| p.to_string_lossy().to_string());
    let git_dir_str = cli
        .git_dir
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    if cli.incremental_analysis {
        use rusqlite::OpenFlags;
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
            &cli.db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            if let Ok(Some(outcome)) =
                graphengine_analysis::health::incremental_fast_path::try_fast_path(
                    &conn, &db_str, true,
                )
            {
                graphengine_analysis::health::pipeline::emit_analysis_mode(
                    graphengine_analysis::health::pipeline::segments::AnalysisMode::ZeroReuse,
                    0,
                    graphengine_analysis::health::pipeline::scope::TrustLevel::L0,
                    &[],
                    &[],
                );
                eprintln!(
                    "[ge-analyze] S2 incremental fast-path ({:?}): reusing cached analysis report",
                    outcome.tier
                );
                write_report_and_exit(&cli, outcome.report);
                return;
            }
        }
    }

    let pipeline_outcome =
        match graphengine_analysis::health::pipeline::run_analysis_pipeline_with_options(
            &db_str,
            analysis_config,
            norms_str.as_deref(),
            git_dir_str.as_deref(),
            overrides.as_ref(),
            cli.incremental_analysis,
            graphengine_analysis::health::pipeline::PipelineOptions {
                force_full_analysis: cli.full_analysis,
            },
        ) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("[ge-analyze] Error: analysis failed: {e}");
                process::exit(4);
            }
        };
    let mut report = pipeline_outcome.report;

    // T7 Layer 0 attach + churn-downgrade. Runs only when the
    // operator both supplied a `--repo-root` and did NOT pass
    // `--no-git-signals`. The attach helper logs and returns
    // `Skipped` on any extractor failure; in either case the
    // report is still writable and the next step proceeds.
    if let (Some(root), false) = (cli.repo_root.as_ref(), cli.no_git_signals) {
        use graphengine_analysis::health::git_signals_attach;
        use graphengine_git_signals::HistoryWindow;
        let outcome = git_signals_attach::attach_git_signals(
            &mut report,
            root.as_path(),
            &HistoryWindow::default_ci(),
        );
        match outcome {
            git_signals_attach::GitSignalAttachOutcome::Attached { shape } => {
                eprintln!("[ge-analyze] git_signals: attached (shape = {:?})", shape);
            }
            git_signals_attach::GitSignalAttachOutcome::Skipped(err) => {
                eprintln!("[ge-analyze] git_signals: skipped — {err}");
            }
        }

        let n = git_signals_attach::apply_dead_code_churn_downgrade_to_annotations(&mut report);
        if n > 0 {
            eprintln!("[ge-analyze] git_signals: downgraded dead-code confidence on {n} node(s)");
        }
    }

    // T8 (universal-fidelity sprint). Attach the per-file
    // extraction-coverage vector persisted by the parser and then
    // apply the coverage-gap downgrader on top of whatever T7 left
    // behind. Any failure to open/read the coverage table is
    // reported and the pipeline continues with an empty vector so
    // the report is still writable (the absence is itself honest).
    {
        use graphengine_analysis::health::coverage_attach;
        use graphengine_parsing::infrastructure::storage::sqlite_repository::SqliteRepository;

        let coverage = match rusqlite::Connection::open_with_flags(
            &db_str,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(conn) => match SqliteRepository::read_file_extraction_coverage_conn(&conn) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!(
                        "[ge-analyze] extraction_coverage: read failed — {e} (continuing with empty set)"
                    );
                    Vec::new()
                }
            },
            Err(e) => {
                eprintln!(
                    "[ge-analyze] extraction_coverage: DB reopen failed — {e} (continuing with empty set)"
                );
                Vec::new()
            }
        };

        let attach_n = coverage.len();
        coverage_attach::attach_extraction_coverage(&mut report, coverage);
        if attach_n > 0 {
            eprintln!(
                "[ge-analyze] extraction_coverage: attached {attach_n} file coverage record(s)"
            );
        }

        let downgraded =
            coverage_attach::apply_extraction_coverage_downgrade_to_annotations(&mut report);
        if downgraded > 0 {
            eprintln!(
                "[ge-analyze] extraction_coverage: downgraded dead-code confidence on {downgraded} node(s)"
            );
        }

        // Recompute the dual-metric companion **after** all T7 and
        // T8 downgrades. Safe to call unconditionally — absent
        // coverage just leaves total == high_confidence, which is
        // informative without being misleading.
        let (total, high) = coverage_attach::recompute_no_callers_confidence_split(&mut report);
        eprintln!("[ge-analyze] dead_code.no_callers: total = {total}, high_confidence = {high}");
    }

    write_report_and_exit(&cli, report);
}

fn write_report_and_exit(cli: &Cli, report: graphengine_analysis::health::report::HealthReport) {
    let json = match cli.format.as_str() {
        "json" => serde_json::to_string(&report),
        _ => serde_json::to_string_pretty(&report),
    };

    let json_str = match json {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[ge-analyze] Error: failed to serialize report: {e}");
            process::exit(4);
        }
    };

    match std::fs::write(&cli.output, &json_str) {
        Ok(_) => {
            graphengine_analysis::health::progress::emit_progress(
                "report_written",
                100,
                "health report written",
            );
            eprintln!(
                "[ge-analyze] Health report written to {}",
                cli.output.display()
            );
        }
        Err(e) => {
            graphengine_analysis::health::progress::emit_error(
                "report_written",
                &format!("cannot write output file: {e}"),
            );
            eprintln!("[ge-analyze] Error: cannot write output file: {e}");
            process::exit(3);
        }
    }
}

fn build_config(cli: &Cli) -> Option<AnalysisConfig> {
    let eco_override = cli
        .ecosystem
        .as_deref()
        .map(config::Ecosystem::from_language_str);

    let mut cfg = match (&cli.config, eco_override) {
        (Some(config_path), _) => {
            let base = eco_override.unwrap_or(config::Ecosystem::Unknown);
            match config::load_config_from_toml(&config_path.to_string_lossy(), base) {
                Ok(mut cfg) => {
                    if let Some(eco) = eco_override {
                        cfg.ecosystem = Some(eco);
                    }
                    cfg
                }
                Err(e) => {
                    eprintln!("[ge-analyze] Error: {e}");
                    process::exit(5);
                }
            }
        }
        (None, Some(eco)) => AnalysisConfig::for_ecosystem(eco),
        (None, None) => return None, // auto-detect from DB
    };

    cfg.exclude_tests = cli.exclude_tests;
    cfg.exclude_generated = cli.exclude_generated;
    Some(cfg)
}
