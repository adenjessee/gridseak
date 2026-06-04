//! Shared progress-event vocabulary for the scan pipeline.
//!
//! # Why this crate exists
//!
//! Before this crate, the parser emitted structured JSON events on stdout
//! using its own private types (`graphengine_parsing::domain::progress`,
//! removed in R3), the analyzer emitted `eprintln!` stage lines on
//! stderr, the desktop shell scraped `[progress] ...` lines with a
//! regex, and `gridseak-engine-runner` forwarded everything verbatim as
//! `ProgressEvent::Raw`. Three formats, three consumers, zero shared
//! vocabulary; the CLI and desktop could not show truthful progress
//! for a parsing-heavy repo without each maintaining their own decoder.
//!
//! This crate is **the** wire format. Every engine binary (parser today,
//! analyzer next) emits these events as one-line JSON to a single stream
//! (parser uses stdout because tracing logs go to stderr; analyzer uses
//! stderr because it owns stdout for `--emit-validation`). The runner's
//! subprocess driver attempts [`try_parse_line`] on every line off both
//! streams; successful parses graduate to a structured event variant in
//! the runner, unparseable lines stay as `Raw` for verbatim forwarding.
//!
//! # What's deliberately *not* here
//!
//! - **Stage lifecycle is the runner's, not the engine's.** "Stage
//!   started"/"Stage finished" are emitted by `gridseak-engine-runner`
//!   itself (it owns the timing boundaries between parser and analyzer
//!   invocations). Engines emit *within-stage* progress only.
//! - **No transport.** The engine emits via [`emit_line`]; the consumer
//!   parses via [`try_parse_line`]. Anything fancier (in-memory queue,
//!   async streams) belongs to the runner.
//! - **No rendering.** The CLI and desktop each interpret these events
//!   for their own UI surface; this crate does not concatenate, paginate,
//!   throttle, or color anything.
//!
//! # Stability contract
//!
//! [`EngineEvent`] is `#[non_exhaustive]`. Variants can be added without
//! breaking consumers, but field shapes within an existing variant are a
//! breaking change to JSONL on the wire and must be coordinated across
//! all engine binaries and consumer surfaces simultaneously.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

mod emitter;

pub use emitter::{
    BufferedEngineEventEmitter, EngineEventEmitter, NullEngineEventEmitter,
    StdoutEngineEventEmitter,
};

/// One progress event emitted by an engine binary.
///
/// The `tag = "type"` discriminator is on the wire so a streaming parser
/// can route a malformed/extended event without knowing every variant.
/// All variants use `rename_all = "snake_case"` so the wire format is
/// stable regardless of Rust identifier conventions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EngineEvent {
    /// Coarse-grained "I'm at percent X of phase Y" event.
    ///
    /// Parser emits one of these per high-level pipeline phase (Discovery,
    /// Pipeline, Resolution, Graph, Export, …). Analyzer emits one per
    /// detector pass (cycle detection, fan metrics, dead-code, …).
    /// `status` distinguishes the lifecycle position so a single phase
    /// can emit `start` → repeated `progress` → `done`/`error`.
    Progress {
        /// Estimated completion percentage 0–100. Construct via
        /// [`EngineEvent::progress`] to enforce the cap; raw JSON with
        /// values past 100 deserializes successfully (we don't want a
        /// malformed upstream event to crash the consumer), but
        /// renderers should clamp before display.
        percent: u8,
        /// Free-form phase identifier (e.g. `parse`, `pipeline`,
        /// `discovery`, `cycle_detection`, `fan_metrics`, `health_score`).
        /// Kept as a `String` rather than a typed enum so the analyzer
        /// can introduce new detector names without re-releasing this
        /// crate. Renderers fall back to verbatim display for unknown
        /// phases.
        phase: String,
        /// `start` | `progress` | `done` | `error` | `clear`.
        /// String for the same forward-compat reason as `phase`.
        status: String,
        /// Human-readable description. Renderers may show verbatim or
        /// drop on narrow terminals.
        message: String,
        /// Set by the parser to the language it's currently processing
        /// (`typescript`, `python`, etc.). Absent for analyzer events
        /// (analysis runs over the merged graph; language is implicit).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },

    /// One-shot manifest of every file the parser is about to process,
    /// emitted after discovery and before file-by-file parsing.
    ///
    /// Renderers use `total_files` as the denominator for the file
    /// counter that follows. `files` is the full list of relative paths;
    /// it can be large for monorepos, so a renderer that only needs the
    /// count should ignore it.
    FileManifest {
        total_files: usize,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },

    /// One file's lifecycle within the parser's syntax-extraction pass.
    /// `status` is `start` | `done` | `error`. `file_index` is 0-based
    /// against the matching [`EngineEvent::FileManifest`].
    FileProgress {
        file_path: String,
        file_index: usize,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },

    /// Engine-level error event, distinct from `Progress { status: "error" }`.
    /// Reserved for unrecoverable failures the engine wants the renderer
    /// to surface immediately. The engine still exits with non-zero on
    /// fatal errors; this event makes the cause visible *before* the
    /// process termination is observable by the runner.
    Error {
        phase: String,
        message: String,
        /// Optional stable code for programmatic dispatch (e.g.
        /// `parser.missing_grammar`). Renderers may show verbatim.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },

    /// One-shot summary of the S1 incremental-scan cache lookup.
    /// Emitted by the parser right after the incremental planner
    /// classifies discovered files against the previous parse DB's
    /// `file_cache` rows. Renderers surface it as a single line so
    /// the user can see how much extraction the cache saved.
    ///
    /// `disabled = true` is reserved for the `--no-incremental` CLI
    /// flag (and for cold-cache scans where there's nothing to hit
    /// against). Renderers should suppress hit-rate noise when
    /// disabled.
    CacheStats {
        /// Number of source files this scan visited
        /// (`cached + reparsed`). Convenience for renderers that
        /// want to compute hit rate without summing.
        total_files: usize,
        /// Files whose cached extraction slice was reused.
        cached: usize,
        /// Files re-extracted from source (cache miss or absent).
        reparsed: usize,
        /// Cache rows pruned because the file no longer exists.
        removed: usize,
        /// True when the incremental path was bypassed
        /// (--no-incremental, schema bump invalidation, etc.).
        /// Renderers may render the line differently or skip it.
        #[serde(default)]
        disabled: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },

    /// S2-β analysis mode line for CLI progress (`analysis: segmented (N files)`).
    AnalysisMode {
        mode: String,
        delta_files: usize,
        message: String,
    },
}

impl EngineEvent {
    /// Build a [`EngineEvent::Progress`] with the percent clamped to
    /// `[0, 100]`. Use this from engine code instead of the raw struct
    /// literal so an off-by-one stage counter cannot emit 110%.
    pub fn progress(
        percent: u8,
        phase: impl Into<String>,
        status: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        EngineEvent::Progress {
            percent: percent.min(100),
            phase: phase.into(),
            status: status.into(),
            message: message.into(),
            language: None,
        }
    }

    /// Same as [`EngineEvent::progress`] but tags the event with a language.
    /// Used by the parser; analyzer events leave `language = None`.
    pub fn progress_lang(
        percent: u8,
        phase: impl Into<String>,
        status: impl Into<String>,
        message: impl Into<String>,
        language: impl Into<String>,
    ) -> Self {
        EngineEvent::Progress {
            percent: percent.min(100),
            phase: phase.into(),
            status: status.into(),
            message: message.into(),
            language: Some(language.into()),
        }
    }

    /// Build a [`EngineEvent::FileManifest`]. `total_files` is
    /// expected to equal `files.len()` (the consumer uses
    /// `total_files` as the denominator for the per-file counter
    /// that follows); we don't assert because some integration
    /// tests deliberately emit a count without listing every path.
    pub fn file_manifest(total_files: usize, files: Vec<String>) -> Self {
        EngineEvent::FileManifest {
            total_files,
            files,
            language: None,
        }
    }

    /// Build a [`EngineEvent::FileProgress`]. `status` is `start` |
    /// `done` | `error`; kept as `&str` so callers don't need to
    /// import a typed enum just to emit one event.
    pub fn file_progress(
        file_path: impl Into<String>,
        file_index: usize,
        status: impl Into<String>,
    ) -> Self {
        EngineEvent::FileProgress {
            file_path: file_path.into(),
            file_index,
            status: status.into(),
            language: None,
        }
    }

    /// Build a [`EngineEvent::Error`]. `code` is optional (e.g.
    /// `parser.missing_grammar`); pass `None` when the error is
    /// ad-hoc and not yet promoted to a programmatic code.
    pub fn error(
        phase: impl Into<String>,
        message: impl Into<String>,
        code: Option<String>,
    ) -> Self {
        EngineEvent::Error {
            phase: phase.into(),
            message: message.into(),
            code,
            language: None,
        }
    }

    /// Build a [`EngineEvent::CacheStats`] for the active scan. The
    /// arithmetic invariant `total_files == cached + reparsed` is
    /// enforced via debug_assert; in release builds a mismatch
    /// passes through (the renderer can still display the values).
    pub fn cache_stats(cached: usize, reparsed: usize, removed: usize) -> Self {
        let total_files = cached + reparsed;
        EngineEvent::CacheStats {
            total_files,
            cached,
            reparsed,
            removed,
            disabled: false,
            language: None,
        }
    }

    /// Build a [`EngineEvent::CacheStats`] for a scan where the
    /// incremental path is bypassed (--no-incremental or
    /// schema-bump invalidation). `total_files` is the discovered
    /// file count.
    pub fn cache_stats_disabled(total_files: usize) -> Self {
        EngineEvent::CacheStats {
            total_files,
            cached: 0,
            reparsed: total_files,
            removed: 0,
            disabled: true,
            language: None,
        }
    }

    /// Serialize this event as one line of JSON terminated by a single
    /// `\n`. Renderers consume the wire format one line at a time, so
    /// emitters must never embed literal newlines inside the JSON
    /// payload. `serde_json::to_string` already guarantees that (escapes
    /// `\n` inside strings), so this is correct by construction; the
    /// trailing newline here is what frames the line on the receiving
    /// side.
    pub fn to_jsonl(&self) -> Result<String, serde_json::Error> {
        let mut s = serde_json::to_string(self)?;
        s.push('\n');
        Ok(s)
    }

    /// Write this event as one JSONL line to `writer` and flush.
    /// The flush matters: the consumer is reading line-by-line off a
    /// pipe with no further buffering, so leaving bytes in the writer's
    /// internal buffer makes the renderer appear stuck.
    pub fn emit<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let jsonl = self
            .to_jsonl()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        writer.write_all(jsonl.as_bytes())?;
        writer.flush()
    }
}

/// Convenience: write the event to stderr as JSONL.
///
/// Used by `ge-analyze` (which owns stdout for `--emit-validation` and
/// for the report path-print, so stderr is the only safe progress channel).
/// Parser-side emission goes through [`StdoutEngineEventEmitter::to_stdout`]
/// (parser keeps stderr clean for tracing logs); this helper exists for
/// the analyzer's eprintln-replacement call sites that don't want to
/// thread an emitter through every function.
pub fn emit_line(event: &EngineEvent) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    event.emit(&mut stderr)
}

/// Best-effort: try to parse `line` as a JSONL [`EngineEvent`].
///
/// Returns `None` if the line is empty, doesn't begin with `{` (a fast
/// reject for tracing-style lines like `INFO foo`), or fails to
/// deserialize. The runner uses this to route subprocess output: a
/// successful parse becomes a structured event; failures fall through
/// to `Raw` for verbatim forwarding (so a parser panic that prints a
/// stack trace still surfaces in the CLI/desktop UI, just unstructured).
///
/// Why a fast `starts_with('{')` check before parsing: parsing JSON
/// rejects most non-JSON lines quickly, but at parse-DB scale the parser
/// emits one-line-per-file and a 100k-file monorepo yields 100k+
/// candidate lines. A no-allocation prefix check filters the obvious
/// negatives before serde_json walks them.
pub fn try_parse_line(line: &str) -> Option<EngineEvent> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_roundtrips_through_jsonl() {
        let event = EngineEvent::progress(42, "pipeline", "progress", "Parsing");
        let jsonl = event.to_jsonl().unwrap();
        assert!(jsonl.ends_with('\n'));
        let parsed = try_parse_line(jsonl.trim()).unwrap();
        assert_eq!(parsed, event);
    }

    #[test]
    fn progress_caps_percent_at_100() {
        let event = EngineEvent::progress(150, "pipeline", "done", "x");
        match event {
            EngineEvent::Progress { percent, .. } => assert_eq!(percent, 100),
            _ => panic!("expected Progress variant"),
        }
    }

    #[test]
    fn progress_with_language_serializes_field() {
        let event = EngineEvent::progress_lang(10, "parse", "start", "Starting", "typescript");
        let jsonl = event.to_jsonl().unwrap();
        assert!(jsonl.contains(r#""language":"typescript""#));
    }

    #[test]
    fn progress_without_language_omits_field() {
        let event = EngineEvent::progress(10, "analyzing", "start", "Cycles");
        let jsonl = event.to_jsonl().unwrap();
        // `skip_serializing_if = "Option::is_none"` must drop the field.
        assert!(!jsonl.contains(r#""language""#));
    }

    #[test]
    fn parser_legacy_progress_shape_parses() {
        // This is the exact byte sequence the parser emitted prior to
        // R3 via its (now-deleted) `StdoutProgressEmitter`. The shared
        // EngineEvent must deserialize it unchanged so any consumer
        // that buffered legacy JSONL on disk (e.g. SHADOW_MODE pilot
        // reports under docs/04-evidence/) continues to parse.
        let line = r#"{"type":"progress","percent":75,"phase":"resolution","status":"done","message":"Done"}"#;
        let parsed = try_parse_line(line).unwrap();
        match parsed {
            EngineEvent::Progress {
                percent,
                phase,
                status,
                message,
                language,
            } => {
                assert_eq!(percent, 75);
                assert_eq!(phase, "resolution");
                assert_eq!(status, "done");
                assert_eq!(message, "Done");
                assert!(language.is_none());
            }
            _ => panic!("expected Progress variant"),
        }
    }

    #[test]
    fn parser_legacy_file_manifest_parses() {
        let line =
            r#"{"type":"file_manifest","total_files":2,"files":["src/main.ts","src/app.ts"]}"#;
        let parsed = try_parse_line(line).unwrap();
        match parsed {
            EngineEvent::FileManifest {
                total_files,
                files,
                language,
            } => {
                assert_eq!(total_files, 2);
                assert_eq!(files, vec!["src/main.ts", "src/app.ts"]);
                assert!(language.is_none());
            }
            _ => panic!("expected FileManifest variant"),
        }
    }

    #[test]
    fn parser_legacy_file_progress_parses() {
        let line =
            r#"{"type":"file_progress","file_path":"src/main.ts","file_index":0,"status":"start"}"#;
        let parsed = try_parse_line(line).unwrap();
        match parsed {
            EngineEvent::FileProgress {
                file_path,
                file_index,
                status,
                language,
            } => {
                assert_eq!(file_path, "src/main.ts");
                assert_eq!(file_index, 0);
                assert_eq!(status, "start");
                assert!(language.is_none());
            }
            _ => panic!("expected FileProgress variant"),
        }
    }

    #[test]
    fn rejects_non_json_tracing_lines() {
        // Parser/analyzer stderr is full of these from tracing.
        assert!(try_parse_line("INFO some::module: doing thing").is_none());
        assert!(try_parse_line("[ge-analyze] Reading graph from database...").is_none());
        assert!(try_parse_line("").is_none());
        assert!(try_parse_line("   ").is_none());
    }

    #[test]
    fn rejects_malformed_json() {
        // Looks JSON-ish (starts with `{`) but is broken — must return
        // None, not panic. The runner falls back to Raw forwarding for
        // these lines.
        assert!(try_parse_line(r#"{"type":"progress","percent":"#).is_none());
        assert!(try_parse_line(r#"{"type":"unknown_kind"}"#).is_none());
        assert!(try_parse_line(r#"{not even close to json}"#).is_none());
    }

    #[test]
    fn rejects_json_object_missing_type_tag() {
        // Tagged enum requires `type`; without it, deserialization fails.
        // We never want to silently mis-classify an arbitrary JSON object
        // as a progress event.
        assert!(try_parse_line(r#"{"percent":50}"#).is_none());
    }

    #[test]
    fn error_event_roundtrips() {
        let event = EngineEvent::Error {
            phase: "parse".into(),
            message: "missing grammar".into(),
            code: Some("parser.missing_grammar".into()),
            language: Some("rust".into()),
        };
        let jsonl = event.to_jsonl().unwrap();
        let parsed = try_parse_line(jsonl.trim()).unwrap();
        assert_eq!(parsed, event);
    }

    #[test]
    fn emit_writes_single_line_to_writer() {
        let event = EngineEvent::progress(50, "parsing", "progress", "halfway");
        let mut buf: Vec<u8> = Vec::new();
        event.emit(&mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.matches('\n').count(), 1);
        assert!(s.ends_with('\n'));
    }

    #[test]
    fn jsonl_payload_never_contains_embedded_newline() {
        // Messages with newlines must be escaped; otherwise the consumer's
        // line-oriented reader would split the event across multiple
        // lines and lose the discriminator.
        let event = EngineEvent::progress(0, "parse", "error", "panic\noccurred");
        let jsonl = event.to_jsonl().unwrap();
        // Exactly one newline — the framing one at the end.
        assert_eq!(jsonl.matches('\n').count(), 1);
        // And the escape sequence is present.
        assert!(jsonl.contains(r"\n"));
    }
}
