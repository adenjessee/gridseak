//! C#-specific [`LanguageSpecificExtractor`] implementation.

use tree_sitter::Node;

use crate::syntax::language::extractor::{binary_operator_text, LanguageSpecificExtractor};
use crate::syntax::utils::csharp_test_detector;

#[derive(Debug, Default)]
pub struct CSharpExtractor;

impl LanguageSpecificExtractor for CSharpExtractor {
    fn language(&self) -> &str {
        "csharp"
    }

    fn is_function_definition(&self, kind: &str) -> bool {
        matches!(
            kind,
            "method_declaration"
                | "constructor_declaration"
                | "lambda_expression"
                | "local_function_statement"
                | "anonymous_method_expression"
        )
    }

    fn is_cyclomatic_decision_point(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "foreach_statement"
                | "while_statement"
                | "do_statement"
                | "switch_section"
                | "catch_clause"
                | "conditional_expression"
        )
    }

    fn is_cognitive_structural(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "foreach_statement"
                | "while_statement"
                | "do_statement"
                | "switch_statement"
                | "switch_expression"
                | "try_statement"
                | "using_statement"
                | "catch_clause"
                | "conditional_expression"
        )
    }

    fn is_flow_break(&self, kind: &str) -> bool {
        matches!(
            kind,
            "break_statement" | "continue_statement" | "throw_statement" | "goto_statement"
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

    fn is_test_symbol(&self, node: &Node, source: &[u8]) -> bool {
        csharp_test_detector::has_csharp_test_attribute(node, source)
    }

    fn is_trait_object_type(&self, type_string: &str) -> bool {
        type_string.contains("virtual") || type_string.contains("interface")
    }
}
