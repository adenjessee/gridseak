//! Process-global progress emission for `ge-analyze`.
//!
//! # What this is
//!
//! `ge-analyze` runs as a subprocess of `gridseak-engine-runner`. The
//! runner forwards every line of the analyzer's stderr through
//! [`graphengine_progress::try_parse_line`]; lines that deserialize as
//! [`graphengine_progress::EngineEvent`] surface in the CLI/desktop as
//! structured `Engine` events (real percentages, stage boundaries),
//! and everything else falls through to `Raw` for verbatim forwarding.
//!
//! The analyzer's detector pipeline ([`crate::health::run_analysis_with_config`])
//! is a long chain of `eprintln!("[ge-analyze] Running ...")` checkpoints.
//! Threading a sink argument through every detector would be a several-
//! hundred-line refactor for zero behavioral gain over a process-global
//! toggle: the analyzer is a one-shot subprocess, and the runner spawns
//! a fresh process per scan, so there's no cross-process contention.
//!
//! Stage 1 of the shadow-mode plan therefore lands the simplest thing
//! that works: a single [`AtomicBool`] that `ge-analyze`'s `--progress-json`
//! flag flips, plus stage-emit helpers the detector pipeline calls at
//! each phase boundary. The lifetime of the toggle is the lifetime of
//! the process — flipped during `main()` before any detector runs,
//! never flipped back.
//!
//! # Wire format
//!
//! Emitted events are [`graphengine_progress::EngineEvent::Progress`]
//! with `phase` set to the detector identifier (e.g. `cycle_detection`,
//! `fan_metrics`, `dead_code`, `complexity`, `health_score`) and `status`
//! set to `"start"` or `"done"`. `language` is always `None` for
//! analyzer events (analysis runs over the merged graph; language is
//! implicit in the parsed DB).
//!
//! Why `Progress` rather than a separate `StageStarted`/`StageFinished`
//! enum variant: the runner already owns stage-boundary events at its
//! own granularity (`Stage::Analyzing` starts when the analyzer
//! subprocess is spawned, finishes when it exits). The events emitted
//! here are *within-stage* progress with a free-form phase string the
//! renderer can map to a percent. Adding analyzer-specific stage
//! variants would force the runner to learn about the analyzer's
//! internal phase taxonomy, which is the opposite of the layering the
//! shared [`EngineEvent`] vocabulary establishes.

use std::io::{self};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use graphengine_progress::EngineEvent;

/// Process-global toggle. `ge-analyze`'s `--progress-json` flag flips
/// this before any detector runs; nothing else touches it. `Relaxed`
/// because there's no ordering relationship to enforce — a flip is
/// idempotent and read-mostly, and we don't care about strict
/// happens-before across threads.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Activate JSONL stage-emission for the remainder of the process.
/// Called once from `ge-analyze::main` when `--progress-json` is passed.
/// Subsequent calls are no-ops (the toggle is set-only by design).
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// `true` if `--progress-json` was passed on this invocation. Renderers
/// inside the analysis library should keep using `eprintln!` for human-
/// readable diagnostics unconditionally; this toggle only gates the
/// *structured* JSONL emission that runs alongside.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Emit a `Progress { status: "start" }` event for a named phase.
///
/// `phase` is the canonical detector name (e.g. `cycle_detection`).
/// `percent` is the cumulative coarse progress across the analyzer's
/// detectors; callers are responsible for picking values that monotone
/// upward. The renderer treats this as a hint, not a contract — if a
/// stage skips ahead the bar just jumps.
pub fn emit_stage_started(phase: &str, percent: u8, message: &str) -> Instant {
    if is_enabled() {
        let _ = emit(&EngineEvent::progress(percent, phase, "start", message));
    }
    Instant::now()
}

/// Emit a `Progress { status: "done" }` event for a named phase. Pass
/// the [`Instant`] returned by the matching [`emit_stage_started`] so
/// the wire event carries an elapsed-time message; renderers can show
/// per-stage timing without computing it themselves.
pub fn emit_stage_finished(phase: &str, percent: u8, started: Instant) {
    if !is_enabled() {
        return;
    }
    let elapsed_ms = started.elapsed().as_millis();
    let message = format!("{phase} done in {elapsed_ms}ms");
    let _ = emit(&EngineEvent::progress(percent, phase, "done", message));
}

/// Emit a free-form mid-stage `Progress { status: "progress" }` event.
/// Use when a detector has internal sub-progress worth surfacing (e.g.
/// per-cycle iteration counts in `cycle_detection`). Most detectors
/// will not need this — the start/finish pair is enough.
pub fn emit_progress(phase: &str, percent: u8, message: &str) {
    if is_enabled() {
        let _ = emit(&EngineEvent::progress(percent, phase, "progress", message));
    }
}

/// Emit a typed error event. Distinct from `eprintln!` of a panic
/// message because the renderer can route this to a "failed during X"
/// banner; the process still exits non-zero via the binary's normal
/// error path, but the structured event lands first so the UI knows
/// which detector failed.
pub fn emit_error(phase: &str, message: &str) {
    if !is_enabled() {
        return;
    }
    let event = EngineEvent::Error {
        phase: phase.into(),
        message: message.into(),
        code: None,
        language: None,
    };
    let _ = emit(&event);
}

/// Write one JSONL event to stderr. Stderr is the right channel because
/// `ge-analyze` reserves stdout for `--emit-validation` JSON output
/// (see `bin/ge_analyze.rs`). All other diagnostic output already goes
/// to stderr via `eprintln!`, so interleaving JSONL there preserves a
/// single chronological ordering for the consumer.
fn emit(event: &EngineEvent) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    event.emit(&mut stderr)
}

/// Convenience: emit a `Progress { status: "done" }` event without an
/// elapsed timer. Used by code paths that don't know the stage start
/// (e.g. when the analyzer skips a detector entirely because a feature
/// flag is off). The renderer treats the event the same way.
pub fn emit_stage_skipped(phase: &str, percent: u8, message: &str) {
    if !is_enabled() {
        return;
    }
    // We tag skipped stages with `status: "done"` rather than introducing
    // a new wire status: the renderer's distinction is start→finish; a
    // skip is "finished without doing anything," which is the same shape
    // from the bar's perspective.
    let _ = emit(&EngineEvent::progress(percent, phase, "done", message));
}

/// Hard reset for tests. Tests that spin up multiple analyzer-like
/// fixtures inside one process must clear the toggle between cases or
/// they'll see structured emission from a prior test bleed into the
/// next. Production binaries never call this.
///
/// `#[doc(hidden)]` because this is a testing seam, not a public API.
#[doc(hidden)]
pub fn _reset_for_tests() {
    ENABLED.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        _reset_for_tests();
        assert!(!is_enabled());
    }

    #[test]
    fn enable_flips_the_toggle() {
        _reset_for_tests();
        assert!(!is_enabled());
        enable();
        assert!(is_enabled());
        _reset_for_tests();
    }

    #[test]
    fn emit_helpers_are_noop_when_disabled() {
        _reset_for_tests();
        // Should not panic; should not write anything. We can't easily
        // capture stderr inside a test (Rust's test runner already
        // hooks it), so the assertion is "doesn't panic and returns" —
        // any side effect on stderr would be visible in CI as noise.
        emit_stage_started("test_phase", 10, "starting");
        emit_progress("test_phase", 50, "halfway");
        emit_stage_finished("test_phase", 100, Instant::now());
        emit_error("test_phase", "fake error");
        emit_stage_skipped("test_phase", 100, "skipped");
    }
}
