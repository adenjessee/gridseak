//! Detects Apex test markers on class and method AST nodes.
//!
//! Apex has two independent ways to mark a symbol as a test:
//!
//! 1. **`@IsTest` annotation** — the modern idiom. Case-insensitive
//!    (`@IsTest`, `@isTest`). Applied to a class, every method in that class
//!    is a test; applied to a method, only that method is a test.
//! 2. **`testMethod` keyword** — the legacy idiom. A per-method modifier
//!    keyword (like `static`, `public`, …). Applied only to methods.
//!
//! Detection tiers:
//! - Direct: the node has `@IsTest` or `testMethod` in its own `modifiers`.
//! - Inherited: an enclosing `class_declaration` has `@IsTest` in its
//!   modifiers — every method and nested class inside such a class is
//!   treated as test. Inner-class inheritance (Sprint E.5) extends the
//!   walk beyond methods so the inner class's *own struct node* picks
//!   up `is_test = true`.
//!
//! Reference: <https://developer.salesforce.com/docs/atlas.en-us.apexcode.meta/apexcode/apex_testing_intro.htm>

use tree_sitter::Node;

/// Returns true if this Apex class/method node is a test.
///
/// The node is expected to be a `class_declaration`, `interface_declaration`,
/// `enum_declaration`, `method_declaration`, or `constructor_declaration`.
pub fn is_apex_test(node: &Node, source: &[u8]) -> bool {
    // Tier 1: direct marker on this node.
    if has_direct_test_marker(node, source) {
        return true;
    }

    // Tier 2: inherit from an enclosing `@IsTest` class. Applies to
    // methods, constructors, AND nested class/interface/enum
    // declarations — a test class's inner `class MockCallout` is
    // also test code and must receive the same tagging as its
    // enclosing methods. Restricting to declaration-kind nodes keeps
    // the walk tight and prevents stray AST nodes (expressions,
    // statements) from being misclassified.
    if matches!(
        node.kind(),
        "method_declaration"
            | "constructor_declaration"
            | "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
    ) && is_inside_istest_class(node, source)
    {
        return true;
    }

    false
}

/// True if the node has `@IsTest` / `@isTest` / `testMethod` among its own
/// immediate modifiers.
fn has_direct_test_marker(node: &Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" && modifiers_contain_test_marker(&child, source) {
            return true;
        }
    }
    false
}

/// Walk up parents looking for a `class_declaration` annotated with `@IsTest`.
/// Nested classes inherit from the outermost `@IsTest` ancestor.
fn is_inside_istest_class(node: &Node, source: &[u8]) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "class_declaration" {
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if child.kind() == "modifiers"
                    && modifiers_contain_istest_annotation(&child, source)
                {
                    return true;
                }
            }
        }
        current = parent.parent();
    }
    false
}

/// True if the `modifiers` node contains either `@IsTest` (any case) or the
/// `testMethod` keyword modifier.
fn modifiers_contain_test_marker(modifiers: &Node, source: &[u8]) -> bool {
    let mut cursor = modifiers.walk();
    for modifier in modifiers.children(&mut cursor) {
        match modifier.kind() {
            "annotation" => {
                if annotation_name_is_istest(&modifier, source) {
                    return true;
                }
            }
            "modifier" => {
                if let Ok(text) = modifier.utf8_text(source) {
                    if text.eq_ignore_ascii_case("testmethod") {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// True if a `modifiers` node contains the `@IsTest` annotation (any case).
fn modifiers_contain_istest_annotation(modifiers: &Node, source: &[u8]) -> bool {
    let mut cursor = modifiers.walk();
    for modifier in modifiers.children(&mut cursor) {
        if modifier.kind() == "annotation" && annotation_name_is_istest(&modifier, source) {
            return true;
        }
    }
    false
}

/// True if this `annotation` node names `@IsTest` (case-insensitive, dot-tail
/// match so namespaced forms like `@my.IsTest` still trip on the short name).
fn annotation_name_is_istest(annotation: &Node, source: &[u8]) -> bool {
    if let Some(name_node) = annotation.child_by_field_name("name") {
        if let Ok(text) = name_node.utf8_text(source) {
            let short = text.rsplit('.').next().unwrap_or(text);
            return short.eq_ignore_ascii_case("IsTest");
        }
    }
    // Fallback: the grammar may expose the annotation name as a non-named
    // field — search immediate children for the first identifier.
    let mut cursor = annotation.walk();
    for child in annotation.children(&mut cursor) {
        if child.kind() == "identifier" {
            if let Ok(text) = child.utf8_text(source) {
                return text.eq_ignore_ascii_case("IsTest");
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::{Parser, Tree};

    fn parse(src: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(tree_sitter_sfapex_vendored::apex::language())
            .unwrap();
        parser.parse(src, None).unwrap()
    }

    fn find_first(node: Node<'_>, kind: &str, name: &str, source: &[u8]) -> Option<Node<'static>> {
        // SAFETY: Node is borrowed from the tree. This helper is only used
        // inside individual tests where the tree outlives the resulting node.
        fn visit<'a>(node: Node<'a>, kind: &str, name: &str, source: &[u8]) -> Option<Node<'a>> {
            if node.kind() == kind {
                if let Some(n) = node.child_by_field_name("name") {
                    if n.utf8_text(source).ok() == Some(name) {
                        return Some(node);
                    }
                }
            }
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if let Some(found) = visit(child, kind, name, source) {
                        return Some(found);
                    }
                }
            }
            None
        }
        // Lifetime laundering: callers in these tests all keep the Tree alive.
        let result = visit(node, kind, name, source);
        result.map(|n| unsafe { std::mem::transmute::<Node<'_>, Node<'static>>(n) })
    }

    #[test]
    fn class_with_istest_annotation_is_test() {
        let src = r#"
@IsTest
public class MyTests {
    static testMethod void first() {}
    @IsTest static void second() {}
    public static void helper() {}
}
"#;
        let tree = parse(src);
        let root = tree.root_node();
        let class = find_first(root, "class_declaration", "MyTests", src.as_bytes()).unwrap();
        assert!(is_apex_test(&class, src.as_bytes()), "class should be test");

        // Every method inside an @IsTest class is a test, including `helper`.
        for name in ["first", "second", "helper"] {
            let method = find_first(root, "method_declaration", name, src.as_bytes()).unwrap();
            assert!(
                is_apex_test(&method, src.as_bytes()),
                "method {name} should be test (inherits from @IsTest class)"
            );
        }
    }

    #[test]
    fn method_with_istest_annotation_is_test() {
        let src = r#"
public class Mixed {
    @IsTest static void testOne() {}
    public static void notATest() {}
}
"#;
        let tree = parse(src);
        let root = tree.root_node();
        let class = find_first(root, "class_declaration", "Mixed", src.as_bytes()).unwrap();
        assert!(!is_apex_test(&class, src.as_bytes()), "class not annotated");

        let test_method =
            find_first(root, "method_declaration", "testOne", src.as_bytes()).unwrap();
        assert!(is_apex_test(&test_method, src.as_bytes()));

        let non_test = find_first(root, "method_declaration", "notATest", src.as_bytes()).unwrap();
        assert!(!is_apex_test(&non_test, src.as_bytes()));
    }

    #[test]
    fn method_with_testmethod_keyword_is_test() {
        let src = r#"
public class Legacy {
    static testMethod void oldStyle() {}
    public static void helper() {}
}
"#;
        let tree = parse(src);
        let root = tree.root_node();
        let test_method =
            find_first(root, "method_declaration", "oldStyle", src.as_bytes()).unwrap();
        assert!(is_apex_test(&test_method, src.as_bytes()));

        let helper = find_first(root, "method_declaration", "helper", src.as_bytes()).unwrap();
        assert!(!is_apex_test(&helper, src.as_bytes()));
    }

    #[test]
    fn istest_is_case_insensitive() {
        for variant in ["@IsTest", "@isTest", "@ISTEST", "@istest"] {
            let src = format!("{variant}\npublic class X {{ static void f() {{}} }}\n");
            let tree = parse(&src);
            let root = tree.root_node();
            let class = find_first(root, "class_declaration", "X", src.as_bytes()).unwrap();
            assert!(
                is_apex_test(&class, src.as_bytes()),
                "class with {variant} should be test"
            );
        }
    }

    #[test]
    fn inner_class_inside_istest_class_is_also_test() {
        // Sprint E.5: the inner-class *struct node* itself must now
        // carry the test tag, not just its methods. This guarantees
        // downstream filters like `WHERE properties->>'is_test' = 'true'`
        // catch mock/helper nested classes that live inside @IsTest
        // outer classes.
        let src = r#"
@IsTest
public class GeocodingServiceTest {
    public class MockSuccess {
        public void respond() {}
    }
}
"#;
        let tree = parse(src);
        let root = tree.root_node();
        let outer = find_first(
            root,
            "class_declaration",
            "GeocodingServiceTest",
            src.as_bytes(),
        )
        .unwrap();
        assert!(is_apex_test(&outer, src.as_bytes()));

        let inner = find_first(root, "class_declaration", "MockSuccess", src.as_bytes()).unwrap();
        assert!(
            is_apex_test(&inner, src.as_bytes()),
            "inner class inside @IsTest outer must be test-tagged"
        );
    }

    #[test]
    fn inner_class_in_non_istest_outer_is_not_test() {
        // Regression: inner classes in ordinary outer classes must NOT
        // pick up `is_test`. Guards against an overly broad Tier-2
        // walk rule.
        let src = r#"
public class Plain {
    public class Nested { }
}
"#;
        let tree = parse(src);
        let root = tree.root_node();
        let inner = find_first(root, "class_declaration", "Nested", src.as_bytes()).unwrap();
        assert!(!is_apex_test(&inner, src.as_bytes()));
    }

    #[test]
    fn regular_class_is_not_test() {
        let src = r#"public class Plain { public static void f() {} }"#;
        let tree = parse(src);
        let root = tree.root_node();
        let class = find_first(root, "class_declaration", "Plain", src.as_bytes()).unwrap();
        assert!(!is_apex_test(&class, src.as_bytes()));
        let method = find_first(root, "method_declaration", "f", src.as_bytes()).unwrap();
        assert!(!is_apex_test(&method, src.as_bytes()));
    }
}
