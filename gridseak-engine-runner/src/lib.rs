//! Scan-pipeline orchestrator shared by `gridseak-cli` and the desktop shell.
//!
//! # Why this crate exists
//!
//! Before this crate, `gridseak-cli/src/main.rs::run_scan_pipeline` and
//! `desktop/src-tauri/src/engine.rs::run_pipeline` each independently
//! orchestrated parser-then-analyzer subprocess calls against a target repo.
//! They drifted on flag defaults (the CLI silently skipped
//! `--exclude-tests` / `--exclude-generated`, the desktop applied them),
//! on polyglot semantics (the CLI silently fell back to parsing only the
//! first detected language when the parser binary was missing), and on
//! scratch-path conventions. The same repo therefore produced different
//! `HealthReport` numbers depending on which surface ran the scan, which
//! breaks the foundation promise — see `docs/00-strategy/FOUNDATION_EXIT_CRITERIA.md`.
//!
//! This crate is the single place where "run parser, run analyzer, emit
//! progress, fail loudly on missing engines" is implemented. Both surfaces
//! become thin wrappers that resolve their environment-specific concerns
//! (binary discovery, scratch dir, progress rendering) and hand the runner
//! a fully resolved [`config::RunPipelineConfig`].
//!
//! # What this crate is **not**
//!
//! - It does not own scan-record persistence — that lives in
//!   [`gridseak-local-store`].
//! - It does not own UI rendering of progress — consumers implement
//!   [`progress::ProgressSink`] for their environment.
//! - It does not own binary discovery rules — consumers resolve
//!   `parser_bin` / `analyzer_bin` themselves (CLI walks `~/.gridseak/bin`
//!   and PATH; desktop uses Tauri sidecar resolution).
//! - It does not own language detection — consumers detect first, then
//!   pass the requested language list to the runner.
//!
//! These boundaries are deliberate. Path resolution and rendering are
//! environment-specific; trying to share them would force this crate to
//! know about Tauri or about home-directory conventions, which would in
//! turn make it harder to test and harder to extend.

pub mod config;
pub mod error;
pub mod progress;
pub mod registry;
mod subprocess;
pub mod version_check;

pub use config::{RunPipelineConfig, RunPipelineOutput};
pub use error::{BinaryKind, RunError};
pub use progress::{ProgressEvent, ProgressSink, Stage};
pub use registry::{LanguageRegistry, SupportedLanguage};
pub use version_check::{
    check_binary_version, probe_engine_binaries, read_version, VersionProbe, VersionProbeOutcome,
    EXPECTED_SIDECAR_VERSION,
};

// Re-export the shared engine-event vocabulary so consumers don't need
// to add a second dependency on `graphengine-progress` just to match on
// the `Engine` variant of `ProgressEvent`.
pub use graphengine_progress::EngineEvent;

use std::path::Path;
use std::time::Instant;

use graphengine_analysis::health::report::HealthReport;
use tokio_util::sync::CancellationToken;

/// Run the full parser-then-analyzer pipeline against `cfg.root`.
///
/// Behavior:
///
/// 1. Validate that `parser_bin` and `analyzer_bin` exist; if not, return
///    [`RunError::BinaryMissing`] with the kind that's missing.
/// 2. Load the language registry from the parser binary's `languages
///    --json` subcommand.
/// 3. Filter `cfg.languages` to drop any entries flagged `discovery_only`
///    in the registry — those cannot be parsed standalone (e.g.
///    Visualforce, which is absorbed by the Apex parser). Skipped
///    languages are reported back in
///    [`RunPipelineOutput::languages_skipped`].
/// 4. If no parseable languages remain, return
///    [`RunError::NoParseableLanguages`] rather than silently producing
///    an empty graph.
/// 5. Run the parser once per remaining language, in order. Pass
///    `--clear` only on the first invocation **when the parse DB is
///    ephemeral** (no `persistent_parse_db` set) so passes 2..N
///    append to the same SQLite DB. When a persistent path is
///    configured (the CLI path post-S1-ε), `--clear` is NEVER passed
///    and incremental scanning's `file_cache` table accumulates
///    across scans. Schema-version bumps and per-file row pruning
///    inside the parser keep the persistent DB honest.
/// 6. Run the analyzer with `--exclude-tests` / `--exclude-generated`
///    according to `cfg`. The analyzer writes the report JSON to
///    `cfg.scratch_dir.join("{scan_id}.report.json")`.
/// 7. Read the report back from disk and return it alongside its path.
///
/// Stderr from each subprocess is drained simultaneously to a per-stage
/// log file under `scratch_dir` (so failures include a tail of the real
/// engine output, not just an exit code) and forwarded to
/// [`ProgressSink`] as raw lines (Stage 1 graduates this to structured
/// events emitted as JSONL by parser and analyzer).
///
/// Cancellation: if `cfg.cancel` is `Some`, the runner aborts at the
/// next subprocess `wait` await point when the token fires, killing the
/// in-flight child and returning [`RunError::Cancelled`]. Consumers
/// without a cancel UI should pass `None`.
pub async fn run_pipeline(mut cfg: RunPipelineConfig) -> Result<RunPipelineOutput, RunError> {
    let cancel = cfg.cancel.take().unwrap_or_default();

    // Lift the progress sink out of `cfg` so we can hold `&cfg` immutably
    // for the rest of the run (every other field is read-only at this
    // point) while the sink is mutably borrowed independently. The split
    // borrow makes the ownership story explicit: the runner has unique
    // access to the sink for the pipeline's lifetime; cfg becomes pure
    // read-only configuration.
    let mut progress: Box<dyn ProgressSink + Send + Sync> =
        std::mem::replace(&mut cfg.progress, Box::new(crate::progress::DiscardSink));

    // ---- 1. Sanity-check inputs -----------------------------------------
    validate_binary(BinaryKind::Parser, &cfg.parser_bin)?;
    validate_binary(BinaryKind::Analyzer, &cfg.analyzer_bin)?;

    // Version-drift check: every sidecar must report the same
    // `CARGO_PKG_VERSION` this runner was built with. The shadow-mode
    // promise is that a `gridseak scan` produces deterministic
    // results, and a stale sidecar silently breaks that promise (e.g.
    // an older `ge-analyze` that does not understand `--progress-json`
    // turns every scan into a parse error). Fail loudly here instead
    // of letting the parser/analyzer subprocess emit an obscure
    // `unexpected argument` deep in the run.
    //
    // The check runs *before* `LanguageRegistry::load_from` because
    // that call already spawns the parser binary — if the parser is
    // stale, the error from the registry load would mask the real
    // diagnosis with a confusing "language registry load failed".
    version_check::check_binary_version(BinaryKind::Parser, &cfg.parser_bin).await?;
    version_check::check_binary_version(BinaryKind::Analyzer, &cfg.analyzer_bin).await?;

    if cfg.languages.is_empty() {
        return Err(RunError::NoParseableLanguages);
    }

    std::fs::create_dir_all(&cfg.scratch_dir).map_err(RunError::Io)?;

    // S1-ε: route the parse DB to the persistent per-project location
    // when the caller supplies one, else fall back to the legacy
    // per-scan-ephemeral path under `scratch_dir`. The report JSON and
    // stderr logs are ALWAYS per-scan because they are the deliverable
    // (one report-per-scan-id is the contract local-store relies on
    // for history / compare). Create the persistent DB's parent
    // directory eagerly so the first call to `SqliteRepository::new`
    // doesn't fail on a fresh project.
    let (db_path, db_is_persistent) = match cfg.persistent_parse_db.as_ref() {
        Some(persistent) => {
            if let Some(parent) = persistent.parent() {
                std::fs::create_dir_all(parent).map_err(RunError::Io)?;
            }
            (persistent.clone(), true)
        }
        None => (
            cfg.scratch_dir.join(format!("{}.sqlite", cfg.scan_id)),
            false,
        ),
    };
    let report_path = cfg.scratch_dir.join(format!("{}.report.json", cfg.scan_id));

    // ---- 2. Load registry ------------------------------------------------
    progress.on_event(ProgressEvent::StageStarted {
        stage: Stage::Preparing,
        language: None,
    });
    let stage_start = Instant::now();

    let registry = registry::LanguageRegistry::load_from(&cfg.parser_bin, &cfg.configs_dir)
        .await
        .map_err(|e| RunError::LanguageRegistry(e.to_string()))?;

    // ---- 3. Filter discovery-only languages ------------------------------
    let (languages, languages_skipped) = registry.partition_parseable(&cfg.languages);
    for skipped in &languages_skipped {
        tracing::warn!(
            scan_id = %cfg.scan_id,
            language = %skipped,
            "skipping discovery-only language; absorbed by companion parser"
        );
    }
    if languages.is_empty() {
        return Err(RunError::NoParseableLanguages);
    }

    progress.on_event(ProgressEvent::StageFinished {
        stage: Stage::Preparing,
        language: None,
        elapsed_ms: stage_start.elapsed().as_millis() as u64,
    });

    // ---- 4. Parse phase --------------------------------------------------
    progress.on_event(ProgressEvent::StageStarted {
        stage: Stage::Parsing,
        language: None,
    });
    let parse_start = Instant::now();

    if db_is_persistent && cfg.incremental {
        if let Err(err) = reset_incremental_scan_stats_for_parse_phase(&db_path) {
            tracing::warn!(
                "S2: failed to reset incremental_scan_stats at parse-phase start ({err}); \
                 analysis mode selection may be wrong on polyglot scans"
            );
        }
    }

    for (idx, language) in languages.iter().enumerate() {
        // S1-ε: with a persistent parse DB, NEVER pass `--clear` —
        // that would wipe the incremental cache (and every row from
        // every prior language pass) on every scan, defeating the
        // entire point of persistence. With the legacy ephemeral
        // path we keep the existing behaviour of clearing on the
        // first language so polyglot scans don't accumulate stale
        // rows from previous scratch-dir reuse (which is rare but
        // technically possible with reused scan_ids).
        let clear_db = !db_is_persistent && idx == 0;
        run_parser_for_language(
            &cfg,
            language,
            clear_db,
            &db_path,
            progress.as_mut(),
            &cancel,
        )
        .await?;
    }

    progress.on_event(ProgressEvent::StageFinished {
        stage: Stage::Parsing,
        language: None,
        elapsed_ms: parse_start.elapsed().as_millis() as u64,
    });

    // ---- 5. Analyze phase ------------------------------------------------
    progress.on_event(ProgressEvent::StageStarted {
        stage: Stage::Analyzing,
        language: None,
    });
    let analyze_start = Instant::now();

    run_analyzer(&cfg, &db_path, &report_path, progress.as_mut(), &cancel).await?;

    progress.on_event(ProgressEvent::StageFinished {
        stage: Stage::Analyzing,
        language: None,
        elapsed_ms: analyze_start.elapsed().as_millis() as u64,
    });

    // ---- 6. Read report from disk ---------------------------------------
    let report = read_report_from_disk(&report_path)?;

    Ok(RunPipelineOutput {
        scan_id: cfg.scan_id,
        db_path,
        report_path,
        report,
        languages_parsed: languages,
        languages_skipped,
    })
}

fn reset_incremental_scan_stats_for_parse_phase(db_path: &Path) -> Result<(), RunError> {
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|e| RunError::Io(std::io::Error::other(e.to_string())))?;
    graphengine_parsing::infrastructure::storage::parse_meta_store::reset_incremental_scan_stats(
        &conn,
    )
    .map_err(|e| RunError::Io(std::io::Error::other(e.to_string())))
}

fn validate_binary(kind: BinaryKind, path: &Path) -> Result<(), RunError> {
    if !path.exists() {
        return Err(RunError::BinaryMissing {
            which: kind,
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn read_report_from_disk(path: &Path) -> Result<HealthReport, RunError> {
    let bytes = std::fs::read(path).map_err(RunError::Io)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| RunError::ReportDeserialize(format!("{}: {e}", path.display())))
}

async fn run_parser_for_language(
    cfg: &RunPipelineConfig,
    language: &str,
    clear_db: bool,
    db_path: &Path,
    progress: &mut (dyn ProgressSink + Send),
    cancel: &CancellationToken,
) -> Result<(), RunError> {
    progress.on_event(ProgressEvent::StageStarted {
        stage: Stage::Parsing,
        language: Some(language.to_string()),
    });
    let started = Instant::now();
    let stderr_path = cfg
        .scratch_dir
        .join(format!("{}.parse.{}.stderr", cfg.scan_id, language));

    let mut args: Vec<std::ffi::OsString> = vec![
        "--configs-dir".into(),
        cfg.configs_dir.as_os_str().into(),
        "parse".into(),
        "--root".into(),
        cfg.root.as_os_str().into(),
        "--db".into(),
        db_path.as_os_str().into(),
        "--lang".into(),
        language.into(),
        // Stage 1: ask the parser to emit JSONL on stdout (manifest +
        // per-file events + phase progress). The subprocess driver parses
        // every line through `graphengine_progress::try_parse_line` and
        // surfaces matches as `ProgressEvent::Engine`. Without this flag
        // the parser stays silent on stdout and the CLI/desktop see only
        // the runner's coarse `StageStarted`/`StageFinished` markers.
        "--progress-json".into(),
    ];
    if clear_db {
        args.push("--clear".into());
    }
    // S1-ε: forward the caller's incremental-scan choice to the
    // parser. The parser binary already understands `--no-incremental`
    // (default: cache lookup ON); we only need to add the flag when
    // the runner is asked to disable it. Keeping the default behaviour
    // implicit avoids polluting every scan's process args with a flag
    // the user didn't explicitly set.
    if !cfg.incremental {
        args.push("--no-incremental".into());
    }

    let outcome = subprocess::run_with_progress(subprocess::SubprocessSpec {
        bin: &cfg.parser_bin,
        args: &args,
        stage: Stage::Parsing,
        language: Some(language.to_string()),
        stderr_log_path: stderr_path.clone(),
        progress,
        cancel,
    })
    .await?;

    if !outcome.success {
        return Err(RunError::ParserFailed {
            language: language.to_string(),
            exit_code: outcome.exit_code,
            stderr_log_path: stderr_path,
            stderr_tail: outcome.stderr_tail,
        });
    }

    progress.on_event(ProgressEvent::StageFinished {
        stage: Stage::Parsing,
        language: Some(language.to_string()),
        elapsed_ms: started.elapsed().as_millis() as u64,
    });
    Ok(())
}

async fn run_analyzer(
    cfg: &RunPipelineConfig,
    db_path: &Path,
    report_path: &Path,
    progress: &mut (dyn ProgressSink + Send),
    cancel: &CancellationToken,
) -> Result<(), RunError> {
    let stderr_path = cfg
        .scratch_dir
        .join(format!("{}.analyze.stderr", cfg.scan_id));

    let mut args: Vec<std::ffi::OsString> = vec![
        "--db".into(),
        db_path.as_os_str().into(),
        "--output".into(),
        report_path.as_os_str().into(),
        // Stage 1: ask `ge-analyze` to emit JSONL on stderr at each
        // detector-stage boundary. Analyzer stderr is also the diagnostic
        // log channel, but the runner routes every line through
        // `try_parse_line` first — JSONL becomes structured `Engine`
        // events, non-JSON lines (the legacy `[ge-analyze] Running ...`
        // prose, panics, stack traces) fall through as `Raw`. Both reach
        // the same sink; renderers decide what to show.
        "--progress-json".into(),
    ];
    if cfg.exclude_tests {
        args.push("--exclude-tests".into());
    }
    if cfg.exclude_generated {
        args.push("--exclude-generated".into());
    }
    if let Some(git_dir) = &cfg.git_dir {
        args.push("--git-dir".into());
        args.push(git_dir.as_os_str().into());
    }
    if cfg.persistent_parse_db.is_some() && cfg.incremental {
        args.push("--incremental-analysis".into());
    }
    if cfg.full_analysis {
        args.push("--full-analysis".into());
    }

    let outcome = subprocess::run_with_progress(subprocess::SubprocessSpec {
        bin: &cfg.analyzer_bin,
        args: &args,
        stage: Stage::Analyzing,
        language: None,
        stderr_log_path: stderr_path.clone(),
        progress,
        cancel,
    })
    .await?;

    if !outcome.success {
        return Err(RunError::AnalyzerFailed {
            exit_code: outcome.exit_code,
            stderr_log_path: stderr_path,
            stderr_tail: outcome.stderr_tail,
        });
    }
    Ok(())
}
