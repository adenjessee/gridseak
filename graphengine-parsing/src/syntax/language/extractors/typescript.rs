//! TypeScript-specific [`LanguageSpecificExtractor`] implementation.
//! Also used for JavaScript — the complexity and receiver patterns are
//! identical because they share the tree-sitter grammar family.

use tree_sitter::Node;

use crate::syntax::language::extractor::{binary_operator_text, LanguageSpecificExtractor};

#[derive(Debug, Default)]
pub struct TypeScriptExtractor;

impl LanguageSpecificExtractor for TypeScriptExtractor {
    fn language(&self) -> &str {
        "typescript"
    }

    fn is_function_definition(&self, kind: &str) -> bool {
        matches!(
            kind,
            "function_declaration"
                | "method_definition"
                | "arrow_function"
                | "function_expression"
                | "generator_function_declaration"
                | "generator_function"
        )
    }

    fn is_cyclomatic_decision_point(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "for_in_statement"
                | "while_statement"
                | "do_statement"
                | "switch_case"
                | "catch_clause"
                | "ternary_expression"
                | "optional_chain_expression"
        )
    }

    fn is_cognitive_structural(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "for_in_statement"
                | "while_statement"
                | "do_statement"
                | "switch_statement"
                | "try_statement"
                | "catch_clause"
                | "ternary_expression"
        )
    }

    fn is_flow_break(&self, kind: &str) -> bool {
        matches!(
            kind,
            "break_statement" | "continue_statement" | "throw_statement"
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
        node.parent()
            .map(|p| p.kind() == "else_clause")
            .unwrap_or(false)
    }

    fn is_trait_object_type(&self, type_string: &str) -> bool {
        type_string.contains("interface")
            || (type_string.contains(':') && type_string.contains("=>"))
    }
}
