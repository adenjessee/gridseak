//! Output rendering for the `gridseak` CLI.
//!
//! This module owns everything that turns persisted `HealthReport` +
//! `ProjectDto` + `ScanRunDto` rows into terminal-friendly bytes. The
//! same `ScanReportView` powers every renderer so the hero number,
//! the priority list, and the metric table never disagree across
//! formats — a Markdown export should say the same thing as the
//! terminal table, just shaped differently for its target medium.
//!
//! Stage 2 ships:
//! - [`view::ScanReportView`] — the format-agnostic model the
//!   renderers consume.
//! - [`width::detect`] — terminal-width probing + layout selection so
//!   the table degrades gracefully on narrow terminals and pipes.
//! - [`table::render_hero`] — the wide/medium/narrow table renderer
//!   that produces the spec's "Example Output" (§Target First-Run UX).
//!
//! Stage 3 adds full markdown / for-llm / json renderers built on the
//! same view model; the placeholder modules at [`markdown`], [`llm`],
//! and [`json`] keep the Stage 2 flag surface honest so users do not
//! see "flag not implemented" for any documented option.

pub mod compare;
pub mod findings;
pub mod graph;
pub mod history;
pub mod json;
pub mod llm;
pub mod markdown;
pub mod metrics;
pub mod recommendations;
pub mod table;
pub mod tier_signaling;
pub mod trends;
pub mod util;
pub mod view;
pub mod width;

use std::io::{self, Write};

#[allow(unused_imports)]
pub use view::ScanReportView;
#[allow(unused_imports)]
pub use width::Layout;

/// Unified dispatch for "render this view in the format the user
/// asked for". Owning this enum + match in one place keeps Stage 4
/// (history/compare/trends) and Stage 5 (recommendations/findings/
/// metrics) from re-implementing the same `if --for-llm else if
/// --json else if --markdown else …` ladder.
///
/// Construction is intentionally `pub` per variant: callers decide
/// which renderer to use; this module decides how that decision maps
/// onto bytes on the wire.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum HeroFormat {
    /// Width-aware terminal table.
    Table { layout: Layout },
    /// GitHub-flavored Markdown.
    Markdown,
    /// Stable JSON envelope.
    Json,
    /// LLM-friendly text with optional token budget.
    ForLlm { budget: Option<usize> },
}

/// Render `view` in the requested format. Stage 4+ commands that
/// share the hero shape (notably `gridseak scan latest` and parts of
/// `gridseak compare`) call this directly.
#[allow(dead_code)]
pub fn render_hero(
    format: &HeroFormat,
    view: &ScanReportView,
    out: &mut dyn Write,
) -> io::Result<()> {
    match format {
        HeroFormat::Table { layout } => table::render_hero(view, *layout, out),
        HeroFormat::Markdown => markdown::render_hero(view, out),
        HeroFormat::Json => json::render_hero(view, out),
        HeroFormat::ForLlm { budget } => llm::render(view, *budget, out).map(|_| ()),
    }
}
