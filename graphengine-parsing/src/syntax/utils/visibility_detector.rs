//! AST-based visibility detection for extracted symbols.
//!
//! Determines whether a function/struct/etc. is publicly exported based on
//! language-specific AST patterns:
//! - Rust: `visibility_modifier` child on the declaration node
//! - TypeScript/JS: `export_statement` ancestor wrapping the declaration
//!
//! Go and Python use naming conventions instead of AST markers; those
//! heuristics are applied separately in `treesitter.rs` after extraction.

/// Detect visibility from the AST context of a matched tree-sitter node.
///
/// Returns a visibility string suitable for `node.set_property("visibility", ...)`:
/// - `"public"` / `"pub_crate"` / `"pub_super"` / `"private"` for Rust
/// - `"exported"` / `"private"` for TypeScript/JavaScript
/// - `None` for languages handled by naming heuristics (Go, Python)
pub fn detect_visibility_from_ast(
    node: &tree_sitter::Node,
    language: &str,
    content: &[u8],
) -> Option<String> {
    match language {
        "rust" => detect_rust_visibility(node, content),
        "typescript" | "javascript" => detect_ts_js_export(node),
        "java" => detect_java_visibility(node, content),
        "csharp" => detect_csharp_visibility(node, content),
        _ => None,
    }
}

/// Rust: check for `visibility_modifier` child on the declaration node.
/// `pub fn foo()` → `visibility_modifier` text is "pub"
/// `pub(crate) fn foo()` → text is "pub(crate)"
fn detect_rust_visibility(node: &tree_sitter::Node, content: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = child.utf8_text(content).unwrap_or("pub");
            return Some(normalize_rust_visibility(text));
        }
    }
    Some("private".to_string())
}

fn normalize_rust_visibility(vis_text: &str) -> String {
    let trimmed = vis_text.trim();
    if trimmed == "pub" {
        "public".to_string()
    } else if trimmed.starts_with("pub(crate)") {
        "pub_crate".to_string()
    } else if trimmed.starts_with("pub(super)") {
        "pub_super".to_string()
    } else if trimmed.starts_with("pub(in") {
        "pub_restricted".to_string()
    } else {
        "public".to_string()
    }
}

/// TypeScript/JavaScript: walk up the tree looking for `export_statement` ancestor.
/// `export function foo() {}` → func_declaration is child of export_statement
/// `export const bar = () => {}` → lexical_declaration is child of export_statement
fn detect_ts_js_export(node: &tree_sitter::Node) -> Option<String> {
    let mut current = Some(*node);
    let mut depth = 0;

    while let Some(n) = current {
        if depth > 3 {
            break;
        }
        if n.kind() == "export_statement" {
            return Some("exported".to_string());
        }
        current = n.parent();
        depth += 1;
    }

    Some("private".to_string())
}

/// Java: check the `modifiers` child for access keywords.
/// `public void foo()` → modifiers text contains "public"
/// Default (no modifier) is package-private.
fn detect_java_visibility(node: &tree_sitter::Node, content: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = child.utf8_text(content).unwrap_or("");
            if text.contains("public") {
                return Some("public".to_string());
            } else if text.contains("protected") {
                return Some("protected".to_string());
            } else if text.contains("private") {
                return Some("private".to_string());
            }
            return Some("package".to_string());
        }
    }
    Some("package".to_string())
}

/// C#: check `modifier` children for access keywords.
/// `public void Foo()` → modifier text is "public"
/// Default (no modifier) is `private` for members, `internal` for types.
fn detect_csharp_visibility(node: &tree_sitter::Node, content: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier" {
            let text = child.utf8_text(content).unwrap_or("");
            match text {
                "public" => return Some("public".to_string()),
                "protected" => return Some("protected".to_string()),
                "private" => return Some("private".to_string()),
                "internal" => return Some("internal".to_string()),
                _ => {}
            }
        }
    }
    Some("private".to_string())
}

/// Apply Go visibility heuristic: uppercase first letter = exported.
pub fn go_visibility_from_name(name: &str) -> &'static str {
    if name
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
    {
        "public"
    } else {
        "private"
    }
}

/// Apply Python visibility heuristic: leading underscore = private.
pub fn python_visibility_from_name(name: &str) -> &'static str {
    if name.starts_with('_') {
        "private"
    } else {
        "public"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_visibility_normalization() {
        assert_eq!(normalize_rust_visibility("pub"), "public");
        assert_eq!(normalize_rust_visibility("pub(crate)"), "pub_crate");
        assert_eq!(normalize_rust_visibility("pub(super)"), "pub_super");
        assert_eq!(
            normalize_rust_visibility("pub(in crate::foo)"),
            "pub_restricted"
        );
    }

    #[test]
    fn go_exported_uppercase() {
        assert_eq!(go_visibility_from_name("Router"), "public");
        assert_eq!(go_visibility_from_name("HandleFunc"), "public");
    }

    #[test]
    fn go_unexported_lowercase() {
        assert_eq!(go_visibility_from_name("router"), "private");
        assert_eq!(go_visibility_from_name("handleFunc"), "private");
    }

    #[test]
    fn python_public_no_underscore() {
        assert_eq!(python_visibility_from_name("get_user"), "public");
        assert_eq!(python_visibility_from_name("Session"), "public");
    }

    #[test]
    fn python_private_underscore() {
        assert_eq!(python_visibility_from_name("_internal"), "private");
        assert_eq!(python_visibility_from_name("__init__"), "private");
        assert_eq!(python_visibility_from_name("__private"), "private");
    }
}
