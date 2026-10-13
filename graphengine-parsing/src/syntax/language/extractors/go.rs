//! Go-specific [`LanguageSpecificExtractor`] implementation.

use tree_sitter::Node;

use crate::syntax::language::extractor::{binary_operator_text, LanguageSpecificExtractor};
use crate::syntax::utils::fqn_builder::build_fqn;

#[derive(Debug, Default)]
pub struct GoExtractor;

impl LanguageSpecificExtractor for GoExtractor {
    fn language(&self) -> &str {
        "go"
    }

    /// Methods are named `Receiver.Method` so two types in one file can both
    /// have `Provision` without colliding into a single FQN.
    fn build_symbol_fqn(
        &self,
        node: &Node,
        source: &[u8],
        simple_name: &str,
        file_path: &str,
        workspace_root: Option<&str>,
    ) -> Option<String> {
        if node.kind() != "method_declaration" {
            return None;
        }
        let receiver = receiver_type_name(node, source)?;
        Some(build_fqn(
            &format!("{receiver}.{simple_name}"),
            file_path,
            workspace_root,
        ))
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

/// `T` for receivers `(t T)`, `(t *T)`, `(t T[K])` and `(t *T[K])`.
fn receiver_type_name(method: &Node, source: &[u8]) -> Option<String> {
    let list = method.child_by_field_name("receiver")?;
    let mut cursor = list.walk();
    let param = list
        .named_children(&mut cursor)
        .find(|c| c.kind() == "parameter_declaration")?;
    let mut ty = param.child_by_field_name("type")?;
    loop {
        match ty.kind() {
            "pointer_type" => ty = ty.named_child(0)?,
            "generic_type" => ty = ty.child_by_field_name("type")?,
            "type_identifier" => return ty.utf8_text(source).ok().map(str::to_string),
            _ => return None,
        }
    }
}
