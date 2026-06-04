//! Detects Java test annotations (@Test, @ParameterizedTest, etc.) on AST nodes.
//!
//! Tier 3 detection: identifies test methods annotated with JUnit/TestNG markers.

const TEST_ANNOTATION_NAMES: &[&str] = &[
    "Test",
    "ParameterizedTest",
    "RepeatedTest",
    "TestFactory",
    "TestTemplate",
    "Disabled",
    "Nested",
];

/// Check whether a tree-sitter node (method_declaration or class_declaration) has a
/// test annotation among its sibling `marker_annotation` or `annotation` nodes.
///
/// In tree-sitter-java, annotations are siblings within a `modifiers` child
/// that precedes the method/class declaration.
pub fn has_java_test_annotation(node: &tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut mod_cursor = child.walk();
            for modifier in child.children(&mut mod_cursor) {
                match modifier.kind() {
                    "marker_annotation" => {
                        if let Some(name_node) = modifier.child_by_field_name("name") {
                            if let Ok(text) = name_node.utf8_text(source) {
                                let name = text.rsplit('.').next().unwrap_or(text);
                                if TEST_ANNOTATION_NAMES.contains(&name) {
                                    return true;
                                }
                            }
                        }
                    }
                    "annotation" => {
                        if let Some(name_node) = modifier.child_by_field_name("name") {
                            if let Ok(text) = name_node.utf8_text(source) {
                                let name = text.rsplit('.').next().unwrap_or(text);
                                if TEST_ANNOTATION_NAMES.contains(&name) {
                                    return true;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Also check preceding sibling annotations (some grammars attach them this way)
    let mut sibling = node.prev_named_sibling();
    while let Some(sib) = sibling {
        match sib.kind() {
            "marker_annotation" | "annotation" => {
                if let Some(name_node) = sib.child_by_field_name("name") {
                    if let Ok(text) = name_node.utf8_text(source) {
                        let name = text.rsplit('.').next().unwrap_or(text);
                        if TEST_ANNOTATION_NAMES.contains(&name) {
                            return true;
                        }
                    }
                }
            }
            _ => break,
        }
        sibling = sib.prev_named_sibling();
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_annotation_names_include_junit5() {
        assert!(TEST_ANNOTATION_NAMES.contains(&"Test"));
        assert!(TEST_ANNOTATION_NAMES.contains(&"ParameterizedTest"));
        assert!(TEST_ANNOTATION_NAMES.contains(&"RepeatedTest"));
    }
}
