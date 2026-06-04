//! Renderer trait and the factory that selects a concrete impl from a
//! `--progress` flag.
//!
//! Renderers consume a fully-aggregated [`StageView`] plus the raw
//! [`ProgressEvent`]. The view gives them "what is happening" without
//! re-implementing state tracking; the raw event lets a verbose
//! renderer also surface `Raw` lines and a future fancy renderer
//! handle `EngineEvent::FileProgress` separately (e.g. to scroll a
//! current-file ticker).
//!
//! Why a trait + dyn dispatch instead of an enum with a `match` per
//! call: renderers are stateful (the fancy renderer holds an in-place
//! line position and a frame budget; the plain renderer holds the last
//! line printed for change-detection). State on an enum requires
//! awkward field-per-variant access; a trait keeps each impl in its
//! own file with clean boundaries.

use std::str::FromStr;

use gridseak_engine_runner::ProgressEvent;

use super::aggregator::StageView;

pub mod fancy;
pub mod plain;
pub mod silent;

/// User-facing rendering mode, mapped from the `--progress` CLI flag.
///
/// `auto` is what the default resolves to before TTY/CI inspection;
/// after [`ProgressMode::resolve_auto`] runs, only the concrete modes
/// (`Fancy`, `Plain`, `Silent`) remain. Renderers never see `Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    /// Decide based on TTY + `CI` env var at startup.
    Auto,
    /// Animated in-place rendering. Requires a TTY.
    Fancy,
    /// Line-oriented append-only output. Safe for logs and pipes.
    Plain,
    /// Emit nothing. Used by `--no-progress` / `--quiet` and by `--json`
    /// when the CLI wants stdout to be the only output channel.
    Silent,
}

impl ProgressMode {
    /// Resolve `Auto` based on whether stderr is a TTY and whether
    /// `CI` is set. Returns `Self` unchanged for concrete modes so the
    /// caller can always trust the returned value to be renderable.
    ///
    /// We probe stderr (not stdout) because progress output goes to
    /// stderr; a user piping stdout into `jq` is still entitled to a
    /// fancy stderr UI.
    pub fn resolve_auto(self) -> Self {
        use std::io::IsTerminal;
        match self {
            ProgressMode::Auto => {
                let stderr_is_tty = std::io::stderr().is_terminal();
                let is_ci = std::env::var("CI")
                    .map(|v| !v.is_empty() && v != "0" && v != "false")
                    .unwrap_or(false);
                if stderr_is_tty && !is_ci {
                    ProgressMode::Fancy
                } else {
                    ProgressMode::Plain
                }
            }
            other => other,
        }
    }

    /// Build a fresh renderer for this mode. Always returns a concrete
    /// renderer; callers must call [`Self::resolve_auto`] first if they
    /// started from `Auto`.
    pub fn into_renderer(self) -> Box<dyn ProgressRenderer> {
        match self.resolve_auto() {
            ProgressMode::Fancy => Box::new(fancy::FancyRenderer::new()),
            ProgressMode::Plain => Box::new(plain::PlainRenderer::new()),
            ProgressMode::Silent => Box::new(silent::SilentRenderer),
            ProgressMode::Auto => unreachable!("resolve_auto() returned Auto"),
        }
    }
}

impl FromStr for ProgressMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "fancy" => Ok(Self::Fancy),
            "plain" => Ok(Self::Plain),
            "off" | "silent" | "none" => Ok(Self::Silent),
            other => Err(format!(
                "unknown progress mode `{other}` (expected one of: auto, fancy, plain, off)"
            )),
        }
    }
}

/// Sink-side renderer trait. Implementations decide how to translate a
/// view + event pair into bytes on stderr.
///
/// `&mut self` because renderers carry per-render state (last-frame
/// timestamp, last-line content, scroll position). The CLI owns the
/// renderer for the duration of the scan; there's no concurrent access.
///
/// `Send + Sync` is required transitively: the runner's
/// [`gridseak_engine_runner::RunPipelineConfig::progress`] field is
/// `Box<dyn ProgressSink + Send + Sync>`, and our [`CliProgressSink`]
/// holds a renderer. The bound on the trait makes the chain typecheck
/// without forcing each renderer's struct to opt in individually
/// (every concrete renderer here is `Send + Sync` by default — they
/// hold only `Option<Instant>` and `String` fields).
pub trait ProgressRenderer: Send + Sync {
    /// Called once when the pipeline begins, before any events. The
    /// plain renderer prints a one-line header; the fancy renderer
    /// reserves screen real estate. The silent renderer is a no-op.
    fn on_start(&mut self) {}

    /// Called for every event the sink receives, immediately after the
    /// aggregator has updated `view`. `event` is the original event so
    /// verbose modes can show raw lines without the aggregator needing
    /// to preserve them.
    fn on_event(&mut self, view: &StageView, event: &ProgressEvent);

    /// Called once when the pipeline ends (success, failure, or
    /// cancel). The fancy renderer clears its in-place block so the
    /// final report renders cleanly on a fresh line; the plain
    /// renderer emits a summary line.
    ///
    /// Currently unused at call sites: see
    /// [`crate::progress::sink::CliProgressSink::finish`] for why.
    /// Stage 2 wires it in. Default body is a no-op; renderers
    /// override only when they need to clean up screen state.
    #[allow(dead_code)]
    fn on_finish(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_modes() {
        assert_eq!(ProgressMode::from_str("auto").unwrap(), ProgressMode::Auto);
        assert_eq!(
            ProgressMode::from_str("fancy").unwrap(),
            ProgressMode::Fancy
        );
        assert_eq!(
            ProgressMode::from_str("plain").unwrap(),
            ProgressMode::Plain
        );
        assert_eq!(ProgressMode::from_str("off").unwrap(), ProgressMode::Silent);
        assert_eq!(
            ProgressMode::from_str("silent").unwrap(),
            ProgressMode::Silent
        );
        assert_eq!(
            ProgressMode::from_str("none").unwrap(),
            ProgressMode::Silent
        );
    }

    #[test]
    fn rejects_unknown_mode() {
        let err = ProgressMode::from_str("turbo").unwrap_err();
        assert!(err.contains("unknown progress mode"));
        assert!(err.contains("turbo"));
    }

    #[test]
    fn auto_resolves_to_concrete_mode() {
        // We can't easily fake TTY+CI in a unit test, but we can assert
        // that auto resolves to something that is *not* Auto.
        let resolved = ProgressMode::Auto.resolve_auto();
        assert_ne!(resolved, ProgressMode::Auto);
        // And concrete modes pass through unchanged.
        assert_eq!(ProgressMode::Plain.resolve_auto(), ProgressMode::Plain);
        assert_eq!(ProgressMode::Silent.resolve_auto(), ProgressMode::Silent);
    }
}
