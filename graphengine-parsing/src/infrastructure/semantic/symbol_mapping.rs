//! Shared caller/callee mapping from semantic index targets to tree-sitter nodes.
//!
//! Adapter-agnostic: accepts [`IndexTarget`] from rust-analyzer, SCIP, or any
//! future [`SemanticIndex`] implementation. rust-analyzer-specific caret logic
//! stays in `rust_layer2.rs`.

use std::path::Path;

use crate::application::ports::IndexTarget;
use crate::domain::{Node, NodeKind, Range};

/// Caller / callee index over the scan's flat `Vec<Node>`. Rebuilt once per
/// `resolve()` call because the symbol list is already in memory.
pub struct SymbolIndex<'a> {
    /// `file path → [symbols in that file, sorted by start_line]`.
    pub(crate) by_file: std::collections::HashMap<&'a str, Vec<&'a Node>>,
    /// Flat symbol list for workspace-wide fallbacks.
    pub(crate) all_functions: Vec<&'a Node>,
}

impl<'a> SymbolIndex<'a> {
    pub fn new(symbols: &'a [Node]) -> Self {
        let mut by_file: std::collections::HashMap<&'a str, Vec<&'a Node>> =
            std::collections::HashMap::new();
        let mut all_functions = Vec::new();
        for node in symbols {
            if node.kind == NodeKind::Function {
                all_functions.push(node);
            }
            by_file
                .entry(node.location.file.as_str())
                .or_default()
                .push(node);
        }
        for v in by_file.values_mut() {
            v.sort_by_key(|n| (n.location.start_line, n.location.start_char));
        }
        Self {
            by_file,
            all_functions,
        }
    }

    /// Tightest `Function`-kind node whose range contains `location`.
    pub fn find_enclosing_function(&self, location: &Range) -> Option<&'a Node> {
        let file_symbols = self.file_symbols_for_location(location)?;
        if let Some(n) = Self::tightest_enclosing_function(file_symbols, location) {
            return Some(n);
        }
        file_symbols
            .iter()
            .copied()
            .filter(|n| n.kind == NodeKind::Function && line_encloses(location, &n.location))
            .min_by_key(|n| range_span(&n.location))
    }

    fn file_symbols_for_location(&self, location: &Range) -> Option<&Vec<&'a Node>> {
        if let Some(syms) = self.by_file.get(location.file.as_str()) {
            return Some(syms);
        }
        self.by_file
            .iter()
            .find(|(k, _)| path_matches(k, &location.file))
            .map(|(_, v)| v)
    }

    fn tightest_enclosing_function(
        file_symbols: &[&'a Node],
        location: &Range,
    ) -> Option<&'a Node> {
        file_symbols
            .iter()
            .copied()
            .filter(|n| n.kind == NodeKind::Function && contains(&n.location, location))
            .min_by_key(|n| range_span(&n.location))
    }

    /// Node whose `(file, kind, range)` best matches an [`IndexTarget`].
    pub fn find_callee_for(&self, target: &IndexTarget) -> Option<&'a Node> {
        let target_path = target.file.to_string_lossy();
        let name = target.symbol_moniker.as_str();

        let Some(file_symbols) = self
            .by_file
            .iter()
            .find(|(k, _)| path_matches(k, target_path.as_ref()))
            .or_else(|| {
                self.by_file
                    .iter()
                    .find(|(k, _)| target_path.ends_with(**k))
            })
            .map(|(_, v)| v.as_slice())
        else {
            return self.find_callee_by_name_workspace(name, target.line);
        };

        let line_hits: Vec<&Node> = file_symbols
            .iter()
            .copied()
            .filter(|n| n.kind == NodeKind::Function)
            .filter(|n| line_contains_target(n, target.line))
            .collect();

        if !line_hits.is_empty() {
            if let Some(exact) = line_hits.iter().find(|n| fqn_ends_with(&n.fqn, name)) {
                return Some(*exact);
            }
            if line_hits.len() == 1 {
                return Some(line_hits[0]);
            }
            return line_hits
                .into_iter()
                .min_by_key(|n| line_distance_to_target(n, target.line));
        }

        let name_hits: Vec<&Node> = file_symbols
            .iter()
            .copied()
            .filter(|n| n.kind == NodeKind::Function && fqn_ends_with(&n.fqn, name))
            .collect();
        match name_hits.len() {
            0 => {
                if let Some(n) = Self::nearest_function_on_file(file_symbols, target.line, 3) {
                    return Some(n);
                }
                None
            }
            1 => Some(name_hits[0]),
            _ => name_hits
                .into_iter()
                .min_by_key(|n| line_distance_to_target(n, target.line)),
        }
    }

    fn nearest_function_on_file(
        file_symbols: &[&'a Node],
        target_line: u32,
        max_line_distance: u32,
    ) -> Option<&'a Node> {
        file_symbols
            .iter()
            .copied()
            .filter(|n| n.kind == NodeKind::Function)
            .filter(|n| line_distance_to_target(n, target_line) <= max_line_distance)
            .min_by_key(|n| line_distance_to_target(n, target_line))
    }

    fn find_callee_by_name_workspace(&self, name: &str, target_line: u32) -> Option<&'a Node> {
        let hits: Vec<&Node> = self
            .all_functions
            .iter()
            .copied()
            .filter(|n| fqn_ends_with(&n.fqn, name))
            .collect();
        match hits.len() {
            0 => None,
            1 => Some(hits[0]),
            _ => hits
                .into_iter()
                .min_by_key(|n| line_distance_to_target(n, target_line)),
        }
    }

    /// Whether any indexed function's FQN ends with `name`.
    pub fn function_name_in_index(&self, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        self.all_functions
            .iter()
            .any(|n| fqn_ends_with(&n.fqn, name))
    }

    /// Whether a function matching `target_symbol_name` exists under `target_path`.
    pub fn target_symbol_in_file(&self, target_path: &Path, target_symbol_name: &str) -> bool {
        let target_path = target_path.to_string_lossy();
        self.all_functions.iter().any(|n| {
            path_matches(&n.location.file, target_path.as_ref())
                && fqn_ends_with(&n.fqn, target_symbol_name)
        })
    }
}

fn line_encloses(inner: &Range, outer: &Range) -> bool {
    inner.start_line >= outer.start_line && inner.start_line <= outer.end_line
}

fn line_contains_target(node: &Node, target_line: u32) -> bool {
    let start = node.location.start_line.saturating_sub(1);
    target_line >= start && target_line <= node.location.end_line
}

fn line_distance_to_target(node: &Node, target_line: u32) -> u32 {
    if target_line < node.location.start_line {
        node.location.start_line - target_line
    } else {
        target_line.saturating_sub(node.location.end_line)
    }
}

fn contains(outer: &Range, inner: &Range) -> bool {
    if outer.file != inner.file {
        return false;
    }
    let outer_start = (outer.start_line, outer.start_char);
    let outer_end = (outer.end_line, outer.end_char);
    let inner_start = (inner.start_line, inner.start_char);
    let inner_end = (inner.end_line, inner.end_char);
    outer_start <= inner_start && inner_end <= outer_end
}

fn range_span(range: &Range) -> (u32, u32) {
    let line_span = range.end_line.saturating_sub(range.start_line);
    let col_span = if range.end_line == range.start_line {
        range.end_char.saturating_sub(range.start_char)
    } else {
        0
    };
    (line_span, col_span)
}

pub(crate) fn path_matches(symbol_path: &str, target_path: &str) -> bool {
    if symbol_path == target_path {
        return true;
    }
    if target_path.ends_with(symbol_path) || symbol_path.ends_with(target_path) {
        return true;
    }
    match (
        std::path::Path::new(symbol_path).canonicalize().ok(),
        std::path::Path::new(target_path).canonicalize().ok(),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

pub(crate) fn fqn_ends_with(fqn: &str, name: &str) -> bool {
    if fqn == name {
        return true;
    }
    if !fqn.ends_with(name) {
        return false;
    }
    let Some(prefix_end) = fqn.len().checked_sub(name.len()) else {
        return false;
    };
    !matches!(
        fqn[..prefix_end].chars().last(),
        Some(c) if c.is_alphanumeric() || c == '_'
    )
}

#[cfg(test)]
mod fqn_tests {
    use super::*;

    #[test]
    fn suffix_name_with_colon_boundary_matches() {
        assert!(fqn_ends_with("crate::module::helper", "helper"));
    }

    #[test]
    fn partial_name_match_rejected() {
        assert!(!fqn_ends_with("crate::module::my_helper", "helper"));
    }

    #[test]
    fn exact_match() {
        assert!(fqn_ends_with("helper", "helper"));
    }

    #[test]
    fn empty_name_rejected_for_non_equal() {
        assert!(!fqn_ends_with("helper", ""));
    }
}

#[cfg(test)]
mod symbol_index_tests {
    use super::*;
    use crate::application::ports::IndexTarget;
    use crate::domain::{Confidence, Node, NodeKind, Provenance, Range as DomainRange};

    fn fn_node(fqn: &str, file: &str, range: DomainRange) -> Node {
        Node::new(
            NodeKind::Function,
            fqn.to_string(),
            range.with_file_path(file),
            Provenance::tree_sitter(),
        )
    }

    #[test]
    fn enclosing_function_picks_tightest() {
        let outer = fn_node(
            "crate::outer",
            "a.rs",
            DomainRange::with_file(1, 0, 30, 0, "a.rs"),
        );
        let inner = fn_node(
            "crate::outer::inner",
            "a.rs",
            DomainRange::with_file(10, 4, 20, 4, "a.rs"),
        );
        let symbols = vec![outer.clone(), inner.clone()];
        let idx = SymbolIndex::new(&symbols);
        let site = DomainRange::with_file(15, 8, 15, 12, "a.rs");
        let got = idx.find_enclosing_function(&site).expect("enclosing");
        assert_eq!(got.fqn, inner.fqn);
    }

    #[test]
    fn callee_lookup_matches_by_line_and_name() {
        let sym = fn_node(
            "crate::target::callee",
            "b.rs",
            DomainRange::with_file(12, 0, 18, 0, "b.rs"),
        );
        let symbols = vec![sym.clone()];
        let idx = SymbolIndex::new(&symbols);
        let target = IndexTarget {
            file: std::path::PathBuf::from("b.rs"),
            line: 12,
            col: 3,
            symbol_moniker: "callee".into(),
            confidence: Confidence::High,
        };
        let got = idx.find_callee_for(&target).expect("callee");
        assert_eq!(got.id, sym.id);
    }

    #[test]
    fn callee_lookup_prefers_exact_name_when_multiple_match_line() {
        let a = fn_node(
            "crate::helpers::callee",
            "c.rs",
            DomainRange::with_file(20, 0, 30, 0, "c.rs"),
        );
        let b = fn_node(
            "crate::helpers::other",
            "c.rs",
            DomainRange::with_file(20, 40, 30, 50, "c.rs"),
        );
        let symbols = vec![a.clone(), b.clone()];
        let idx = SymbolIndex::new(&symbols);
        let target = IndexTarget {
            file: std::path::PathBuf::from("c.rs"),
            line: 25,
            col: 0,
            symbol_moniker: "callee".into(),
            confidence: Confidence::High,
        };
        let got = idx.find_callee_for(&target).expect("callee");
        assert_eq!(got.id, a.id);
    }

    #[test]
    fn find_callee_falls_back_to_name_when_focus_line_outside_span() {
        let callee = fn_node(
            "crate::MyType::run",
            "impl_methods.rs",
            DomainRange::with_file(10, 0, 25, 1, "impl_methods.rs"),
        );
        let symbols = vec![callee.clone()];
        let idx = SymbolIndex::new(&symbols);
        let target = IndexTarget {
            file: std::path::PathBuf::from("impl_methods.rs"),
            line: 8,
            col: 4,
            symbol_moniker: "run".into(),
            confidence: Confidence::High,
        };
        let got = idx.find_callee_for(&target).expect("name fallback");
        assert_eq!(got.id, callee.id);
    }

    trait WithFilePath: Sized {
        fn with_file_path(self, file: &str) -> Self;
    }
    impl WithFilePath for DomainRange {
        fn with_file_path(mut self, file: &str) -> Self {
            self.file = file.to_string();
            self
        }
    }
}
