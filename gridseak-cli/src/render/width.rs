//! Terminal width detection + layout selection.
//!
//! Why this lives in its own module: rendering decisions ("is this a
//! wide table or a stacked card?") are inputs to several renderers
//! (table today, markdown's PR-vs-issue framing tomorrow). Centralising
//! the probe means we can override it from tests with one constant
//! rather than monkey-patching every renderer.

/// The four layout buckets renderers branch on. Thresholds are tuned
/// for the spec's "Example Output" (lines 146-178): the wide table
/// needs ≈80 columns to keep one priority row on one line, the
/// medium layout drops the Severity column, and the narrow layout
/// stacks each row as a key/value card. Plain text is the safe
/// fallback for pipes and ≤39-col terminals where even cards wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// ≥ 100 cols. Full table with every column.
    Wide,
    /// 70..=99 cols. Table with compact column set.
    Medium,
    /// 40..=69 cols. Stacked key/value cards.
    Narrow,
    /// < 40 cols, no TTY, or a pipe. One label-per-line text.
    Plain,
}

impl Layout {
    /// Map a column count to a layout. Returns `Plain` when the count
    /// is `None` (no TTY detected — typical for pipes and CI).
    pub fn from_columns(cols: Option<u16>) -> Self {
        match cols {
            None => Self::Plain,
            Some(c) if c >= 100 => Self::Wide,
            Some(c) if c >= 70 => Self::Medium,
            Some(c) if c >= 40 => Self::Narrow,
            Some(_) => Self::Plain,
        }
    }

    /// True if this layout should attempt tabular output rather than
    /// cards/plain text. Reserved for the rule-of-three: every
    /// renderer that wants to short-circuit table layout (e.g. CSV
    /// in a future stage) can branch on it without re-encoding the
    /// thresholds.
    #[allow(dead_code)]
    pub fn is_tabular(&self) -> bool {
        matches!(self, Self::Wide | Self::Medium)
    }
}

/// Probe the active terminal and return the selected layout.
///
/// Override sequence:
/// 1. `GRIDSEAK_FORCE_LAYOUT={wide|medium|narrow|plain}` — primarily
///    for snapshot tests but also useful for CI logs that benefit
///    from a specific layout regardless of terminal capabilities.
/// 2. `COLUMNS=<n>` — POSIX-standard column hint; respected so a
///    `COLUMNS=120 gridseak scan .` produces the same wide layout
///    in a constrained TTY as in a real wide one.
/// 3. `terminal_size::terminal_size()` against the *standard output*
///    handle. We deliberately probe stdout (not stderr) because
///    layout decisions affect what we write to stdout; if stdout
///    is a pipe (no TTY) the result is `None` and we fall back to
///    `Plain`, which is the right default for pipelined output.
pub fn detect() -> Layout {
    if let Ok(forced) = std::env::var("GRIDSEAK_FORCE_LAYOUT") {
        if let Some(layout) = parse_forced_layout(&forced) {
            return layout;
        }
    }
    if let Ok(cols) = std::env::var("COLUMNS") {
        if let Ok(n) = cols.trim().parse::<u16>() {
            return Layout::from_columns(Some(n));
        }
    }
    let cols = terminal_size::terminal_size_of(std::io::stdout()).map(|(w, _)| w.0);
    Layout::from_columns(cols)
}

fn parse_forced_layout(raw: &str) -> Option<Layout> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "wide" => Some(Layout::Wide),
        "medium" => Some(Layout::Medium),
        "narrow" => Some(Layout::Narrow),
        "plain" => Some(Layout::Plain),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_match_threshold_boundaries() {
        assert_eq!(Layout::from_columns(Some(100)), Layout::Wide);
        assert_eq!(Layout::from_columns(Some(99)), Layout::Medium);
        assert_eq!(Layout::from_columns(Some(70)), Layout::Medium);
        assert_eq!(Layout::from_columns(Some(69)), Layout::Narrow);
        assert_eq!(Layout::from_columns(Some(40)), Layout::Narrow);
        assert_eq!(Layout::from_columns(Some(39)), Layout::Plain);
        assert_eq!(Layout::from_columns(None), Layout::Plain);
    }

    #[test]
    fn parse_forced_layout_round_trips() {
        assert_eq!(parse_forced_layout("wide"), Some(Layout::Wide));
        assert_eq!(parse_forced_layout(" MEDIUM "), Some(Layout::Medium));
        assert_eq!(parse_forced_layout("plain"), Some(Layout::Plain));
        assert_eq!(parse_forced_layout("garbage"), None);
    }
}
