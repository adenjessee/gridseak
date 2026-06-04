//! Detects Rust test attributes (#[test], #[cfg(test)]) on AST nodes.
//!
//! Tier 3 detection: identifies inline test code within production files,
//! which is the standard Rust pattern for unit tests.

/// Check whether a tree-sitter node (function_item or mod_item) has a
/// `#[test]` or `#[cfg(test)]` attribute among its siblings / decorators.
///
/// In tree-sitter-rust, attributes are `attribute_item` children of the
/// same parent block, appearing *before* the annotated item.  For items
/// at the top level of a file they are sibling nodes; for items inside a
/// `declaration_list` they are also siblings within that list.
pub fn has_rust_test_attribute(node: &tree_sitter::Node, source: &[u8]) -> bool {
    // Walk preceding siblings looking for attribute_item nodes
    let mut sibling = node.prev_named_sibling();
    while let Some(sib) = sibling {
        if sib.kind() != "attribute_item" {
            break;
        }
        if let Ok(text) = sib.utf8_text(source) {
            let trimmed = text.trim();
            if is_test_attribute(trimmed) || is_cfg_test_attribute(trimmed) {
                return true;
            }
        }
        sibling = sib.prev_named_sibling();
    }

    false
}

/// Check whether a tree-sitter node is inside a `#[cfg(test)]` module.
/// Walks up the containment tree looking for a mod_item with `#[cfg(test)]`.
pub fn is_inside_cfg_test_module(node: &tree_sitter::Node, source: &[u8]) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "mod_item" && has_rust_test_attribute(&parent, source) {
            return true;
        }
        // declaration_list -> mod_item
        if parent.kind() == "declaration_list" {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "mod_item" && has_rust_test_attribute(&grandparent, source)
                {
                    return true;
                }
            }
        }
        current = parent.parent();
    }
    false
}

fn is_test_attribute(attr_text: &str) -> bool {
    // Matches: #[test], #[tokio::test], #[async_std::test]
    let inner = match attr_text
        .strip_prefix("#[")
        .and_then(|s| s.strip_suffix(']'))
    {
        Some(s) => s.trim(),
        None => return false,
    };
    inner == "test" || inner.ends_with("::test") || inner.starts_with("test(")
}

fn is_cfg_test_attribute(attr_text: &str) -> bool {
    let inner = match attr_text
        .strip_prefix("#[")
        .and_then(|s| s.strip_suffix(']'))
    {
        Some(s) => s.trim(),
        None => return false,
    };
    inner == "cfg(test)" || inner.starts_with("cfg(test,") || inner.contains("cfg(test)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_test_attribute() {
        assert!(is_test_attribute("#[test]"));
        assert!(is_test_attribute("#[tokio::test]"));
        assert!(is_test_attribute("#[async_std::test]"));
        assert!(!is_test_attribute("#[derive(Debug)]"));
        assert!(!is_test_attribute("#[allow(unused)]"));
    }

    #[test]
    fn test_is_cfg_test_attribute() {
        assert!(is_cfg_test_attribute("#[cfg(test)]"));
        assert!(!is_cfg_test_attribute("#[cfg(feature = \"foo\")]"));
        assert!(!is_cfg_test_attribute("#[test]"));
    }
}
