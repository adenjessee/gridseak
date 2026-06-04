//! Line-oriented append-only renderer.
//!
//! Used in CI (`CI=true`), when stderr is not a TTY (pipes, log
//! collectors), and as the explicit `--progress plain` mode.
//!
//! Design constraints:
//!
//! - **Append-only**: must never use ANSI cursor movement. CI log
//!   collectors and `tee` recorders treat `\x1b[` sequences as garbage.
//! - **Deduplicated**: the aggregator updates the view on every event,
//!   but a 600-file parser pass would produce hundreds of nearly-
//!   identical lines if we printed verbatim. We coalesce by "only
//!   print when the visible information actually changes."
//! - **Stderr only**: stdout is reserved for the final report block
//!   (table/markdown/JSON depending on `--format`). The renderer never
//!   touches stdout.
//!
//! What "the visible information actually changes" means here:
//!
//! - top-level stage transitions (parsing→analyzing) → always print
//! - language transitions within parsing → always print
//! - file count progresses past a threshold → print every 25 files OR
//!   when crossing a 10% point of the current pass
//! - analyzer phase messages → always print (they're already coarse)
//! - errors → always print

use std::io::Write;
use std::time::Duration;

use gridseak_engine_runner::{EngineEvent, ProgressEvent, Stage};

use crate::progress::aggregator::StageView;

use super::ProgressRenderer;

pub struct PlainRenderer {
    /// Last `(stage, language, files_done_threshold, message)` tuple
    /// we printed. A new event prints only if its tuple differs.
    last_signature: Option<Signature>,
    /// Print rate-limit for `Raw` lines. They can be very chatty. We
    /// always pass tracing-style errors and warnings through (heuristic
    /// match on `ERROR` / `WARN` / `panicked`), but for `INFO`/`DEBUG`
    /// chatter we drop in plain mode. `--verbose` would surface them;
    /// for now we don't have that flag yet (Stage 2 adds it).
    raw_filter: RawFilter,
    /// S1 cache-stats: printed once per scan, after the planner emits
    /// the CacheStats event. We latch this so a later view-change
    /// (e.g. the next status line) doesn't re-emit the line.
    cache_stats_emitted: bool,
    analysis_mode_emitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Signature {
    stage: Option<Stage>,
    language: Option<String>,
    files_bucket: Option<usize>,
    message: String,
}

struct RawFilter;

impl RawFilter {
    fn should_show(&self, line: &str) -> bool {
        let upper = line.to_ascii_uppercase();
        // Surface the worrying stuff; drop chatty INFO/DEBUG.
        upper.contains("ERROR")
            || upper.contains("WARN")
            || upper.contains("PANIC")
            || upper.contains("FAILED")
    }
}

impl PlainRenderer {
    pub fn new() -> Self {
        Self {
            last_signature: None,
            raw_filter: RawFilter,
            cache_stats_emitted: false,
            analysis_mode_emitted: false,
        }
    }

    fn maybe_print_analysis_mode(&mut self, view: &StageView) {
        if self.analysis_mode_emitted {
            return;
        }
        let Some(msg) = view.analysis_mode.as_ref() else {
            return;
        };
        self.analysis_mode_emitted = true;
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "[gridseak] analysis: {msg}");
    }

    /// Print the S1 cache-stats line once, when the aggregator has
    /// observed the planner's `CacheStats` event. Idempotent — the
    /// `cache_stats_emitted` latch ensures repeated calls (one per
    /// downstream view update) do not re-emit.
    fn maybe_print_cache_stats(&mut self, view: &StageView) {
        if self.cache_stats_emitted {
            return;
        }
        let Some(stats) = view.cache_stats else {
            return;
        };
        self.cache_stats_emitted = true;
        let mut stderr = std::io::stderr().lock();
        if stats.disabled {
            let _ = writeln!(
                stderr,
                "[gridseak] incremental: disabled ({} files, full reparse)",
                stats.total_files
            );
        } else {
            let removed = if stats.removed > 0 {
                format!(", {} removed", stats.removed)
            } else {
                String::new()
            };
            let _ = writeln!(
                stderr,
                "[gridseak] incremental: {} files ({} cached, {} reparsed{removed})",
                stats.total_files, stats.cached, stats.reparsed
            );
        }
    }

    fn signature(&self, view: &StageView) -> Signature {
        Signature {
            stage: view.stage,
            language: view.language.clone(),
            // Bucket file counts so we don't print on every file done.
            // Threshold: print whenever we cross a 5% point of the
            // active pass, OR whenever `files_done % 25 == 0`.
            files_bucket: view.files_total.map(|total| {
                let by_percent = if total > 0 {
                    (view.files_done * 20) / total // 5% buckets
                } else {
                    0
                };
                let by_count = view.files_done / 25;
                by_percent + by_count
            }),
            message: view.message.clone(),
        }
    }

    fn write_status(&self, view: &StageView) {
        let stage = match view.stage {
            Some(s) => s.to_string(),
            None => "starting".into(),
        };
        let lang = match &view.language {
            Some(l) => format!(" {l}"),
            None => String::new(),
        };
        let counter = match (view.files_total, view.files_done) {
            (Some(total), done) if total > 0 => format!(" [{done}/{total}]"),
            _ => String::new(),
        };
        let elapsed = fmt_elapsed(view.elapsed);
        let pct = view.overall_percent;
        let message = if view.message.is_empty() {
            "".into()
        } else {
            format!(" — {}", view.message)
        };
        let mut stderr = std::io::stderr().lock();
        // No newline-on-error handling: stderr writes that fail are not
        // recoverable mid-scan and the kernel will still flush whatever
        // it can. Best-effort.
        let _ = writeln!(
            stderr,
            "[gridseak] {pct:>3}% {stage}{lang}{counter} ({elapsed}){message}"
        );
    }
}

fn fmt_elapsed(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

impl ProgressRenderer for PlainRenderer {
    fn on_start(&mut self) {
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "[gridseak] starting scan");
    }

    fn on_event(&mut self, view: &StageView, event: &ProgressEvent) {
        // Errors and raw warnings: always pass through (after filtering).
        if let ProgressEvent::Raw { line, .. } = event {
            if self.raw_filter.should_show(line) {
                let mut stderr = std::io::stderr().lock();
                let _ = writeln!(stderr, "[gridseak] {line}");
            }
            return;
        }

        if let ProgressEvent::Engine {
            event:
                EngineEvent::Error {
                    phase,
                    message,
                    code,
                    ..
                },
            ..
        } = event
        {
            let mut stderr = std::io::stderr().lock();
            let code_str = code
                .as_deref()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            let _ = writeln!(
                stderr,
                "[gridseak] error during {phase}{code_str}: {message}"
            );
            return;
        }

        self.maybe_print_cache_stats(view);
        self.maybe_print_analysis_mode(view);

        let sig = self.signature(view);
        if self.last_signature.as_ref() == Some(&sig) {
            return;
        }
        self.last_signature = Some(sig);
        self.write_status(view);
    }

    fn on_finish(&mut self) {
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "[gridseak] scan complete");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_filter_passes_errors_and_warnings() {
        let f = RawFilter;
        assert!(f.should_show("INFO foo ERROR bar"));
        assert!(f.should_show("WARN: something"));
        assert!(f.should_show("thread 'main' panicked at"));
        assert!(f.should_show("Parsing FAILED for src/x.ts"));
    }

    #[test]
    fn raw_filter_drops_chatter() {
        let f = RawFilter;
        assert!(!f.should_show("INFO loading config"));
        assert!(!f.should_show("DEBUG node count 1234"));
        assert!(!f.should_show("trace: walking ast"));
    }

    #[test]
    fn fmt_elapsed_under_minute() {
        assert_eq!(fmt_elapsed(Duration::from_secs(0)), "0s");
        assert_eq!(fmt_elapsed(Duration::from_secs(42)), "42s");
    }

    #[test]
    fn fmt_elapsed_minutes() {
        assert_eq!(fmt_elapsed(Duration::from_secs(60)), "1m0s");
        assert_eq!(fmt_elapsed(Duration::from_secs(125)), "2m5s");
    }
}
