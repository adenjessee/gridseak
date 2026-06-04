//! CLI-side consumption of `gridseak_engine_runner::ProgressEvent`.
//!
//! # Why this module exists
//!
//! The engine runner emits a four-variant `ProgressEvent` stream:
//! `StageStarted`, `StageFinished`, `Engine(EngineEvent)`, and `Raw`.
//! The CLI needs to translate those into terminal output that matches
//! the shadow-mode spec's progress requirements — truthful percentages,
//! current-file display, width-aware rendering, TTY/CI mode detection,
//! and a clean handoff from progress noise to the final report block.
//!
//! Stage 1 lands the consumer machinery. Stage 2 makes the `fancy`
//! renderer pretty (in-place ANSI updates, file counters, current-file
//! truncation). The shape is:
//!
//! ```text
//!     ProgressEvent (from runner)
//!         │
//!         ▼
//!     ProgressAggregator (stateful: which stage, what %, current file)
//!         │
//!         ▼
//!     dyn ProgressRenderer (plain | silent | fancy)
//!         │
//!         ▼
//!     stderr (so stdout stays parseable when the user pipes --json)
//! ```
//!
//! The renderer trait is dyn-dispatch so the `--progress` CLI flag picks
//! the implementation at runtime without monomorphising the whole
//! pipeline driver.
//!
//! # Why progress goes to stderr
//!
//! The shadow-mode spec is explicit: `--json` keeps stdout parseable;
//! the human progress UI lives on stderr. The CLI's stdout is reserved
//! for the JSON/markdown/table report block. This module never writes
//! to stdout.

pub mod aggregator;
pub mod renderer;
pub mod sink;

pub use renderer::ProgressMode;
pub use sink::CliProgressSink;

// Aggregator + StageView + ProgressRenderer are re-exported through
// their submodules; main.rs uses ProgressMode + CliProgressSink only.
// Stage 2 will surface ProgressAggregator if the report builder needs
// to peek at the live view; until then, leave them as crate-internal.
