//! Port for batch / in-process semantic indexes (SCIP, rust-analyzer library).
//!
//! Adapters implement [`SemanticIndex`] and return language-neutral
//! [`IndexTarget`] values. Call-site → graph-node mapping lives in
//! [`crate::infrastructure::semantic::symbol_mapping`].

use std::path::{Path, PathBuf};

use crate::domain::Confidence;

/// A resolved definition returned by a semantic index query.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexTarget {
    pub file: PathBuf,
    pub line: u32,
    pub col: u32,
    pub symbol_moniker: String,
    pub confidence: Confidence,
}

/// Offline semantic index (SCIP file, in-process analyzer snapshot, …).
pub trait SemanticIndex {
    /// Resolve the definition at `(file, line, col)` in the indexed workspace.
    fn definition_at(&self, file: &Path, line: u32, col: u32) -> Option<IndexTarget>;

    /// Whether this file has a document in the index.
    ///
    /// Default `true` keeps mock indexes permissive. SCIP returns `false`
    /// for republish / example trees the indexer skipped so heuristic
    /// name-match does not invent Call edges the compiler never saw.
    fn has_file(&self, _file: &Path) -> bool {
        true
    }

    /// Language tag for disclosure / routing (`"rust"`, `"typescript"`, …).
    fn language(&self) -> &str;

    /// Implementing symbols for `symbol` (may-call set). Default: empty.
    fn implementations_of(&self, _symbol: &str) -> Vec<IndexTarget> {
        Vec::new()
    }

    /// Every definition occurrence of `symbol`. Several when the indexer
    /// reuses one symbol for distinct same-named declarations (scip-python:
    /// functions nested in different methods of one class). Default: empty.
    fn declarations_of(&self, _symbol: &str) -> Vec<IndexTarget> {
        Vec::new()
    }

    /// Definition of `symbol`, if the index has one. Default: `None`.
    fn definition_of(&self, _symbol: &str) -> Option<IndexTarget> {
        None
    }

    /// Direct supertypes of the type `symbol` (its base classes), in
    /// declaration order. Default: empty.
    fn supertypes_of(&self, _symbol: &str) -> Vec<String> {
        Vec::new()
    }

    /// Module-level classes and functions named `name`. Default: empty.
    fn module_level_definitions(&self, _name: &str) -> Vec<IndexTarget> {
        Vec::new()
    }

    /// Start position (0-based line, column) and symbol of every occurrence
    /// the index records in `file`, or `None` when unknown. Default: `None`.
    fn occurrence_starts(&self, _file: &Path) -> Option<Vec<(u32, u32, String)>> {
        None
    }

    /// Reference sites of `symbol` (second caller witness). Default: empty.
    fn references_of(&self, _symbol: &str) -> Vec<IndexTarget> {
        Vec::new()
    }

    /// Every `(reference site, definition)` pair the index knows, or `None`
    /// when the index cannot enumerate occurrences. When `Some`, the
    /// resolver treats the index as the sole authority for indexed files
    /// instead of mapping tree-sitter call sites one caret at a time.
    fn reference_occurrences(&self) -> Option<Vec<(IndexTarget, IndexTarget)>> {
        None
    }
}
