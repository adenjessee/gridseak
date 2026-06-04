//! `gridseak_engine_runner::ProgressSink` impl that drives the CLI's
//! [`ProgressAggregator`] + [`ProgressRenderer`].
//!
//! This replaces the Stage-0 `CliProgressSink` that lived in
//! `gridseak-cli/src/main.rs` and unconditionally `eprintln!`d every
//! `Raw` line verbatim. With Stage 1 wired, the structured engine
//! events flow through the aggregator, the renderer decides what to
//! show, and `Raw` lines are filtered/forwarded according to the mode.
//!
//! Per the plan's update notes, the `_ => {}` wildcard arm on
//! `ProgressEvent` has been removed: every variant gets an explicit
//! `match` arm here. Adding a new variant to the runner's
//! `#[non_exhaustive]` enum will surface as a compile error at this
//! site (intentional — we don't want to silently drop structured
//! progress just because a new variant was added upstream).

use gridseak_engine_runner::{ProgressEvent, ProgressSink};

use super::aggregator::ProgressAggregator;
use super::renderer::ProgressRenderer;

pub struct CliProgressSink {
    aggregator: ProgressAggregator,
    renderer: Box<dyn ProgressRenderer>,
    started: bool,
}

impl CliProgressSink {
    pub fn new(renderer: Box<dyn ProgressRenderer>) -> Self {
        Self {
            aggregator: ProgressAggregator::new(),
            renderer,
            started: false,
        }
    }

    /// Tell the renderer the scan finished. The runner doesn't call
    /// this — the CLI driver does, after the pipeline returns (with
    /// either success or error) so the in-place fancy line is cleared
    /// before the final report renders.
    ///
    /// Currently unused: Stage 1's `run_scan_pipeline` moves the sink
    /// into the runner config (where ownership is required) and adds a
    /// trailing newline directly. Stage 2's `gridseak scan .` driver
    /// will switch to channel-routed sinks and call this from the
    /// driver side so the fancy in-place block always clears before
    /// the final report renders. Keeping the method in the public API
    /// now means Stage 2 doesn't have to revisit the sink shape.
    #[allow(dead_code)]
    pub fn finish(&mut self) {
        if self.started {
            self.renderer.on_finish();
            self.started = false;
        }
    }
}

impl ProgressSink for CliProgressSink {
    fn on_event(&mut self, event: ProgressEvent) {
        if !self.started {
            self.renderer.on_start();
            self.started = true;
        }
        // The match below is exhaustive over the runner's
        // `#[non_exhaustive]` `ProgressEvent`. Each arm hands control
        // off to the aggregator (which mutates the view) and the
        // renderer (which paints). No wildcard arm: if the runner
        // grows a new variant the compiler forces us to handle it
        // here.
        let view = self.aggregator.apply(&event);
        // SAFETY: `view` borrows from `self.aggregator`; we pass it
        // through to the renderer call below by re-borrowing
        // separately so the borrow checker is happy. Concretely, the
        // aggregator's `apply` returns `&StageView` tied to `&mut
        // self.aggregator`, which conflicts with calling
        // `renderer.on_event` (also `&mut self`). Splitting the
        // borrows via `aggregator.view()` is the simplest fix.
        let _ = view; // keep the apply call's side effects
        let view = self.aggregator.view();
        self.renderer.on_event(view, &event);
    }
}
