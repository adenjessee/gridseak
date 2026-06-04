//! Per-call-site trace sink for `textDocument/definition` requests.
//!
//! Enabled by setting `GRAPHENGINE_LSP_TRACE_DEFS=/absolute/path/to/trace.jsonl`
//! at process start. Each successful or failed definition request appends
//! one JSON line with:
//!
//! * `ts_ms`      — wall clock since epoch, milliseconds.
//! * `file`       — absolute source path the request was made *against*.
//! * `line`       — zero-based LSP line (after the `-1` conversion).
//! * `character`  — zero-based LSP UTF-16 column that was sent.
//! * `byte_col`   — raw tree-sitter start column prior to UTF-16 conversion.
//! * `symbol`     — the bare symbol name passed to `find_definition`.
//! * `elapsed_ms` — milliseconds the request spent in flight (per attempt).
//! * `outcome`    — one of `"null"`, `"hit"`, `"error"`.
//! * `hit_uri`    — present on `"hit"`; the URI of the resolved definition.
//! * `hit_line`   — present on `"hit"`; zero-based start line.
//! * `hit_col`    — present on `"hit"`; zero-based UTF-16 start column.
//! * `err`        — present on `"error"`; the [`LspError`] Display string.
//!
//! The sink is a global `OnceLock<Option<...>>` so the per-call cost is one
//! env read at process start plus a fast-path `None` check on every
//! subsequent call. When the env var is absent the overhead is a single
//! pointer comparison.
//!
//! This module exists to satisfy the Tier 3 jorje-P0 debug step from the
//! 48 h demo-readiness plan. It is instrumentation code — deliberately
//! narrow-scope, no public state beyond a free function, and it does NOT
//! influence resolver semantics: record-only.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::infrastructure::lsp::errors::LspError;

/// The outcome of a single `textDocument/definition` attempt from the
/// engine's perspective. One variant per possible classification in the
/// Tier-3 debug plan's decision rules (null / timing vs null / position
/// vs server error).
pub enum DefOutcome<'a> {
    /// Server answered `null` — no definition found.
    Null,
    /// Server returned a location. `uri` / `line` / `col` are the
    /// zero-based coordinates the LSP response carried.
    Hit { uri: &'a str, line: u32, col: u32 },
    /// Transport / protocol-level failure surfaced as [`LspError`].
    Err(&'a LspError),
}

struct TraceSink {
    writer: Mutex<BufWriter<File>>,
}

static SINK: OnceLock<Option<TraceSink>> = OnceLock::new();

fn sink() -> Option<&'static TraceSink> {
    SINK.get_or_init(|| {
        let raw = std::env::var("GRAPHENGINE_LSP_TRACE_DEFS").ok()?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let path = PathBuf::from(trimmed);
        if let Some(parent) = path.parent() {
            // Silently try to create the directory. If this fails the
            // OpenOptions step below will surface the error, but we
            // don't want to panic from instrumentation code.
            let _ = std::fs::create_dir_all(parent);
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        Some(TraceSink {
            writer: Mutex::new(BufWriter::new(file)),
        })
    })
    .as_ref()
}

/// Append one record to the trace file if tracing is enabled. No-op
/// otherwise. Failures (poisoned mutex, write errors) are swallowed
/// so the instrumentation cannot change resolver behaviour.
pub fn record(
    file: &str,
    line: u32,
    character: u32,
    byte_col: u32,
    symbol: &str,
    elapsed: Duration,
    outcome: DefOutcome<'_>,
) {
    let Some(sink) = sink() else { return };

    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut obj = json!({
        "ts_ms": ts_ms,
        "file": file,
        "line": line,
        "character": character,
        "byte_col": byte_col,
        "symbol": symbol,
        "elapsed_ms": elapsed.as_millis() as u64,
    });

    match outcome {
        DefOutcome::Null => {
            obj["outcome"] = json!("null");
        }
        DefOutcome::Hit { uri, line, col } => {
            obj["outcome"] = json!("hit");
            obj["hit_uri"] = json!(uri);
            obj["hit_line"] = json!(line);
            obj["hit_col"] = json!(col);
        }
        DefOutcome::Err(err) => {
            obj["outcome"] = json!("error");
            obj["err"] = json!(err.to_string());
        }
    }

    let line = match serde_json::to_string(&obj) {
        Ok(s) => s,
        Err(_) => return,
    };

    let Ok(mut guard) = sink.writer.lock() else {
        return;
    };
    let _ = guard.write_all(line.as_bytes());
    let _ = guard.write_all(b"\n");
    let _ = guard.flush();
}

/// Returns `true` when a trace sink is configured. Callers can use this
/// to skip expensive pre-computation of trace fields when tracing is off.
pub fn enabled() -> bool {
    sink().is_some()
}
