//! Java-specific [`LanguageSpecificExtractor`] implementation.

use tree_sitter::Node;

use crate::syntax::language::extractor::{binary_operator_text, LanguageSpecificExtractor};
use crate::syntax::utils::java_test_detector;

#[derive(Debug, Default)]
pub struct JavaExtractor;

impl LanguageSpecificExtractor for JavaExtractor {
    fn language(&self) -> &str {
        "java"
    }

    fn is_function_definition(&self, kind: &str) -> bool {
        matches!(
            kind,
            "method_declaration" | "constructor_declaration" | "lambda_expression"
        )
    }

    fn is_cyclomatic_decision_point(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "enhanced_for_statement"
                | "while_statement"
                | "do_statement"
                | "switch_label"
                | "catch_clause"
                | "ternary_expression"
        )
    }

    fn is_cognitive_structural(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "enhanced_for_statement"
                | "while_statement"
                | "do_statement"
                | "switch_expression"
                | "try_statement"
                | "try_with_resources_statement"
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

    fn is_test_symbol(&self, node: &Node, source: &[u8]) -> bool {
        java_test_detector::has_java_test_annotation(node, source)
    }

    fn is_trait_object_type(&self, type_string: &str) -> bool {
        type_string.contains("interface") || type_string.ends_with("Interface")
    }
}
