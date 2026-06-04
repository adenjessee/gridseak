//! Structured-progress integration test.
//!
//! Asserts that with Stage 1 wired:
//!
//! 1. The parser's `--progress-json` stdout emission is parsed by the
//!    runner's subprocess driver into `ProgressEvent::Engine` events
//!    (not `ProgressEvent::Raw`).
//! 2. The analyzer's `--progress-json` stderr emission is also parsed
//!    into `ProgressEvent::Engine` events, so the consumer sees a
//!    unified structured stream regardless of which engine produced
//!    the event.
//! 3. Both engines flow per-stage events through: the parser must emit
//!    at least one `Progress` event during its pipeline; the analyzer
//!    must emit `Progress` events whose `phase` matches the canonical
//!    detector names (`cycle_detection`, `fan_metrics`, etc.).
//! 4. Unparseable stderr lines (tracing logs, banners) still arrive as
//!    `ProgressEvent::Raw` — the runner doesn't drop them. This guards
//!    the "structured first, raw fallback" contract.
//!
//! If this test starts failing while the per-crate tests stay green,
//! the integration boundary regressed (e.g. the parser stopped passing
//! `--progress-json`, the analyzer's progress module got mis-wired,
//! or `try_parse_line` lost an event shape).

mod common;

use common::{ensure_engine_binaries, pipeline_config};
use gridseak_engine_runner::{
    progress::{ProgressEvent, ProgressSink},
    run_pipeline, EngineEvent, Stage,
};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Sink that captures every event into a `Vec` for post-run analysis.
/// `Arc<Mutex<Vec<...>>>` because the runner wants `Box<dyn ProgressSink
/// + Send + Sync>` and we need to inspect the captured stream after
/// `run_pipeline` returns. Cheap, contended only once-per-event.
#[derive(Clone)]
struct RecordingSink {
    events: Arc<Mutex<Vec<ProgressEvent>>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn events(&self) -> Vec<ProgressEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl ProgressSink for RecordingSink {
    fn on_event(&mut self, event: ProgressEvent) {
        self.events.lock().unwrap().push(event);
    }
}

#[tokio::test]
async fn parser_emits_structured_engine_events() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch tempdir");
    let scan_id = Uuid::new_v4();
    let mut cfg = pipeline_config(scratch.path(), scan_id, vec!["rust".into()]);
    let sink = RecordingSink::new();
    cfg.progress = Box::new(sink.clone());

    run_pipeline(cfg).await.expect("pipeline runs");

    let events = sink.events();

    // At least one Engine event with a stage of Parsing must appear.
    // If the parser regressed to silent stdout (or `--progress-json`
    // got dropped from the args), this assertion would catch it
    // because every line would land in `Raw` instead.
    let any_parser_engine_event = events.iter().any(|e| {
        matches!(
            e,
            ProgressEvent::Engine {
                stage: Stage::Parsing,
                event: EngineEvent::Progress { .. }
                    | EngineEvent::FileManifest { .. }
                    | EngineEvent::FileProgress { .. },
                ..
            }
        )
    });
    assert!(
        any_parser_engine_event,
        "expected at least one ProgressEvent::Engine during the Parsing stage; \
         received {} total events, of which {} were Engine variants. \
         If only Raw events arrived, check that gridseak-engine-runner is \
         passing `--progress-json` to graphengine-parsing.",
        events.len(),
        events
            .iter()
            .filter(|e| matches!(e, ProgressEvent::Engine { .. }))
            .count()
    );
}

#[tokio::test]
async fn analyzer_emits_structured_engine_events() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch tempdir");
    let scan_id = Uuid::new_v4();
    let mut cfg = pipeline_config(scratch.path(), scan_id, vec!["rust".into()]);
    let sink = RecordingSink::new();
    cfg.progress = Box::new(sink.clone());

    run_pipeline(cfg).await.expect("pipeline runs");

    let events = sink.events();

    // The analyzer's instrumented stages should appear as Engine
    // events with `stage: Analyzing`. We don't require every detector
    // to fire (some are conditional on `repo_classification`), but
    // we do require at least one — `loading` is unconditional.
    let analyzer_phases: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::Engine {
                stage: Stage::Analyzing,
                event: EngineEvent::Progress { phase, .. },
                ..
            } => Some(phase.clone()),
            _ => None,
        })
        .collect();

    assert!(
        !analyzer_phases.is_empty(),
        "expected at least one ProgressEvent::Engine during Analyzing with \
         a Progress payload; got none. If only Raw events arrived from the \
         analyzer, check that ge-analyze receives --progress-json and that \
         graphengine_analysis::health::progress::enable() flips the toggle."
    );

    // Spot-check that the canonical `loading` phase appears.
    // Implementation detail: the analyzer emits this from the very
    // first eprintln boundary, so any non-trivial pipeline run will
    // include it.
    assert!(
        analyzer_phases.iter().any(|p| p == "loading"),
        "expected the analyzer's `loading` phase among emitted phases; \
         observed = {analyzer_phases:?}"
    );
}

#[tokio::test]
async fn unparseable_stderr_still_surfaces_as_raw() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch tempdir");
    let scan_id = Uuid::new_v4();
    let mut cfg = pipeline_config(scratch.path(), scan_id, vec!["rust".into()]);
    let sink = RecordingSink::new();
    cfg.progress = Box::new(sink.clone());

    run_pipeline(cfg).await.expect("pipeline runs");

    let events = sink.events();

    // Both engines write some non-JSON output to stderr (the parser's
    // tracing logs; the analyzer's `[ge-analyze] ...` banners). Those
    // lines must NOT be silently dropped — they have to arrive as
    // `Raw` so a future verbose renderer can show them.
    let raw_count = events
        .iter()
        .filter(|e| matches!(e, ProgressEvent::Raw { .. }))
        .count();

    assert!(
        raw_count > 0,
        "expected at least one ProgressEvent::Raw to flow through; got 0. \
         The subprocess driver must forward unparseable stderr lines as \
         Raw, not drop them."
    );
}

#[tokio::test]
async fn stage_lifecycle_events_still_fire() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch tempdir");
    let scan_id = Uuid::new_v4();
    let mut cfg = pipeline_config(scratch.path(), scan_id, vec!["rust".into()]);
    let sink = RecordingSink::new();
    cfg.progress = Box::new(sink.clone());

    run_pipeline(cfg).await.expect("pipeline runs");

    let events = sink.events();

    // The runner itself emits Stage{Started,Finished} for Preparing /
    // Parsing / Analyzing. These are independent of engine emission;
    // structured engine events flowing in does NOT regress the
    // lifecycle stream.
    let stage_started_count = events
        .iter()
        .filter(|e| matches!(e, ProgressEvent::StageStarted { .. }))
        .count();
    let stage_finished_count = events
        .iter()
        .filter(|e| matches!(e, ProgressEvent::StageFinished { .. }))
        .count();

    assert!(
        stage_started_count >= 3,
        "expected at least 3 StageStarted events (Preparing, Parsing, Analyzing), got {stage_started_count}"
    );
    assert!(
        stage_finished_count >= 3,
        "expected at least 3 StageFinished events, got {stage_finished_count}"
    );
}
