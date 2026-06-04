//! Go-specific [`LanguageSpecificExtractor`] implementation.

use tree_sitter::Node;

use crate::syntax::language::extractor::{binary_operator_text, LanguageSpecificExtractor};

#[derive(Debug, Default)]
pub struct GoExtractor;

impl LanguageSpecificExtractor for GoExtractor {
    fn language(&self) -> &str {
        "go"
    }

    fn is_function_definition(&self, kind: &str) -> bool {
        matches!(
            kind,
            "function_declaration" | "method_declaration" | "func_literal"
        )
    }

    fn is_cyclomatic_decision_point(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "expression_case"
                | "type_case"
                | "communication_case"
        )
    }

    fn is_cognitive_structural(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "expression_switch_statement"
                | "type_switch_statement"
                | "select_statement"
        )
    }

    fn is_flow_break(&self, kind: &str) -> bool {
        matches!(
            kind,
            "break_statement" | "continue_statement" | "goto_statement"
        )
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
        if node.kind() != "if_statement" {
            return false;
        }
        let parent_is_if = node
            .parent()
            .map(|p| p.kind() == "if_statement")
            .unwrap_or(false);
        if !parent_is_if {
            return false;
        }
        node.prev_named_sibling()
            .map(|s| s.kind() == "block")
            .unwrap_or(false)
    }

    fn is_trait_object_type(&self, type_string: &str) -> bool {
        type_string.contains("interface")
    }
}
