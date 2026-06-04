//! Rust-specific [`LanguageSpecificExtractor`] implementation.
//!
//! Behaviour is identical to the pre-refactor `match "rust" => ...` arms in
//! `complexity_extractor`, `symbol_extractor`, `trait_context_detector`, and
//! `receiver_detector`. See those files' git history for the source.

use tree_sitter::Node;

use crate::syntax::language::extractor::{binary_operator_text, LanguageSpecificExtractor};
use crate::syntax::utils::rust_test_detector;

#[derive(Debug, Default)]
pub struct RustExtractor;

impl LanguageSpecificExtractor for RustExtractor {
    fn language(&self) -> &str {
        "rust"
    }

    fn is_function_definition(&self, kind: &str) -> bool {
        matches!(kind, "function_item" | "closure_expression")
    }

    fn is_cyclomatic_decision_point(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_expression"
                | "for_expression"
                | "while_expression"
                | "loop_expression"
                | "match_arm"
                | "if_let_expression"
                | "while_let_expression"
                | "try_expression"
        )
    }

    fn is_cognitive_structural(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_expression"
                | "if_let_expression"
                | "for_expression"
                | "while_expression"
                | "while_let_expression"
                | "loop_expression"
                | "match_expression"
        )
    }

    fn is_flow_break(&self, kind: &str) -> bool {
        matches!(kind, "break_expression" | "continue_expression")
    }

    fn is_logical_operator_node(&self, node: &Node, source: &[u8]) -> bool {
        if node.kind() != "binary_expression" {
            return false;
        }
        binary_operator_text(node, source)
            .map(|op| op == "&&" || op == "||")
            .unwrap_or(false)
    }

    fn is_continuation_if(&self, node: &Node) -> bool {
        if node.kind() != "if_expression" {
            return false;
        }
        node.parent()
            .map(|p| p.kind() == "else_clause")
            .unwrap_or(false)
    }

    fn is_test_symbol(&self, node: &Node, source: &[u8]) -> bool {
        // Symbol-level (modules/structs/enums): attribute only. A module with
        // `#[cfg(test)]` or `#[test]` is a test module; we don't walk the
        // parent chain here because walking would incorrectly flag inner
        // non-test modules nested inside an outer test module.
        rust_test_detector::has_rust_test_attribute(node, source)
    }

    fn is_test_function(&self, node: &Node, source: &[u8]) -> bool {
        // Function-level: either the function itself has `#[test]`/`#[cfg(test)]`
        // OR it lives inside an enclosing `#[cfg(test)]` module.
        rust_test_detector::has_rust_test_attribute(node, source)
            || rust_test_detector::is_inside_cfg_test_module(node, source)
    }

    fn is_trait_object_type(&self, type_string: &str) -> bool {
        type_string.contains("&dyn")
            || type_string.contains("Box<dyn")
            || type_string.contains("Arc<dyn")
            || type_string.contains("Rc<dyn")
            || type_string.contains("dyn ")
    }
}
