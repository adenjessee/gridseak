//! Silent renderer: emits nothing.
//!
//! Selected by `--no-progress`, `--quiet`, or `--progress off`. Also
//! selected internally by the CLI when stdout is being used for machine
//! output (`--json`) and the user did not explicitly ask for fancy
//! stderr UI — that case is debatable; for now the rule is "if you want
//! progress *and* JSON, ask for both explicitly," which keeps the
//! default JSON output strictly parseable.

use gridseak_engine_runner::ProgressEvent;

use crate::progress::aggregator::StageView;

use super::ProgressRenderer;

pub struct SilentRenderer;

impl ProgressRenderer for SilentRenderer {
    fn on_event(&mut self, _view: &StageView, _event: &ProgressEvent) {}
}
