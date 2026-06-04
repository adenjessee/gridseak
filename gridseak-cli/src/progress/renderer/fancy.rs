//! In-place TTY renderer.
//!
//! Stage 1 lands a working implementation that uses simple ANSI cursor
//! controls (carriage return + line erase) to repaint a single status
//! line in place. Stage 2 will replace this with a multi-line block
//! (stage list + current file + counter) using the spec's recommended
//! layout from `CLI_SHADOW_MODE_DISTRIBUTION_SPEC.md` lines 384-398.
//! The trait surface stays stable across the upgrade.
//!
//! What "in-place" means here:
//!
//! - Writes use `\r` (carriage return) + erase-to-end-of-line
//!   (`\x1b[K`), then the new content. No newline is emitted until
//!   [`ProgressRenderer::on_finish`] is called.
//! - We only redraw on a frame budget (default 100ms) so a high-volume
//!   `FileProgress` stream doesn't saturate the terminal.
//! - Errors and stage transitions break out of the in-place line and
//!   start a fresh line with the actual message, so error context
//!   isn't overwritten by the next progress frame.

use std::io::Write;
use std::time::{Duration, Instant};

use gridseak_engine_runner::{EngineEvent, ProgressEvent, Stage};

use crate::progress::aggregator::StageView;

use super::ProgressRenderer;

/// Minimum gap between in-place redraws. 100 ms feels live without
/// burning the terminal on a fast monorepo scan. Stage transitions and
/// errors bypass this throttle so they're always visible immediately.
const FRAME_INTERVAL: Duration = Duration::from_millis(100);

pub struct FancyRenderer {
    last_paint: Option<Instant>,
    last_stage: Option<Stage>,
    last_language: Option<String>,
    /// Whether we have an in-place line currently displayed. When
    /// `true`, we own the bottom row of the terminal and must clear it
    /// before writing any append-only output (errors, raw warnings).
    has_inplace_line: bool,
}

impl FancyRenderer {
    pub fn new() -> Self {
        Self {
            last_paint: None,
            last_stage: None,
            last_language: None,
            has_inplace_line: false,
        }
    }

    fn paint_status(&mut self, view: &StageView, force: bool) {
        if !force {
            if let Some(last) = self.last_paint {
                if last.elapsed() < FRAME_INTERVAL {
                    return;
                }
            }
        }
        let line = format_status_line(view);
        let mut stderr = std::io::stderr().lock();
        // \r returns cursor to column 1; \x1b[K erases to end-of-line.
        // Then write the new line; do NOT emit a newline so the next
        // paint overwrites this one.
        let _ = write!(stderr, "\r\x1b[K{line}");
        let _ = stderr.flush();
        self.last_paint = Some(Instant::now());
        self.has_inplace_line = true;
    }

    fn break_inplace_line(&mut self) {
        if self.has_inplace_line {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr);
            self.has_inplace_line = false;
        }
    }

    fn append_line(&mut self, line: &str) {
        self.break_inplace_line();
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "{line}");
    }
}

fn format_status_line(view: &StageView) -> String {
    let pct = view.overall_percent;
    let stage = view
        .stage
        .map(|s| s.to_string())
        .unwrap_or_else(|| "starting".into());
    let lang = view
        .language
        .as_deref()
        .map(|l| format!(" {l}"))
        .unwrap_or_default();
    let counter = match (view.files_total, view.files_done) {
        (Some(total), done) if total > 0 => format!(" [{done}/{total}]"),
        _ => String::new(),
    };
    let elapsed = {
        let secs = view.elapsed.as_secs();
        if secs < 60 {
            format!("{secs}s")
        } else {
            format!("{}m{}s", secs / 60, secs % 60)
        }
    };
    let detail = if view.detail.is_empty() {
        "".into()
    } else {
        // Truncate detail to keep the line bounded. Real width-aware
        // logic lands in Stage 2; here we just cap at a reasonable
        // length.
        let max_detail = 60;
        let detail = if view.detail.len() > max_detail {
            format!("…{}", &view.detail[view.detail.len() - max_detail + 1..])
        } else {
            view.detail.clone()
        };
        format!(" {detail}")
    };
    format!("⏳ {pct:>3}% {stage}{lang}{counter}{detail} · {elapsed}")
}

impl ProgressRenderer for FancyRenderer {
    fn on_start(&mut self) {
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "GridSeak scanning…");
    }

    fn on_event(&mut self, view: &StageView, event: &ProgressEvent) {
        // Stage / language transitions always force an immediate
        // repaint so the user sees the boundary as it happens.
        let force_paint = match event {
            ProgressEvent::StageStarted { stage, language } => {
                let changed = Some(*stage) != self.last_stage || language != &self.last_language;
                if changed {
                    self.last_stage = Some(*stage);
                    self.last_language = language.clone();
                }
                changed
            }
            ProgressEvent::StageFinished { .. } => true,
            ProgressEvent::Engine { event, .. } => {
                // Errors break the in-place line and print verbatim.
                if let EngineEvent::Error {
                    phase,
                    message,
                    code,
                    ..
                } = event
                {
                    let code_str = code
                        .as_deref()
                        .map(|c| format!(" [{c}]"))
                        .unwrap_or_default();
                    self.append_line(&format!("ERROR during {phase}{code_str}: {message}"));
                    return;
                }
                false
            }
            ProgressEvent::Raw { line, .. } => {
                // Pass-through for noisy warnings/errors. Skip routine
                // tracing noise.
                let upper = line.to_ascii_uppercase();
                if upper.contains("ERROR")
                    || upper.contains("WARN")
                    || upper.contains("PANIC")
                    || upper.contains("FAILED")
                {
                    self.append_line(line);
                }
                return;
            }
            // Cross-crate `#[non_exhaustive]` requires a wildcard. Log
            // so an unhandled new variant is visible during dev/CI;
            // don't repaint because we don't know what changed.
            unknown => {
                self.append_line(&format!(
                    "[gridseak] progress: unhandled ProgressEvent variant: {unknown:?}"
                ));
                return;
            }
        };

        self.paint_status(view, force_paint);
    }

    fn on_finish(&mut self) {
        // Clear the in-place block so the final report starts on a
        // fresh line. Stage 2 will replace this with a "took Xs"
        // closer; for now a single newline is enough.
        self.break_inplace_line();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn format_status_line_handles_no_stage() {
        let view = StageView {
            elapsed: Duration::from_secs(3),
            ..Default::default()
        };
        let line = format_status_line(&view);
        assert!(line.contains("starting"));
        assert!(line.contains("3s"));
    }

    #[test]
    fn format_status_line_shows_counter_when_known() {
        let view = StageView {
            stage: Some(Stage::Parsing),
            language: Some("ts".into()),
            files_total: Some(100),
            files_done: 42,
            overall_percent: 30,
            ..Default::default()
        };
        let line = format_status_line(&view);
        assert!(line.contains("[42/100]"));
        assert!(line.contains("ts"));
        assert!(line.contains("30%"));
    }

    #[test]
    fn format_status_line_truncates_long_detail() {
        let view = StageView {
            stage: Some(Stage::Parsing),
            detail: "x".repeat(200),
            ..Default::default()
        };
        let line = format_status_line(&view);
        // Should contain the ellipsis indicator and be bounded in
        // length.
        assert!(line.contains('…'));
    }
}
