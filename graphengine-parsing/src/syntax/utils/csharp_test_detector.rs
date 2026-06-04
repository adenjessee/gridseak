//! Detects C# test attributes ([Fact], [Theory], [Test], [TestMethod], etc.) on AST nodes.
//!
//! Tier 3 detection: identifies test methods annotated with xUnit, NUnit, or MSTest markers.

const TEST_ATTRIBUTE_NAMES: &[&str] = &[
    // xUnit
    "Fact",
    "Theory",
    "InlineData",
    // NUnit
    "Test",
    "TestCase",
    "TestCaseSource",
    "TestFixture",
    "SetUp",
    "TearDown",
    "OneTimeSetUp",
    "OneTimeTearDown",
    // MSTest
    "TestMethod",
    "TestClass",
    "TestInitialize",
    "TestCleanup",
    "ClassInitialize",
    "ClassCleanup",
    "DataTestMethod",
];

/// Check whether a tree-sitter node (method_declaration or class_declaration) has a
/// test attribute among its `attribute_list` children.
///
/// In tree-sitter-c-sharp, attributes are `attribute_list` children containing
/// `attribute` nodes with a `name` field.
pub fn has_csharp_test_attribute(node: &tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_list" {
            let mut attr_cursor = child.walk();
            for attr in child.children(&mut attr_cursor) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        if let Ok(text) = name_node.utf8_text(source) {
                            let name = text.rsplit('.').next().unwrap_or(text);
                            let bare = name.strip_suffix("Attribute").unwrap_or(name);
                            if TEST_ATTRIBUTE_NAMES.contains(&bare) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }

    // Also check preceding sibling attribute_lists (some structures attach them this way)
    let mut sibling = node.prev_named_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "attribute_list" {
            let mut attr_cursor = sib.walk();
            for attr in sib.children(&mut attr_cursor) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        if let Ok(text) = name_node.utf8_text(source) {
                            let name = text.rsplit('.').next().unwrap_or(text);
                            let bare = name.strip_suffix("Attribute").unwrap_or(name);
                            if TEST_ATTRIBUTE_NAMES.contains(&bare) {
                                return true;
                            }
                        }
                    }
                }
            }
        } else {
            break;
        }
        sibling = sib.prev_named_sibling();
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attribute_names_include_xunit() {
        assert!(TEST_ATTRIBUTE_NAMES.contains(&"Fact"));
        assert!(TEST_ATTRIBUTE_NAMES.contains(&"Theory"));
    }

    #[test]
    fn test_attribute_names_include_nunit() {
        assert!(TEST_ATTRIBUTE_NAMES.contains(&"Test"));
        assert!(TEST_ATTRIBUTE_NAMES.contains(&"TestCase"));
        assert!(TEST_ATTRIBUTE_NAMES.contains(&"TestFixture"));
    }

    #[test]
    fn test_attribute_names_include_mstest() {
        assert!(TEST_ATTRIBUTE_NAMES.contains(&"TestMethod"));
        assert!(TEST_ATTRIBUTE_NAMES.contains(&"TestClass"));
    }
}
