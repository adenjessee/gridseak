//! Apex class-level sharing-modifier classification.
//!
//! Apex enforces row-level security via the `with sharing` /
//! `without sharing` / `inherited sharing` modifiers on top-level
//! classes. From the Apex Developer Guide and the Salesforce
//! Well-Architected security pillar:
//!
//! - **`with sharing`** — the class enforces the running user's record
//!   sharing rules. Safe default for any class invoked from a UI
//!   context.
//! - **`without sharing`** — the class explicitly bypasses sharing
//!   rules. Required for certain administrative use cases (batch jobs,
//!   schedulable cleanups), but also a common source of CRUD/FLS
//!   privilege-escalation bugs when used in the wrong place.
//! - **`inherited sharing`** — the class adopts the sharing context of
//!   its caller. Salesforce recommends this for utility/service
//!   classes so behaviour is contextual and predictable.
//! - **`omitted`** — when a top-level class declares **no** sharing
//!   modifier, Apex silently defaults to `without sharing` when the
//!   entry point is `Schedulable`/`Batchable`/REST/etc. Salesforce
//!   security guidance treats this as a smell because the security
//!   posture is implicit and easily missed in code review.
//!
//! Inner classes: the sharing modifier on an inner class is usually
//! absent (the Apex compiler historically forbade it and, in practice,
//! most code relies on the outer-class modifier). Sprint E.5 changes
//! the inner-class behaviour from "return `None`" to "inherit the
//! outer class's modifier", which matches how the Apex runtime
//! actually evaluates row-level security on inner-class code paths.
//! If an inner class *does* declare its own modifier (newer grammars
//! tolerate it), the inner's own modifier wins over the outer's.
//!
//! # Output contract
//!
//! [`classify`] returns:
//!
//! - `Some(SharingModel)` for any `class_declaration`:
//!     * top-level class: its own modifier, or `Omitted` if none.
//!     * inner class: its own modifier if declared, otherwise the
//!       enclosing top-level class's modifier, otherwise `Omitted`.
//! - `None` for interfaces, enums, or any other node kind. Returning
//!   `None` on non-classes keeps the property table free of noise on
//!   nodes where the modifier is meaningless.

use tree_sitter::Node;

/// Classified Apex sharing model. Lowercase snake-case wire form keeps
/// the JSON property stable across runs and platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SharingModel {
    /// `with sharing` — enforces the running user's row-level security.
    WithSharing,
    /// `without sharing` — explicitly bypasses row-level security.
    WithoutSharing,
    /// `inherited sharing` — adopts the caller's sharing context.
    InheritedSharing,
    /// Top-level class with no sharing modifier declared. Apex defaults
    /// to `without sharing` in many entry-point scenarios when this is
    /// the case, which is a Salesforce security finding.
    Omitted,
}

impl SharingModel {
    /// Stable wire-format string used as the JSON property value.
    pub fn as_str(self) -> &'static str {
        match self {
            SharingModel::WithSharing => "with_sharing",
            SharingModel::WithoutSharing => "without_sharing",
            SharingModel::InheritedSharing => "inherited_sharing",
            SharingModel::Omitted => "omitted",
        }
    }
}

/// Classify the sharing model of a class node. Returns `None` for
/// non-class nodes (interfaces, enums, etc.).
///
/// For inner classes, the rule is:
/// 1. If the inner class declares its own sharing modifier, use it.
/// 2. Otherwise, walk up to the nearest enclosing `class_declaration`
///    and use that outer class's modifier.
/// 3. If neither the inner nor any ancestor declares a modifier,
///    emit `Omitted` — downstream security tooling already treats
///    `omitted` as a reviewable finding.
pub fn classify(node: &Node, source: &[u8]) -> Option<SharingModel> {
    if node.kind() != "class_declaration" {
        return None;
    }
    if let Some(own) = read_sharing_modifier(node, source) {
        return Some(own);
    }
    if let Some(outer_modifier) = outer_sharing_modifier(node, source) {
        return Some(outer_modifier);
    }
    Some(SharingModel::Omitted)
}

/// Walk up from `node` looking for an enclosing `class_declaration`
/// that declares a sharing modifier. Used exclusively by inner-class
/// inheritance — a top-level class's own `classify` call never
/// reaches this function because it reads its own modifier first.
fn outer_sharing_modifier(node: &Node, source: &[u8]) -> Option<SharingModel> {
    let mut cursor = node.parent();
    while let Some(parent) = cursor {
        if parent.kind() == "class_declaration" {
            if let Some(modifier) = read_sharing_modifier(&parent, source) {
                return Some(modifier);
            }
        }
        cursor = parent.parent();
    }
    None
}

/// Walk the `modifiers` child of a class declaration and return the
/// first sharing modifier found. Apex compilers reject conflicting
/// sharing modifiers, so we don't have to disambiguate.
fn read_sharing_modifier(node: &Node, _source: &[u8]) -> Option<SharingModel> {
    let modifiers = node.child_by_field_name("modifiers").or_else(|| {
        for i in 0..node.child_count() {
            if let Some(c) = node.child(i) {
                if c.kind() == "modifiers" {
                    return Some(c);
                }
            }
        }
        None
    })?;

    walk_for_sharing(&modifiers)
}

fn walk_for_sharing(node: &Node) -> Option<SharingModel> {
    for i in 0..node.named_child_count() {
        let child = node.named_child(i)?;
        match child.kind() {
            "with_sharing" => return Some(SharingModel::WithSharing),
            "without_sharing" => return Some(SharingModel::WithoutSharing),
            "inherited_sharing" => return Some(SharingModel::InheritedSharing),
            // `modifier` wraps the actual token in some grammar revisions —
            // descend one level so we don't miss a wrapped sharing token.
            "modifier" => {
                if let Some(found) = walk_for_sharing(&child) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::{Parser, Tree};

    fn parse(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(tree_sitter_sfapex_vendored::apex::language())
            .expect("set apex grammar");
        parser.parse(source, None).expect("parse apex")
    }

    fn first_class<'a>(tree: &'a Tree) -> Node<'a> {
        let root = tree.root_node();
        find_class(root).expect("source must contain a class declaration")
    }

    fn find_class<'a>(node: Node<'a>) -> Option<Node<'a>> {
        if node.kind() == "class_declaration" {
            return Some(node);
        }
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i) {
                if let Some(found) = find_class(child) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn detects_with_sharing() {
        let src = "public with sharing class Demo {}";
        let tree = parse(src);
        let class = first_class(&tree);
        assert_eq!(
            classify(&class, src.as_bytes()),
            Some(SharingModel::WithSharing)
        );
    }

    #[test]
    fn detects_without_sharing() {
        let src = "public without sharing class Demo {}";
        let tree = parse(src);
        let class = first_class(&tree);
        assert_eq!(
            classify(&class, src.as_bytes()),
            Some(SharingModel::WithoutSharing),
        );
    }

    #[test]
    fn detects_inherited_sharing() {
        let src = "public inherited sharing class Demo {}";
        let tree = parse(src);
        let class = first_class(&tree);
        assert_eq!(
            classify(&class, src.as_bytes()),
            Some(SharingModel::InheritedSharing),
        );
    }

    #[test]
    fn missing_modifier_yields_omitted_for_top_level_class() {
        let src = "public class Demo {}";
        let tree = parse(src);
        let class = first_class(&tree);
        assert_eq!(
            classify(&class, src.as_bytes()),
            Some(SharingModel::Omitted)
        );
    }

    #[test]
    fn inner_class_inherits_outer_with_sharing() {
        let src = r#"
            public with sharing class Outer {
                public class Inner {
                    public void run() {}
                }
            }
        "#;
        let tree = parse(src);
        let outer = first_class(&tree);
        fn find_named_class<'a>(node: Node<'a>, want: &str, source: &[u8]) -> Option<Node<'a>> {
            if node.kind() == "class_declaration" {
                for i in 0..node.named_child_count() {
                    let c = node.named_child(i)?;
                    if c.kind() == "identifier" {
                        if let Ok(text) = c.utf8_text(source) {
                            if text == want {
                                return Some(node);
                            }
                        }
                    }
                }
            }
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    if let Some(found) = find_named_class(child, want, source) {
                        return Some(found);
                    }
                }
            }
            None
        }
        let inner = find_named_class(outer, "Inner", src.as_bytes()).expect("inner class exists");
        assert_eq!(
            classify(&inner, src.as_bytes()),
            Some(SharingModel::WithSharing),
            "inner class must inherit outer `with sharing`"
        );
    }

    #[test]
    fn inner_class_inherits_outer_without_sharing() {
        let src = r#"
            public without sharing class Outer {
                public class Inner { public void run() {} }
            }
        "#;
        let tree = parse(src);
        let outer = first_class(&tree);
        fn find_named_class<'a>(node: Node<'a>, want: &str, source: &[u8]) -> Option<Node<'a>> {
            if node.kind() == "class_declaration" {
                for i in 0..node.named_child_count() {
                    let c = node.named_child(i)?;
                    if c.kind() == "identifier" {
                        if let Ok(text) = c.utf8_text(source) {
                            if text == want {
                                return Some(node);
                            }
                        }
                    }
                }
            }
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    if let Some(found) = find_named_class(child, want, source) {
                        return Some(found);
                    }
                }
            }
            None
        }
        let inner = find_named_class(outer, "Inner", src.as_bytes()).expect("inner class exists");
        assert_eq!(
            classify(&inner, src.as_bytes()),
            Some(SharingModel::WithoutSharing),
        );
    }

    #[test]
    fn inner_with_no_outer_modifier_is_omitted() {
        // Neither outer nor inner declare a modifier → Omitted is the
        // deterministic signal. Matches the security-review behaviour
        // for top-level classes so inner classes aren't silently
        // excluded from "missing modifier" reports.
        let src = "public class Outer { public class Inner {} }";
        let tree = parse(src);
        let outer = first_class(&tree);
        fn find_named_class<'a>(node: Node<'a>, want: &str, source: &[u8]) -> Option<Node<'a>> {
            if node.kind() == "class_declaration" {
                for i in 0..node.named_child_count() {
                    let c = node.named_child(i)?;
                    if c.kind() == "identifier" {
                        if let Ok(text) = c.utf8_text(source) {
                            if text == want {
                                return Some(node);
                            }
                        }
                    }
                }
            }
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    if let Some(found) = find_named_class(child, want, source) {
                        return Some(found);
                    }
                }
            }
            None
        }
        let inner = find_named_class(outer, "Inner", src.as_bytes()).unwrap();
        assert_eq!(
            classify(&inner, src.as_bytes()),
            Some(SharingModel::Omitted)
        );
    }

    #[test]
    fn case_insensitive_modifier_keywords() {
        let src = "public WITH SHARING class Demo {}";
        let tree = parse(src);
        let class = first_class(&tree);
        assert_eq!(
            classify(&class, src.as_bytes()),
            Some(SharingModel::WithSharing)
        );
    }

    #[test]
    fn modifier_alongside_annotation_still_classified() {
        let src = r#"
            @SuppressWarnings('PMD')
            public without sharing class Demo {}
        "#;
        let tree = parse(src);
        let class = first_class(&tree);
        assert_eq!(
            classify(&class, src.as_bytes()),
            Some(SharingModel::WithoutSharing),
        );
    }

    #[test]
    fn returns_none_for_non_class_node() {
        let src = "public interface Demo { void doit(); }";
        let tree = parse(src);
        let root = tree.root_node();
        // Pick the interface_declaration node — sharing classification
        // is meaningful only for classes.
        fn find_interface<'a>(node: Node<'a>) -> Option<Node<'a>> {
            if node.kind() == "interface_declaration" {
                return Some(node);
            }
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    if let Some(found) = find_interface(child) {
                        return Some(found);
                    }
                }
            }
            None
        }
        let iface = find_interface(root).expect("interface present");
        assert_eq!(classify(&iface, src.as_bytes()), None);
    }

    #[test]
    fn wire_format_strings_are_stable() {
        assert_eq!(SharingModel::WithSharing.as_str(), "with_sharing");
        assert_eq!(SharingModel::WithoutSharing.as_str(), "without_sharing");
        assert_eq!(SharingModel::InheritedSharing.as_str(), "inherited_sharing");
        assert_eq!(SharingModel::Omitted.as_str(), "omitted");
    }
}
