//! Symbol lookup utility functions
//!
//! Provides utilities for finding symbols (functions, modules, etc.) in syntax results.
//! Two API tiers:
//!
//! 1. **Indexed** (preferred): Accept a `&SymbolIndex` for O(file_syms) containment queries.
//! 2. **Legacy**: Accept `&SyntaxResults` and scan linearly. Kept for backward-compat in tests.

use crate::application::ports::SyntaxResults;
use crate::domain::{Node, NodeKind, Range};
use crate::infrastructure::lsp::utils::range_utils::is_within_range;
use crate::symbol_index::SymbolIndex;

// ---------------------------------------------------------------
// Indexed API (O(file_fns) / O(file_mods) per lookup)
// ---------------------------------------------------------------

/// Find the innermost function containing a range, using a prebuilt index.
pub fn find_containing_function_indexed(location: &Range, index: &SymbolIndex) -> Option<String> {
    index
        .find_containing_function(location)
        .map(|rec| rec.id.clone())
}

/// Find the module containing a range, using a prebuilt index.
pub fn find_containing_module_indexed(location: &Range, index: &SymbolIndex) -> Option<String> {
    index
        .find_containing_module(location)
        .map(|rec| rec.id.clone())
}

/// Find the most specific symbol at a location, using a prebuilt index.
pub fn find_symbol_at_location_indexed<'a>(
    location: &Range,
    index: &'a SymbolIndex,
) -> Option<&'a crate::symbol_index::SymbolRecord> {
    index.find_symbol_at_location(location)
}

// ---------------------------------------------------------------
// Legacy API (linear scan — kept for tests and call sites that
// don't yet have an index)
// ---------------------------------------------------------------

pub fn find_containing_function(
    call_site: &Range,
    syntax_results: &SyntaxResults,
) -> Option<String> {
    for symbol in &syntax_results.symbols {
        if symbol.kind == NodeKind::Function && is_within_range(call_site, &symbol.location) {
            return Some(symbol.id.clone());
        }
    }
    None
}

pub fn find_containing_module(range: &Range, syntax_results: &SyntaxResults) -> Option<String> {
    let mut modules: Vec<&Node> = syntax_results
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == NodeKind::Module && symbol.location.file == range.file)
        .collect();

    if let Some(module) = modules
        .iter()
        .copied()
        .find(|module| is_within_range(range, &module.location))
    {
        return Some(module.id.clone());
    }

    modules.sort_by_key(|module| (module.location.start_line, module.location.start_char));
    modules.first().map(|module| module.id.clone())
}

pub fn find_symbol_at_location<'a>(
    location: &Range,
    syntax_results: &'a SyntaxResults,
) -> Option<&'a Node> {
    let mut best: Option<&Node> = None;
    let mut best_span: Option<(u32, u32)> = None;

    for symbol in &syntax_results.symbols {
        if symbol.location.file != location.file {
            continue;
        }
        if !is_within_range(location, &symbol.location) {
            continue;
        }

        let line_span = symbol
            .location
            .end_line
            .saturating_sub(symbol.location.start_line);
        let char_span = symbol
            .location
            .end_char
            .saturating_sub(symbol.location.start_char);

        let span = (line_span, char_span);
        match (best_span, best) {
            (None, _) => {
                best = Some(symbol);
                best_span = Some(span);
            }
            (Some(cur), Some(cur_sym)) => {
                let better = span < cur
                    || (span == cur
                        && cur_sym.kind == NodeKind::Module
                        && symbol.kind != NodeKind::Module);
                if better {
                    best = Some(symbol);
                    best_span = Some(span);
                }
            }
            _ => {}
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Node, Provenance, ProvenanceSource};

    #[test]
    fn test_find_containing_function() {
        let mut syntax = SyntaxResults::new();
        let func_range = Range::with_file(1, 0, 10, 0, "test.rs".to_string());
        let call_range = Range::with_file(5, 0, 5, 10, "test.rs".to_string());

        syntax.symbols.push(Node {
            id: "func1".to_string(),
            kind: NodeKind::Function,
            fqn: "test::func1".to_string(),
            location: func_range,
            provenance: Provenance::new(
                ProvenanceSource::TreeSitter,
                crate::domain::Confidence::High,
            ),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        });

        assert_eq!(
            find_containing_function(&call_range, &syntax),
            Some("func1".to_string())
        );

        // Indexed version should give same result
        let index = SymbolIndex::from_syntax(&syntax);
        assert_eq!(
            find_containing_function_indexed(&call_range, &index),
            Some("func1".to_string())
        );
    }

    #[test]
    fn test_find_containing_module() {
        let mut syntax = SyntaxResults::new();
        let module_range = Range::with_file(1, 0, 20, 0, "test.rs".to_string());
        let item_range = Range::with_file(10, 0, 10, 10, "test.rs".to_string());

        syntax.symbols.push(Node {
            id: "mod1".to_string(),
            kind: NodeKind::Module,
            fqn: "test::mod1".to_string(),
            location: module_range,
            provenance: Provenance::new(
                ProvenanceSource::TreeSitter,
                crate::domain::Confidence::High,
            ),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        });

        assert_eq!(
            find_containing_module(&item_range, &syntax),
            Some("mod1".to_string())
        );

        let index = SymbolIndex::from_syntax(&syntax);
        assert_eq!(
            find_containing_module_indexed(&item_range, &index),
            Some("mod1".to_string())
        );
    }

    #[test]
    fn test_find_symbol_at_location_prefers_specific_symbol_over_file_scoped_module() {
        let mut syntax = SyntaxResults::new();
        let file = "test.ts".to_string();

        syntax.symbols.push(Node {
            id: "mod_file".to_string(),
            kind: NodeKind::Module,
            fqn: "file_module".to_string(),
            location: Range::with_file(1, 0, 100, 0, file.clone()),
            provenance: Provenance::new(
                ProvenanceSource::TreeSitter,
                crate::domain::Confidence::Low,
            ),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        });

        syntax.symbols.push(Node {
            id: "fn".to_string(),
            kind: NodeKind::Function,
            fqn: "fn".to_string(),
            location: Range::with_file(10, 0, 10, 10, file.clone()),
            provenance: Provenance::new(
                ProvenanceSource::TreeSitter,
                crate::domain::Confidence::High,
            ),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        });

        let loc = Range::with_file(10, 5, 10, 6, file.clone());
        let sym = find_symbol_at_location(&loc, &syntax).expect("symbol");
        assert_eq!(sym.id, "fn");
        assert_eq!(sym.kind, NodeKind::Function);
    }
}
