//! Inputs and outputs for [`crate::run_pipeline`].
//!
//! All paths are caller-resolved. The runner does not search for binaries,
//! configs, or scratch dirs — that's the consumer's responsibility (CLI
//! has its own discovery rules; desktop uses Tauri sidecar resolution).
//! Making this a hard rule keeps the runner trivially testable: a unit
//! test or fixture-based integration test just builds a config with
//! `tempfile::tempdir()` paths and goes.

use std::path::PathBuf;

use graphengine_analysis::health::report::HealthReport;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::progress::ProgressSink;

/// Fully resolved inputs for one pipeline run.
///
/// Construct with positional fields directly — public, no builder. This is a
/// workspace-internal crate; a builder would be over-engineering.
pub struct RunPipelineConfig {
    /// Repository root to scan.
    pub root: PathBuf,

    /// Languages the consumer believes are present in the repo.
    /// The runner will filter `discovery_only` languages internally;
    /// what survives lands in [`RunPipelineOutput::languages_parsed`].
    pub languages: Vec<String>,

    /// Path to the `graphengine-parsing` binary. Must exist; the runner
    /// returns [`crate::RunError::BinaryMissing`] if it doesn't.
    pub parser_bin: PathBuf,

    /// Path to the `ge-analyze` binary. Must exist (same contract).
    pub analyzer_bin: PathBuf,

    /// Directory containing language YAML configs (passed to parser via
    /// `--configs-dir`).
    pub configs_dir: PathBuf,

    /// Where the runner writes the report JSON and the per-stage stderr
    /// logs. Created if absent. By default the parse SQLite DB also
    /// lands here as `{scan_id}.sqlite` (ephemeral, per-scan); set
    /// [`Self::persistent_parse_db`] to redirect the parse DB to a
    /// stable per-project location so S1 incremental scanning can
    /// reuse the `file_cache` table across scans.
    pub scratch_dir: PathBuf,

    /// Optional persistent parse DB path. When `Some`, the runner
    /// writes the parse SQLite DB to this path instead of
    /// `scratch_dir/{scan_id}.sqlite`, and DROPS the `--clear` flag
    /// on the first language pass so the cache survives across
    /// scans. The schema-version bump path
    /// (`PARSE_META_SCHEMA_VERSION`) handles upgrade-time
    /// invalidation; per-file row pruning
    /// (`prune_files_from_graph`) keeps node/edge rows in sync with
    /// the incremental scan plan. The report JSON and stderr logs
    /// continue to land under `scratch_dir` because they are
    /// per-scan deliverables. `None` preserves the legacy
    /// per-scan-ephemeral behaviour for desktop and integration
    /// tests that don't have a project id yet.
    pub persistent_parse_db: Option<PathBuf>,

    /// Caller-supplied scan id. The runner uses this for filenames
    /// (`{scan_id}.report.json`, `{scan_id}.parse.<lang>.stderr`,
    /// and `{scan_id}.sqlite` when [`Self::persistent_parse_db`] is
    /// `None`) and includes it in [`RunPipelineOutput`]. Generating
    /// the id is a consumer concern — desktop persists the id via
    /// `begin_scan` before the pipeline runs so the UI can link to
    /// in-progress state.
    pub scan_id: Uuid,

    /// Pass `--exclude-tests` to the analyzer. Defaults differ between
    /// surfaces today; the parity guarantee is "both surfaces always pass
    /// this through to the analyzer the same way."
    pub exclude_tests: bool,

    /// Pass `--exclude-generated` to the analyzer.
    pub exclude_generated: bool,

    /// S1 incremental scanning toggle. `true` (default) lets the
    /// parser orchestrator consult its `file_cache` table and skip
    /// re-extracting unchanged files; `false` translates to
    /// `--no-incremental` on the parser invocation, forcing a full
    /// reparse. Wired in by the CLI's `gridseak scan
    /// --no-incremental` opt-out so a user investigating cache
    /// staleness can bypass it without rebuilding the parser. The
    /// flag is independent of [`Self::persistent_parse_db`]: a
    /// persistent DB with `incremental: false` still benefits from
    /// schema-version-bump invalidation but reparses every file
    /// this run.
    pub incremental: bool,

    /// S2-γ: force full analysis (L3); forwarded to ge-analyze as `--full-analysis`.
    pub full_analysis: bool,

    /// Optional git directory for analyzer's temporal-coupling pass. When
    /// present, analyzer reads `git log --name-only` from `{git_dir}` and
    /// produces co-change signals. When absent, temporal metrics are
    /// reported with `MetricStatus::Unmeasured` so the report is still
    /// honest.
    pub git_dir: Option<PathBuf>,

    /// Where to send progress events. The runner emits stage-lifecycle
    /// events plus raw stderr lines from each subprocess. Stage 1 of the
    /// shadow-mode plan graduates this to a structured event shape.
    /// `Send + Sync` because consumers (CLI, desktop) spawn the runner
    /// inside `tokio::spawn`, which requires `&RunPipelineConfig: Send`,
    /// which requires every field to be `Sync`. All in-tree sinks
    /// (`CliProgressSink`, `DesktopProgressSink`, `DiscardSink`) are
    /// trivially `Sync`; future sinks must be too.
    pub progress: Box<dyn ProgressSink + Send + Sync>,

    /// Optional cancellation token. When fired, the runner kills any
    /// in-flight child process and returns [`crate::RunError::Cancelled`].
    /// `tokio_util::sync::CancellationToken` (rather than
    /// `tokio::sync::oneshot::Receiver`) because it can be awaited
    /// repeatedly across the parser-per-language and analyzer phases.
    pub cancel: Option<CancellationToken>,
}

/// Result of one successful pipeline run.
///
/// `Debug` is derived purely so test assertions like `expect_err(...)`
/// compile (`Result::expect_err` requires `T: Debug`). It also makes
/// the type easier to log defensively from consumers.
#[derive(Debug)]
pub struct RunPipelineOutput {
    pub scan_id: Uuid,

    /// Path to the SQLite graph DB the parser produced.
    pub db_path: PathBuf,

    /// Path to the analyzer's JSON report.
    pub report_path: PathBuf,

    /// In-memory deserialized report. Same content as `report_path`'s
    /// JSON; provided so consumers don't have to re-read and re-parse.
    pub report: HealthReport,

    /// Languages that were actually parsed, in order.
    pub languages_parsed: Vec<String>,

    /// Languages dropped from the request because the registry flagged
    /// them `discovery_only`. Surfaced so consumers can show a "skipped
    /// X (handled by Y)" note in their UI.
    pub languages_skipped: Vec<String>,
}
