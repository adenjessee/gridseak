//! Rust-specific [`LanguageSpecificExtractor`] implementation.
//!
//! Behaviour is identical to the pre-refactor `match "rust" => ...` arms in
//! `complexity_extractor`, `symbol_extractor`, `trait_context_detector`, and
//! `receiver_detector`. See those files' git history for the source.

use tree_sitter::Node;

use crate::syntax::language::extractor::{binary_operator_text, LanguageSpecificExtractor};
use crate::syntax::utils::fqn_builder::build_fqn;
use crate::syntax::utils::rust_test_detector;

/// Inline `mod` names and the `impl` type / `trait` name enclosing `node`,
/// outermost first. A function nested in another function's body keeps only
/// the owners outside that body's item (`fn` bodies are not path segments).
fn rust_owner_path(node: &Node, source: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = node.parent();
    while let Some(n) = cur {
        match n.kind() {
            "impl_item" => {
                if let Some(t) = n
                    .child_by_field_name("type")
                    .and_then(|t| type_name(&t, source))
                {
                    out.push(t);
                }
            }
            "trait_item" | "mod_item" => {
                if let Some(name) = n
                    .child_by_field_name("name")
                    .and_then(|t| t.utf8_text(source).ok())
                {
                    out.push(name.to_string());
                }
            }
            _ => {}
        }
        cur = n.parent();
    }
    out.reverse();
    out
}

/// `Circle` for `Circle`, `Circle<T>`, `&'a Circle`, `crate::m::Circle<T>`;
/// the full text (without spaces) for tuples, slices and other shapes.
fn type_name(ty: &Node, source: &[u8]) -> Option<String> {
    match ty.kind() {
        "type_identifier" | "primitive_type" => ty.utf8_text(source).ok().map(str::to_string),
        "generic_type" => type_name(&ty.child_by_field_name("type")?, source),
        "scoped_type_identifier" => type_name(&ty.child_by_field_name("name")?, source),
        "reference_type" | "pointer_type" => type_name(&ty.child_by_field_name("type")?, source),
        _ => ty
            .utf8_text(source)
            .ok()
            .map(|s| s.chars().filter(|c| !c.is_whitespace()).collect()),
    }
}

#[derive(Debug, Default)]
pub struct RustExtractor;

impl LanguageSpecificExtractor for RustExtractor {
    fn language(&self) -> &str {
        "rust"
    }

    /// Functions are named by the inline modules and the `impl`/`trait`
    /// block around them: `file_mod::tests::helper`, `file_mod::Circle::area`,
    /// `file_mod::Area::twice`. Without the owner, `Circle::is_empty` and
    /// `Square::is_empty` in one file shared an FQN (and, with equal bodies,
    /// one node ID), and every hotspot read `mod::is_empty`.
    fn build_symbol_fqn(
        &self,
        node: &Node,
        source: &[u8],
        simple_name: &str,
        file_path: &str,
        workspace_root: Option<&str>,
    ) -> Option<String> {
        if !matches!(node.kind(), "function_item" | "function_signature_item") {
            return None;
        }
        let owners = rust_owner_path(node, source);
        if owners.is_empty() {
            return None;
        }
        Some(build_fqn(
            &format!("{}::{simple_name}", owners.join("::")),
            file_path,
            workspace_root,
        ))
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
