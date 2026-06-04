//! Python-specific [`LanguageSpecificExtractor`] implementation.

use tree_sitter::Node;

use crate::syntax::language::extractor::LanguageSpecificExtractor;

#[derive(Debug, Default)]
pub struct PythonExtractor;

impl LanguageSpecificExtractor for PythonExtractor {
    fn language(&self) -> &str {
        "python"
    }

    fn is_function_definition(&self, kind: &str) -> bool {
        kind == "function_definition"
    }

    fn is_cyclomatic_decision_point(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "elif_clause"
                | "for_statement"
                | "while_statement"
                | "except_clause"
                | "conditional_expression"
        )
    }

    fn is_cognitive_structural(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "elif_clause"
                | "for_statement"
                | "while_statement"
                | "try_statement"
                | "except_clause"
        )
    }

    fn is_flow_break(&self, kind: &str) -> bool {
        matches!(
            kind,
            "break_statement" | "continue_statement" | "raise_statement"
        )
    }

    fn is_logical_operator_node(&self, node: &Node, _source: &[u8]) -> bool {
        node.kind() == "boolean_operator"
    }

    fn is_continuation_if(&self, node: &Node) -> bool {
        node.kind() == "elif_clause"
    }

    fn is_trait_object_type(&self, type_string: &str) -> bool {
        type_string.contains("Protocol")
            || type_string.contains("typing.Protocol")
            || type_string.contains("ABC")
    }
}
