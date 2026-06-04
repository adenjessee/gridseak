//! Input type for a single semantic-resolution query. Deliberately
//! minimal — a file path and a (line, column) offset — so the adapter
//! has no compile-time dependency on `graphengine-parsing`'s domain
//! types. The translation from a parsing-layer
//! `UnresolvedReference::Call` (or `FrameworkBinding`) to this input
//! shape lives in the caller.

use std::path::PathBuf;

/// A single semantic-resolution request against the adapter's
/// `AnalysisHost`.
///
/// Coordinates follow the `graphengine-parsing` convention:
/// line is 1-based, column is 0-based byte offset into the line. The
/// adapter translates these into `ra_ap_ide`'s internal `FilePosition`
/// at resolution time, collapsing the off-by-one between the
/// engine's human-facing representation and rust-analyzer's
/// LineIndex/TextSize model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticQueryInput {
    /// Absolute path to the file containing the reference we want to
    /// resolve. Must be a path rust-analyzer's VFS knows about —
    /// in practice a file inside the workspace this adapter was
    /// constructed with.
    pub file: PathBuf,
    /// 1-based line number of the reference (matches
    /// `graphengine-parsing::domain::Range::start_line`).
    pub line: u32,
    /// 0-based column (byte offset within the line) of the reference.
    /// Matches `graphengine-parsing::domain::Range::start_char`.
    pub column: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_input_clones_and_equates() {
        let a = SemanticQueryInput {
            file: PathBuf::from("/tmp/foo.rs"),
            line: 1,
            column: 0,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }
}
