//! Emitter abstraction for [`EngineEvent`].
//!
//! Before R3 the parser owned its own emitter trait in
//! `graphengine_parsing::domain::progress` (deleted in R3) plus
//! three concrete implementations (`Null*`, `Stdout*`, `Buffered*`).
//! The analyzer used the bare `emit_line` helper in [`crate`] root.
//! Two parallel emission paths, two parallel trait surfaces, two
//! parallel test-buffering helpers — all targeting the same wire
//! format. R3 collapsed this into one trait + one set of impls
//! living next to the wire format itself.
//!
//! # Why the trait is single-method (not three)
//!
//! The old parser trait had `emit` + `emit_file_manifest` +
//! `emit_file_progress` because its event types were three
//! separate structs. [`EngineEvent`] is already a tagged union —
//! one `emit(EngineEvent)` covers every wire variant and any
//! future variant added under the `#[non_exhaustive]` contract.
//! Callers construct the variant they want via
//! [`EngineEvent::progress`] / [`EngineEvent::progress_lang`] /
//! [`EngineEvent::file_manifest`] / [`EngineEvent::file_progress`]
//! / [`EngineEvent::error`].
//!
//! # Stdout vs stderr targets
//!
//! Parser-side emission goes to **stdout** (its tracing logs use
//! stderr); analyzer-side emission goes to **stderr** (it owns
//! stdout for `--emit-validation` output). The
//! [`StdoutEngineEventEmitter`] honors that split via two named
//! constructors — [`StdoutEngineEventEmitter::to_stdout`] for the
//! parser and [`StdoutEngineEventEmitter::to_stderr`] for the
//! analyzer — rather than hard-coding a single fd or making the
//! caller carry a writer trait object. The name is preserved
//! ("Stdout…") because it is the historical and most common
//! target; the stderr variant is a deliberate exception.
//!
//! # Buffered helper test surface
//!
//! [`BufferedEngineEventEmitter`] preserves the pre-R3 buffered
//! emitter's test ergonomics: variant-filtered
//! accessors (`progress_events`, `file_manifests`,
//! `file_progress_events`, `errors`) plus `all_events` and `clear`.
//! Existing tests port mechanically — only the type names and the
//! event-construction calls change.

use std::io;
use std::sync::{Arc, Mutex};

use crate::EngineEvent;

/// Trait for types that can emit [`EngineEvent`] over a wire (stdout,
/// stderr, in-memory buffer, …).
///
/// `Send + Sync` because the orchestrator stores the emitter behind
/// an `Arc<dyn EngineEventEmitter>` and may share it across rayon
/// worker threads during per-file syntax extraction.
pub trait EngineEventEmitter: Send + Sync {
    /// Emit one event. Concrete implementations decide the target
    /// (stdout / stderr / buffer / null). The contract on the wire
    /// is "one JSONL line per call, flushed before return"; impls
    /// that hit a real file descriptor must call `writer.flush()`
    /// (see [`StdoutEngineEventEmitter`]) so the consumer's
    /// line-oriented reader doesn't stall waiting for an unflushed
    /// buffer.
    fn emit(&self, event: EngineEvent) -> io::Result<()>;

    /// `true` when this emitter will actually do something with an
    /// emitted event. Callers gate expensive event construction on
    /// this so a `NullEngineEventEmitter` doesn't pay the cost of
    /// formatting messages it will immediately discard.
    fn is_enabled(&self) -> bool;
}

/// No-op emitter. Used when progress reporting is disabled (e.g.
/// the parser is invoked as a library by a test that doesn't care
/// about progress, or the CLI is in `--quiet` mode).
#[derive(Debug, Clone, Copy, Default)]
pub struct NullEngineEventEmitter;

impl EngineEventEmitter for NullEngineEventEmitter {
    fn emit(&self, _event: EngineEvent) -> io::Result<()> {
        Ok(())
    }

    fn is_enabled(&self) -> bool {
        false
    }
}

/// Targets stdout or stderr depending on the constructor used.
///
/// `enabled = false` short-circuits without touching the file
/// descriptor — useful for binaries that conditionally enable
/// structured progress via a CLI flag without restructuring the
/// emitter wiring per-call.
#[derive(Debug, Clone)]
pub struct StdoutEngineEventEmitter {
    enabled: bool,
    target: StdoutTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StdoutTarget {
    Stdout,
    Stderr,
}

impl StdoutEngineEventEmitter {
    /// Emit to **stdout**. Use this from the parser — its tracing
    /// logs go to stderr, so stdout is the clean JSONL channel.
    pub fn to_stdout(enabled: bool) -> Self {
        Self {
            enabled,
            target: StdoutTarget::Stdout,
        }
    }

    /// Emit to **stderr**. Use this from `ge-analyze` — it owns
    /// stdout for `--emit-validation` payloads and the report
    /// path-print, so structured progress has to share stderr with
    /// tracing logs (the consumer's line-router distinguishes
    /// JSONL from tracing via the leading-`{` check in
    /// [`crate::try_parse_line`]).
    pub fn to_stderr(enabled: bool) -> Self {
        Self {
            enabled,
            target: StdoutTarget::Stderr,
        }
    }
}

impl EngineEventEmitter for StdoutEngineEventEmitter {
    fn emit(&self, event: EngineEvent) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        match self.target {
            StdoutTarget::Stdout => {
                let mut out = io::stdout().lock();
                event.emit(&mut out)
            }
            StdoutTarget::Stderr => {
                let mut err = io::stderr().lock();
                event.emit(&mut err)
            }
        }
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// In-memory emitter used in tests. Captures every emitted event in
/// the order it was emitted. Always reports `is_enabled = true` so
/// gated event-construction paths still fire under test.
#[derive(Debug, Clone, Default)]
pub struct BufferedEngineEventEmitter {
    events: Arc<Mutex<Vec<EngineEvent>>>,
}

impl BufferedEngineEventEmitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// All emitted events in order, in their original tagged-union
    /// shape. Use the variant-filtered helpers below when you only
    /// care about one variant.
    pub fn all_events(&self) -> Vec<EngineEvent> {
        self.events.lock().unwrap().clone()
    }

    /// Discard every captured event. Useful when a test exercises a
    /// pipeline phase that emits a known prelude and you only want
    /// to assert on what came after a reset point.
    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }

    /// Filter to only [`EngineEvent::Progress`] entries.
    pub fn progress_events(&self) -> Vec<EngineEvent> {
        self.filter(|e| matches!(e, EngineEvent::Progress { .. }))
    }

    /// Filter to only [`EngineEvent::FileManifest`] entries.
    pub fn file_manifests(&self) -> Vec<EngineEvent> {
        self.filter(|e| matches!(e, EngineEvent::FileManifest { .. }))
    }

    /// Filter to only [`EngineEvent::FileProgress`] entries.
    pub fn file_progress_events(&self) -> Vec<EngineEvent> {
        self.filter(|e| matches!(e, EngineEvent::FileProgress { .. }))
    }

    /// Filter to only [`EngineEvent::Error`] entries.
    pub fn errors(&self) -> Vec<EngineEvent> {
        self.filter(|e| matches!(e, EngineEvent::Error { .. }))
    }

    fn filter(&self, pred: impl Fn(&EngineEvent) -> bool) -> Vec<EngineEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| pred(e))
            .cloned()
            .collect()
    }
}

impl EngineEventEmitter for BufferedEngineEventEmitter {
    fn emit(&self, event: EngineEvent) -> io::Result<()> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }

    fn is_enabled(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_emitter_discards_and_reports_disabled() {
        let e = NullEngineEventEmitter;
        assert!(!e.is_enabled());
        e.emit(EngineEvent::progress(50, "test", "start", "x"))
            .unwrap();
    }

    #[test]
    fn buffered_emitter_preserves_order() {
        let e = BufferedEngineEventEmitter::new();
        e.emit(EngineEvent::progress(0, "a", "start", "first"))
            .unwrap();
        e.emit(EngineEvent::progress(100, "a", "done", "second"))
            .unwrap();
        let all = e.all_events();
        assert_eq!(all.len(), 2);
        match &all[0] {
            EngineEvent::Progress { message, .. } => assert_eq!(message, "first"),
            _ => panic!("expected Progress variant"),
        }
    }

    #[test]
    fn buffered_emitter_filters_by_variant() {
        let e = BufferedEngineEventEmitter::new();
        e.emit(EngineEvent::progress(0, "a", "start", "p")).unwrap();
        e.emit(EngineEvent::file_manifest(2, vec!["a".into(), "b".into()]))
            .unwrap();
        e.emit(EngineEvent::file_progress("a", 0, "start")).unwrap();
        e.emit(EngineEvent::error("x", "boom", None)).unwrap();
        assert_eq!(e.progress_events().len(), 1);
        assert_eq!(e.file_manifests().len(), 1);
        assert_eq!(e.file_progress_events().len(), 1);
        assert_eq!(e.errors().len(), 1);
    }

    #[test]
    fn buffered_emitter_clear_resets_to_empty() {
        let e = BufferedEngineEventEmitter::new();
        e.emit(EngineEvent::progress(0, "a", "start", "")).unwrap();
        assert_eq!(e.all_events().len(), 1);
        e.clear();
        assert_eq!(e.all_events().len(), 0);
    }

    #[test]
    fn stdout_emitter_short_circuits_when_disabled() {
        // We can't easily assert on stdout/stderr writes in a unit
        // test, but the disabled path must never touch the file
        // descriptor — that's tested implicitly by the fact that
        // this test does not panic on a closed-stdout environment.
        let e = StdoutEngineEventEmitter::to_stdout(false);
        assert!(!e.is_enabled());
        e.emit(EngineEvent::progress(0, "a", "start", "")).unwrap();
        let e = StdoutEngineEventEmitter::to_stderr(false);
        assert!(!e.is_enabled());
        e.emit(EngineEvent::progress(0, "a", "start", "")).unwrap();
    }
}
