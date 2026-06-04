//! Progress-event contract for [`crate::run_pipeline`].
//!
//! # Layering
//!
//! Stage 0 of the shadow-mode plan introduced this trait with two
//! variants: stage lifecycle from the runner itself, and `Raw` lines
//! forwarded verbatim from subprocess stderr/stdout. Stage 1 added the
//! [`graphengine_progress::EngineEvent`] vocabulary: the parser and
//! analyzer emit one-line JSON events to their stdout/stderr, and the
//! [`crate::subprocess`] driver attempts to parse each line as an engine
//! event before falling back to `Raw`. Successful parses surface as
//! [`ProgressEvent::Engine`] so consumers (CLI renderer, desktop sink)
//! can show truthful percentages, file counters, and current-file paths
//! without scraping prose.
//!
//! Consumers do *not* depend on the exact set of variants today. The
//! enum is `#[non_exhaustive]` so future stages can add fields without
//! breaking desktop/CLI's `match` arms. With Stage 1 wired, both the
//! [`gridseak_cli::progress::CliProgressSink`] and the desktop's
//! `DesktopProgressSink` have explicit `match` arms for `Engine` —
//! per the plan's update notes, the `_ => {}` wildcard arms have been
//! removed so any new variant added in later stages forces a typecheck
//! review at every consumer.

/// Lifecycle stages emitted by the runner. The runner orchestrates
/// `Preparing` (registry load + filtering) → `Parsing` (one
/// invocation per language) → `Analyzing` (single invocation).
/// Cancellation and failure are signalled via the [`crate::RunError`]
/// return path, not as additional stages — they are events about the
/// pipeline's outcome, not phases of normal execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Preparing,
    Parsing,
    Analyzing,
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stage::Preparing => f.write_str("preparing"),
            Stage::Parsing => f.write_str("parsing"),
            Stage::Analyzing => f.write_str("analyzing"),
        }
    }
}

/// Events the runner emits to its [`ProgressSink`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ProgressEvent {
    /// A stage just began. For `Parsing`, `language` is `None` for the
    /// stage-wide marker and `Some(name)` for each per-language sub-event
    /// emitted before that language's subprocess spawns. Emitted by the
    /// runner itself — engines don't know about stage boundaries; the
    /// runner owns the timing between parser and analyzer invocations.
    StageStarted {
        stage: Stage,
        language: Option<String>,
    },

    /// A stage finished. `language` mirrors the matching `StageStarted`
    /// — `None` is the stage-wide marker; `Some(name)` is the per-language
    /// sub-stage that just completed. `elapsed_ms` is the wall-clock
    /// duration of the stage (or sub-stage).
    StageFinished {
        stage: Stage,
        language: Option<String>,
        elapsed_ms: u64,
    },

    /// A structured progress event parsed off a subprocess stream.
    ///
    /// The runner attempts [`graphengine_progress::try_parse_line`] on
    /// every line forwarded from the parser's stdout and the analyzer's
    /// stderr. Successful parses surface here; failures fall through to
    /// [`ProgressEvent::Raw`]. The `stage` field records which subprocess
    /// produced the event so consumers can route a parser
    /// `Progress { phase: "pipeline" }` event differently from an
    /// analyzer `Progress { phase: "cycle_detection" }` event, even
    /// though both are `EngineEvent::Progress` variants on the wire.
    Engine {
        stage: Stage,
        language: Option<String>,
        event: graphengine_progress::EngineEvent,
    },

    /// One line of stdout/stderr captured from an active subprocess that
    /// could not be parsed as an [`graphengine_progress::EngineEvent`].
    /// This is the fallback path for tracing logs, panics, stack traces,
    /// and any other unstructured output. Renderers typically show these
    /// only in `--verbose` mode (the structured `Engine` stream is what
    /// drives the default progress UI).
    Raw {
        stage: Stage,
        language: Option<String>,
        line: String,
    },
}

/// Sink for progress events. Consumers implement this to render to their
/// environment.
///
/// `Send` is required because the runner streams events from a tokio
/// task; the sink crosses an `await` boundary while reading subprocess
/// stderr. Sync is *not* required — desktop's impl wraps a `tauri::AppHandle`
/// (which is `Send + Sync`); CLI's impl just writes to stderr.
///
/// `&mut self` is required because some sinks may want to maintain
/// mutable state (e.g. a ratatui-style UI counting current-file
/// throughput). The runner holds the sink uniquely for the duration of
/// the pipeline run.
pub trait ProgressSink: Send {
    fn on_event(&mut self, event: ProgressEvent);
}

/// A no-op sink. Useful in tests and in non-interactive CLI modes
/// (`--quiet`).
pub struct DiscardSink;
impl ProgressSink for DiscardSink {
    fn on_event(&mut self, _event: ProgressEvent) {}
}
