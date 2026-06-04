//! Stateful aggregator that turns the runner's event stream into a
//! single "what is happening right now" view.
//!
//! Why this is a separate type from the renderer: rendering is the
//! cheap part. State maintenance (which stage are we in, how many files
//! parsed, what was the last percent the analyzer reported, how long
//! has the current stage been running) is the part that's easy to get
//! subtly wrong. Keeping it in one place means tests for state
//! transitions live in one file and the renderers consume a flat
//! snapshot they cannot mutate.

use std::time::{Duration, Instant};

use gridseak_engine_runner::{EngineEvent, ProgressEvent, Stage};

/// One-shot summary of the S1 incremental-scan cache lookup. Mirrors
/// `EngineEvent::CacheStats` but lives on `StageView` so renderers
/// don't need to depend directly on `graphengine-progress`. Present
/// on `StageView` once the parser emits its cache-stats event right
/// after the planner runs (between discovery and extraction); absent
/// for scans where the parser didn't emit one (cold cache before this
/// release, or a non-parsing flow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStatsView {
    pub total_files: usize,
    pub cached: usize,
    pub reparsed: usize,
    pub removed: usize,
    pub disabled: bool,
}

/// Snapshot of what the scan is doing right now. Renderers receive
/// this on every event so they can decide whether to repaint without
/// reaching back into the aggregator.
#[derive(Debug, Clone)]
pub struct StageView {
    /// Which top-level pipeline stage is active. `None` before the
    /// first event arrives.
    pub stage: Option<Stage>,

    /// Active parser language sub-stage, if any. None during the
    /// `Preparing` and `Analyzing` stages (analyzer doesn't tag events
    /// with a language; analysis runs over the merged graph).
    pub language: Option<String>,

    /// Estimated 0–100 progress for the *whole pipeline*. Computed by
    /// blending the runner's stage boundaries with the engine's
    /// in-stage `EngineEvent::Progress` percent. See [`stage_window`]
    /// for the slice of `[0, 100]` each stage owns.
    pub overall_percent: u8,

    /// The most recent human-readable status line emitted by the
    /// engine (`Progress.message`). Empty before the first engine
    /// event arrives. Renderers may truncate for narrow terminals.
    pub message: String,

    /// Most recent per-language detail (typically the current file
    /// being parsed). Empty when none has been emitted yet.
    pub detail: String,

    /// Total files in the active parser pass, if known.
    /// `FileManifest.total_files` populates this.
    pub files_total: Option<usize>,

    /// Files completed in the active parser pass.
    /// Incremented on each `FileProgress { status: "done" }`.
    pub files_done: usize,

    /// Elapsed wall-clock since the pipeline started. Used by renderers
    /// to show "scanning… 12s" without each having to track time.
    pub elapsed: Duration,

    /// S1 incremental-scan cache outcome for this scan. Set by the
    /// aggregator when `EngineEvent::CacheStats` arrives (once per
    /// scan, between discovery and extraction). Renderers display
    /// this as a single line "incremental: 612 files (594 cached, 18
    /// reparsed)" so the user can see how much extraction the cache
    /// saved.
    pub cache_stats: Option<CacheStatsView>,

    /// S2-β analysis mode summary (`analysis: segmented (N files)`).
    pub analysis_mode: Option<String>,
}

impl Default for StageView {
    fn default() -> Self {
        Self {
            stage: None,
            language: None,
            overall_percent: 0,
            message: String::new(),
            detail: String::new(),
            files_total: None,
            files_done: 0,
            elapsed: Duration::ZERO,
            cache_stats: None,
            analysis_mode: None,
        }
    }
}

/// Owns the running interpretation of the event stream.
///
/// Holds an `Instant` for `started` because the aggregator may outlive
/// any one renderer (e.g. when a future feature switches renderers
/// mid-scan). Renderers ask for the elapsed time via [`StageView::elapsed`]
/// rather than reading the clock themselves.
pub struct ProgressAggregator {
    view: StageView,
    started: Instant,
    /// Pre-stage marker: the runner emits `StageStarted { Parsing,
    /// None }` once before per-language sub-stages. We seed
    /// `language_stage_count` with the count of languages we've
    /// observed so the percentage can interpolate between them as
    /// each language finishes.
    languages_finished: usize,
    /// How many parser-per-language sub-stages we've observed starting.
    /// We can't know the *total* up front from the event stream alone,
    /// so the renderer asymptotically approaches but never quite hits
    /// the parser-stage's upper bound until `StageStarted { Analyzing }`
    /// fires. This is fine: the spec says progress should be truthful,
    /// not precise.
    languages_started: usize,
}

/// `[base, ceil]` pct window each top-level stage occupies in the
/// overall 0–100 bar. Chosen to approximate the typical scan profile
/// (parsing dominates; analysis is fast; preparation is near-instant).
/// Renderers don't depend on the exact numbers — these only need to be
/// monotone and sum to roughly `[0, 100]`. See
/// `desktop/src-tauri/src/engine.rs` for a similar mapping used by the
/// desktop sink.
fn stage_window(stage: Stage) -> (u8, u8) {
    match stage {
        Stage::Preparing => (0, 5),
        Stage::Parsing => (5, 65),
        Stage::Analyzing => (65, 100),
    }
}

impl ProgressAggregator {
    pub fn new() -> Self {
        Self {
            view: StageView::default(),
            started: Instant::now(),
            languages_finished: 0,
            languages_started: 0,
        }
    }

    /// Apply one event and return the updated view.
    ///
    /// Returns `&StageView` rather than a fresh `StageView` because the
    /// renderer reads the same field set repeatedly and we want to
    /// avoid an allocation per event in the hot path. Renderers that
    /// need to retain a snapshot across calls can `.clone()` the view.
    ///
    /// # `#[non_exhaustive]` and the fallback arm
    ///
    /// `ProgressEvent` is marked `#[non_exhaustive]` in
    /// `gridseak-engine-runner`. Rust requires a wildcard arm when
    /// matching such enums from a downstream crate, even when we cover
    /// every current variant. The fallback below is therefore *not*
    /// silent: it logs the unhandled variant to stderr so a new variant
    /// added upstream is visible in CI logs until an explicit arm is
    /// added here. (`non_exhaustive_omitted_patterns_lint` would give
    /// us a compile-time signal, but it is still nightly-only as of
    /// Rust 1.91, so the log is the next-best maintenance signal.)
    pub fn apply(&mut self, event: &ProgressEvent) -> &StageView {
        self.view.elapsed = self.started.elapsed();

        match event {
            ProgressEvent::StageStarted { stage, language } => {
                self.view.stage = Some(*stage);
                self.view.language = language.clone();
                if let (Stage::Parsing, Some(_)) = (*stage, language) {
                    self.languages_started = self.languages_started.saturating_add(1);
                    // Reset per-language counters when a new language begins.
                    self.view.files_total = None;
                    self.view.files_done = 0;
                }
                self.recompute_overall();
            }

            ProgressEvent::StageFinished {
                stage, language, ..
            } => {
                if let (Stage::Parsing, Some(_)) = (*stage, language) {
                    self.languages_finished = self.languages_finished.saturating_add(1);
                }
                self.recompute_overall();
            }

            ProgressEvent::Engine {
                stage,
                language,
                event,
            } => {
                self.view.stage = Some(*stage);
                if let Some(lang) = language {
                    self.view.language = Some(lang.clone());
                }
                self.apply_engine_event(event);
            }

            ProgressEvent::Raw { .. } => {
                // Raw lines don't change the view directly. Renderers
                // that show verbose output read raw lines off the
                // event stream themselves (the sink forwards `Raw`
                // events to the renderer as-is).
            }
            // Cross-crate `#[non_exhaustive]` requires a wildcard arm.
            // We log so the unknown variant is visible during dev /
            // CI rather than silently dropped. See module docs above.
            unknown => {
                eprintln!(
                    "[gridseak] progress: unhandled ProgressEvent variant; \
                     update gridseak-cli/src/progress/aggregator.rs to render it. \
                     event = {unknown:?}"
                );
            }
        }

        &self.view
    }

    fn apply_engine_event(&mut self, event: &EngineEvent) {
        match event {
            EngineEvent::Progress {
                percent,
                phase: _,
                status: _,
                message,
                language,
            } => {
                self.view.message = message.clone();
                if let Some(lang) = language {
                    self.view.language = Some(lang.clone());
                }
                self.set_overall_from_engine_percent(*percent);
            }
            EngineEvent::FileManifest {
                total_files,
                files: _,
                ..
            } => {
                self.view.files_total = Some(*total_files);
                self.view.files_done = 0;
            }
            EngineEvent::FileProgress {
                file_path,
                file_index: _,
                status,
                ..
            } => {
                self.view.detail = file_path.clone();
                if status == "done" || status == "error" {
                    self.view.files_done = self.view.files_done.saturating_add(1);
                }
                // Within a parser pass, interpolate using files_done /
                // files_total inside the current language's slice of
                // the parsing window. We compute on the fly rather
                // than storing per-language windows because
                // `languages_started` is the authoritative count.
                self.recompute_overall();
            }
            EngineEvent::Error { message, .. } => {
                self.view.message = message.clone();
            }
            EngineEvent::CacheStats {
                total_files,
                cached,
                reparsed,
                removed,
                disabled,
                ..
            } => {
                // S1 (incremental scanning). One-shot summary; the
                // parser emits exactly one of these per scan, right
                // after the planner classifies discovered files
                // against the previous parse DB's `file_cache` rows.
                // Latched on `view.cache_stats` so any renderer can
                // display it without re-receiving the event.
                self.view.cache_stats = Some(CacheStatsView {
                    total_files: *total_files,
                    cached: *cached,
                    reparsed: *reparsed,
                    removed: *removed,
                    disabled: *disabled,
                });
            }
            EngineEvent::AnalysisMode { message, .. } => {
                self.view.analysis_mode = Some(message.clone());
            }
            // `EngineEvent` is `#[non_exhaustive]`. Rust requires a
            // wildcard arm in downstream crates regardless of whether
            // we exhaustively cover all current variants. We log
            // unknown variants so adding one upstream is visible in
            // dev/CI rather than silently no-op'd.
            unknown => {
                eprintln!(
                    "[gridseak] progress: unhandled EngineEvent variant; \
                     update gridseak-cli/src/progress/aggregator.rs to render it. \
                     event = {unknown:?}"
                );
            }
        }
    }

    /// Recompute `overall_percent` from the stage + sub-stage state.
    /// Called whenever a structural event (stage boundary, file done)
    /// shifts the position; engine-emitted percents go through
    /// [`set_overall_from_engine_percent`] for direct mapping.
    ///
    /// # Ratchet
    ///
    /// The result is taken max() with `self.view.overall_percent`.
    /// Progress should never go backwards: when a second language pass
    /// starts, the denominator grows (we now know there are 2
    /// languages, not 1), which would naively re-divide the work done
    /// in pass 1 into a smaller fraction. The shadow-mode spec is
    /// explicit that progress must be truthful but never invent
    /// precision — and going *backwards* is a UX lie that suggests
    /// "something broke." So we cap the new value at no less than the
    /// previous high-water mark and accept that the bar may stall
    /// briefly while the underlying real % catches up.
    fn recompute_overall(&mut self) {
        let stage = match self.view.stage {
            Some(s) => s,
            None => {
                self.view.overall_percent = 0;
                return;
            }
        };
        let (base, ceil) = stage_window(stage);
        let pct = match stage {
            Stage::Preparing => base,
            Stage::Analyzing => base,
            Stage::Parsing => {
                let lang_count = self.languages_started.max(1);
                let parsing_window = (ceil - base) as f32;
                let per_lang = parsing_window / lang_count as f32;
                let completed_langs = self.languages_finished.min(lang_count) as f32;
                let in_progress = match (self.view.files_total, self.view.files_done) {
                    (Some(total), done) if total > 0 => (done as f32 / total as f32).min(1.0),
                    _ => 0.0,
                };
                // `completed_langs * per_lang` is the work fully done;
                // `in_progress * per_lang` is the in-flight language's
                // contribution. Capped at `parsing_window` so a
                // pre-StageFinished `files_done == files_total` can
                // brush right up against `ceil` without exceeding it.
                let earned =
                    (completed_langs * per_lang + in_progress * per_lang).min(parsing_window);
                (base as f32 + earned).round() as u8
            }
        };
        let capped = pct.min(100);
        // Ratchet: never go backwards. The previous value is always a
        // truthful lower bound — the engine has actually done that
        // much work — so dropping below it would be the lie.
        self.view.overall_percent = capped.max(self.view.overall_percent);
    }

    /// Map an engine-reported percent (0–100 within its phase) onto
    /// the overall bar's slice for the active stage. Falls back to
    /// the structural recompute when no stage is active.
    ///
    /// Also applies the ratchet: an analyzer's `Progress { percent: 30 }`
    /// after the bar has reached 65% (from parsing) doesn't yank it
    /// back to ~70%; it stays where it was until the engine catches up.
    fn set_overall_from_engine_percent(&mut self, engine_pct: u8) {
        let stage = match self.view.stage {
            Some(s) => s,
            None => {
                self.view.overall_percent = engine_pct.min(100).max(self.view.overall_percent);
                return;
            }
        };
        let (base, ceil) = stage_window(stage);
        let scaled = base as f32 + (engine_pct.min(100) as f32 / 100.0) * (ceil - base) as f32;
        let new_pct = scaled.round() as u8;
        self.view.overall_percent = new_pct.max(self.view.overall_percent);
    }

    pub fn view(&self) -> &StageView {
        &self.view
    }
}

impl Default for ProgressAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridseak_engine_runner::Stage;

    fn stage_started(stage: Stage, lang: Option<&str>) -> ProgressEvent {
        ProgressEvent::StageStarted {
            stage,
            language: lang.map(|s| s.to_string()),
        }
    }

    fn stage_finished(stage: Stage, lang: Option<&str>) -> ProgressEvent {
        ProgressEvent::StageFinished {
            stage,
            language: lang.map(|s| s.to_string()),
            elapsed_ms: 100,
        }
    }

    fn engine_progress(percent: u8) -> ProgressEvent {
        ProgressEvent::Engine {
            stage: Stage::Analyzing,
            language: None,
            event: EngineEvent::progress(percent, "test", "progress", "msg"),
        }
    }

    fn engine_manifest(stage: Stage, lang: &str, total: usize) -> ProgressEvent {
        ProgressEvent::Engine {
            stage,
            language: Some(lang.into()),
            event: EngineEvent::FileManifest {
                total_files: total,
                files: vec![],
                language: Some(lang.into()),
            },
        }
    }

    fn engine_file(stage: Stage, lang: &str, path: &str, status: &str) -> ProgressEvent {
        ProgressEvent::Engine {
            stage,
            language: Some(lang.into()),
            event: EngineEvent::FileProgress {
                file_path: path.into(),
                file_index: 0,
                status: status.into(),
                language: Some(lang.into()),
            },
        }
    }

    #[test]
    fn starts_at_zero() {
        let agg = ProgressAggregator::new();
        assert_eq!(agg.view().overall_percent, 0);
        assert!(agg.view().stage.is_none());
    }

    #[test]
    fn preparing_stage_sets_floor() {
        let mut agg = ProgressAggregator::new();
        agg.apply(&stage_started(Stage::Preparing, None));
        let v = agg.view();
        assert_eq!(v.stage, Some(Stage::Preparing));
        assert_eq!(v.overall_percent, stage_window(Stage::Preparing).0);
    }

    #[test]
    fn parsing_one_language_interpolates_with_files() {
        let mut agg = ProgressAggregator::new();
        agg.apply(&stage_started(Stage::Parsing, None));
        agg.apply(&stage_started(Stage::Parsing, Some("typescript")));
        agg.apply(&engine_manifest(Stage::Parsing, "typescript", 4));

        let p_before = agg.view().overall_percent;
        agg.apply(&engine_file(Stage::Parsing, "typescript", "a.ts", "done"));
        let p_after_one = agg.view().overall_percent;
        assert!(
            p_after_one > p_before,
            "percent should move forward after first file done"
        );

        agg.apply(&engine_file(Stage::Parsing, "typescript", "b.ts", "done"));
        agg.apply(&engine_file(Stage::Parsing, "typescript", "c.ts", "done"));
        agg.apply(&engine_file(Stage::Parsing, "typescript", "d.ts", "done"));

        let (_, ceil) = stage_window(Stage::Parsing);
        // After all files of the only language complete, we should be
        // at or near the upper bound of the parsing window (the
        // language sub-stage hasn't "finished" yet — that fires when
        // StageFinished arrives, so we expect within 1pt of ceil).
        assert!(agg.view().overall_percent <= ceil);
        assert!(agg.view().overall_percent >= ceil - 1);
    }

    #[test]
    fn parsing_two_languages_keeps_progress_monotone() {
        let mut agg = ProgressAggregator::new();
        agg.apply(&stage_started(Stage::Parsing, None));

        // Language 1: parses 2 files.
        agg.apply(&stage_started(Stage::Parsing, Some("typescript")));
        agg.apply(&engine_manifest(Stage::Parsing, "typescript", 2));
        agg.apply(&engine_file(Stage::Parsing, "typescript", "a.ts", "done"));
        agg.apply(&engine_file(Stage::Parsing, "typescript", "b.ts", "done"));
        let p_after_lang1 = agg.view().overall_percent;
        agg.apply(&stage_finished(Stage::Parsing, Some("typescript")));

        // Language 2: parses 2 files.
        agg.apply(&stage_started(Stage::Parsing, Some("python")));
        agg.apply(&engine_manifest(Stage::Parsing, "python", 2));
        let p_lang2_start = agg.view().overall_percent;
        // Manifest resets per-lang counters; overall should never go
        // backwards across the language transition.
        assert!(
            p_lang2_start >= p_after_lang1 || p_lang2_start + 1 >= p_after_lang1,
            "p_lang2_start={p_lang2_start} should be roughly >= p_after_lang1={p_after_lang1}"
        );
    }

    #[test]
    fn engine_progress_during_analyzing_scales_into_window() {
        let mut agg = ProgressAggregator::new();
        agg.apply(&stage_started(Stage::Analyzing, None));
        agg.apply(&engine_progress(50));
        let (base, ceil) = stage_window(Stage::Analyzing);
        let v = agg.view();
        assert!(v.overall_percent >= base, "below base");
        assert!(v.overall_percent <= ceil, "above ceiling");
        // 50% of the analyzing window's range, plus base.
        let expected = base as f32 + 0.5 * (ceil - base) as f32;
        let delta = (v.overall_percent as f32 - expected).abs();
        assert!(delta <= 1.0, "delta {delta} too large");
    }

    #[test]
    fn engine_progress_caps_at_100() {
        let mut agg = ProgressAggregator::new();
        agg.apply(&stage_started(Stage::Analyzing, None));
        agg.apply(&engine_progress(100));
        let (_, ceil) = stage_window(Stage::Analyzing);
        assert_eq!(agg.view().overall_percent, ceil);
    }

    #[test]
    fn manifest_sets_files_total_and_resets_done() {
        let mut agg = ProgressAggregator::new();
        agg.apply(&stage_started(Stage::Parsing, None));
        agg.apply(&stage_started(Stage::Parsing, Some("ts")));
        agg.apply(&engine_manifest(Stage::Parsing, "ts", 17));
        assert_eq!(agg.view().files_total, Some(17));
        assert_eq!(agg.view().files_done, 0);
    }

    #[test]
    fn ratchet_holds_when_engine_progress_drops_within_parsing() {
        // Real-world regression: the parser emits its own per-phase
        // percents (Discovery=15, Syntax=20, Symbols=38, ...). After
        // a FileProgress {status: "done"} pushes the bar to the top
        // of the parsing window, the next parser Progress event with
        // a lower engine_percent must NOT drag the bar backwards.
        let mut agg = ProgressAggregator::new();
        agg.apply(&stage_started(Stage::Parsing, None));
        agg.apply(&stage_started(Stage::Parsing, Some("rust")));
        agg.apply(&engine_manifest(Stage::Parsing, "rust", 1));
        agg.apply(&engine_file(Stage::Parsing, "rust", "src/lib.rs", "done"));
        let high = agg.view().overall_percent;
        assert!(
            high >= 60,
            "expected bar near top of parsing window after files done, got {high}"
        );
        // Parser emits "Extracted 7 symbols" at engine_pct=38.
        agg.apply(&ProgressEvent::Engine {
            stage: Stage::Parsing,
            language: Some("rust".into()),
            event: EngineEvent::progress_lang(38, "syntax", "done", "Extracted symbols", "rust"),
        });
        assert_eq!(
            agg.view().overall_percent,
            high,
            "ratchet must hold against an engine percent that scales below the current bar"
        );
    }

    #[test]
    fn raw_events_do_not_change_view() {
        let mut agg = ProgressAggregator::new();
        agg.apply(&stage_started(Stage::Parsing, Some("ts")));
        let before_pct = agg.view().overall_percent;
        let before_msg = agg.view().message.clone();
        agg.apply(&ProgressEvent::Raw {
            stage: Stage::Parsing,
            language: Some("ts".into()),
            line: "some tracing noise".into(),
        });
        assert_eq!(agg.view().overall_percent, before_pct);
        assert_eq!(agg.view().message, before_msg);
    }
}
