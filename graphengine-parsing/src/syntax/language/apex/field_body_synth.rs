//! R41 + R39 — synthesize Function nodes for Apex field-initializer and
//! property-accessor bodies so the heuristic resolver can attribute
//! enclosed call sites to a real caller.
//!
//! # Why this module exists
//!
//! `heuristic_resolver::find_enclosing_function` walks all `Function`
//! nodes indexed by file and returns the smallest one whose range
//! contains a call-site. The Apex extractor historically produced
//! `Function` nodes only for `method_declaration` and
//! `constructor_declaration`. Two well-known classes of call sites
//! therefore had no enclosing Function and were silently dropped
//! before any dispatch arm ran:
//!
//! - **R41 — field initializer expressions.** A field like
//!   `private Map<Id, List<Foo>> cache = new Map<Id, List<Foo>>{ opp.Id => getFoo(opp) };`
//!   lives at class scope. `getFoo(opp)` is captured by the
//!   `call_sites` query but has no enclosing Function in the graph.
//! - **R39 — property accessor bodies.** A property like
//!   `public Map<Id, Account> accountById { get { if (...) { loadAccountByIdMap(); } ... } set; }`
//!   in Apex is parsed as a `field_declaration` carrying an
//!   `accessor_list`. Calls inside the `get { ... }` / `set { ... }`
//!   block are similarly orphaned.
//!
//! # Synthesis shape (mirrors the existing `__trigger__` convention)
//!
//! For each orphan body we synthesize exactly one `Function` node
//! whose range covers the body:
//!
//! - R41 field initializer body:
//!     - FQN: `<path>::<ClassDotted>::<fieldName>.__init__()`
//!     - Range: the `value:` expression of the `variable_declarator`.
//!     - Properties: `synthetic=true`, `synthetic_kind="apex_field_initializer"`, `parent_class_id=<class_id>`, `field_name=<fieldName>`.
//! - R39 property accessor body:
//!     - FQN: `<path>::<ClassDotted>::<propName>.__get__()` / `.__set__()`
//!     - Range: the `body:` block of the `accessor_declaration`.
//!     - Properties: `synthetic=true`, `synthetic_kind="apex_property_get"` / `"apex_property_set"`, `parent_class_id=<class_id>`, `property_name=<propName>`.
//!
//! The FQN keeps the field / property name as a dotted prefix of the
//! method-segment (`<field>.__init__`) so `compose_method_fqn`'s
//! existing two-segment shape keeps working and no separator convention
//! is broken. `.__init__` / `.__get__` / `.__set__` cannot collide
//! with any legal Apex identifier because double-underscore-bracketed
//! names cannot be declared in user Apex.
//!
//! # Parent edge
//!
//! The symbol_extractor wires a `Contains` edge from the owning
//! `class_declaration`'s Struct node to each synthesized Function.
//! That edge is emitted by the caller (`symbol_extractor.rs`) by
//! iterating the returned `Vec<DomainNode>`; this module's only job
//! is to produce correctly-shaped nodes.

use tree_sitter::Node;

use crate::domain::{Confidence, Node as DomainNode, NodeKind, Provenance, ProvenanceSource};
use crate::syntax::language::apex::fqn as apex_fqn;
use crate::syntax::utils::node_converter::node_to_range;

/// Walk the class body of `class_decl` and synthesize one Function
/// node per orphan body shape found:
///
/// - R41: `field_declaration > variable_declarator` with a `value:`
///   initializer and NO sibling `accessor_list` (real field, not a
///   property).
/// - R39: `field_declaration > accessor_list > accessor_declaration`
///   with a non-null `body:` block.
///
/// Returns an empty vec for non-`class_declaration` nodes, classes with
/// no fields, or fields whose initializers / accessor bodies contain
/// nothing worth owning. The caller (`ApexExtractor::synthesize_symbol_siblings`)
/// stitches each returned node into the graph with a `Contains` edge
/// from the class's Struct node.
///
/// Why `class_declaration`-only: triggers use their own `__trigger__`
/// synthesis path, interfaces have no concrete bodies, enums cannot
/// carry field initializers in Apex. Restricting to classes keeps the
/// per-symbol cost to one kind comparison for every non-class symbol.
pub fn synthesize_field_body_functions(
    class_decl: &Node,
    source: &[u8],
    parent_class: &DomainNode,
    file_path: &str,
    workspace_root: Option<&str>,
) -> Vec<DomainNode> {
    if class_decl.kind() != "class_declaration" {
        return Vec::new();
    }
    let Some(body) = class_decl.child_by_field_name("body") else {
        return Vec::new();
    };

    let mut synthesized: Vec<DomainNode> = Vec::new();
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
        if member.kind() != "field_declaration" {
            continue;
        }
        process_field_declaration(
            &member,
            class_decl,
            source,
            parent_class,
            file_path,
            workspace_root,
            &mut synthesized,
        );
    }
    synthesized
}

fn process_field_declaration(
    field_decl: &Node,
    class_decl: &Node,
    source: &[u8],
    parent_class: &DomainNode,
    file_path: &str,
    workspace_root: Option<&str>,
    out: &mut Vec<DomainNode>,
) {
    let accessor_list = find_accessor_list(field_decl);
    let field_name = primary_declarator_name(field_decl, source);

    // R39 path: property with accessor_list. An Apex property has the
    // shape `<type> <name> { get ...; set ...; }`; the grammar still
    // parses it as a `field_declaration` whose `variable_declarator`
    // has no `value:` and whose children include an `accessor_list`.
    if let (Some(accessors), Some(name)) = (accessor_list, field_name.as_deref()) {
        let mut walker = accessors.walk();
        for accessor in accessors.named_children(&mut walker) {
            if accessor.kind() != "accessor_declaration" {
                continue;
            }
            if let Some(node) = synthesize_accessor_body(
                &accessor,
                class_decl,
                source,
                parent_class,
                name,
                file_path,
                workspace_root,
            ) {
                out.push(node);
            }
        }
        return;
    }

    // R41 path: non-property field(s) with value-initializer(s).
    let mut walker = field_decl.walk();
    for decl in field_decl.named_children(&mut walker) {
        if decl.kind() != "variable_declarator" {
            continue;
        }
        let Some(name_node) = decl.child_by_field_name("name") else {
            continue;
        };
        let Ok(name) = name_node.utf8_text(source) else {
            continue;
        };
        let Some(value) = decl.child_by_field_name("value") else {
            continue;
        };
        if !expression_may_contain_call(&value) {
            continue;
        }
        out.push(build_initializer_node(
            &value,
            class_decl,
            source,
            parent_class,
            name,
            file_path,
            workspace_root,
        ));
    }
}

fn synthesize_accessor_body(
    accessor: &Node,
    class_decl: &Node,
    source: &[u8],
    parent_class: &DomainNode,
    property_name: &str,
    file_path: &str,
    workspace_root: Option<&str>,
) -> Option<DomainNode> {
    let body = accessor.child_by_field_name("body")?;
    let kind = accessor_kind(accessor, source)?;
    let marker = match kind {
        AccessorKind::Get => "__get__",
        AccessorKind::Set => "__set__",
    };
    let synthetic_kind = match kind {
        AccessorKind::Get => "apex_property_get",
        AccessorKind::Set => "apex_property_set",
    };

    let body_range = node_to_range(&body, file_path);
    let fqn = apex_fqn::build_field_body_fqn(
        class_decl,
        source,
        property_name,
        marker,
        file_path,
        workspace_root,
    );
    let provenance = Provenance::new(ProvenanceSource::TreeSitter, Confidence::High);
    let mut node = match body.utf8_text(source).ok() {
        Some(text) => DomainNode::with_body(
            NodeKind::Function,
            fqn,
            body_range,
            provenance,
            text,
            Some("apex"),
        ),
        None => DomainNode::new(NodeKind::Function, fqn, body_range, provenance),
    };
    node.set_property("synthetic", true);
    node.set_property("synthetic_kind", synthetic_kind);
    node.set_property("parent_class_id", parent_class.id.clone());
    node.set_property("property_name", property_name.to_string());
    Some(node)
}

fn build_initializer_node(
    value: &Node,
    class_decl: &Node,
    source: &[u8],
    parent_class: &DomainNode,
    field_name: &str,
    file_path: &str,
    workspace_root: Option<&str>,
) -> DomainNode {
    let range = node_to_range(value, file_path);
    let fqn = apex_fqn::build_field_body_fqn(
        class_decl,
        source,
        field_name,
        "__init__",
        file_path,
        workspace_root,
    );
    let provenance = Provenance::new(ProvenanceSource::TreeSitter, Confidence::High);
    let mut node = match value.utf8_text(source).ok() {
        Some(text) => DomainNode::with_body(
            NodeKind::Function,
            fqn,
            range,
            provenance,
            text,
            Some("apex"),
        ),
        None => DomainNode::new(NodeKind::Function, fqn, range, provenance),
    };
    node.set_property("synthetic", true);
    node.set_property("synthetic_kind", "apex_field_initializer");
    node.set_property("parent_class_id", parent_class.id.clone());
    node.set_property("field_name", field_name.to_string());
    node
}

// ---------------------------------------------------------------------------
// Tree walk helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum AccessorKind {
    Get,
    Set,
}

fn accessor_kind(accessor: &Node, source: &[u8]) -> Option<AccessorKind> {
    // The grammar exposes the `get` / `set` token via the `accessor:`
    // field (unnamed token, so we read its text rather than its kind
    // to stay agnostic to tree-sitter-sfapex node-kind churn).
    if let Some(token) = accessor.child_by_field_name("accessor") {
        if let Ok(text) = token.utf8_text(source) {
            return match text.trim() {
                "get" => Some(AccessorKind::Get),
                "set" => Some(AccessorKind::Set),
                _ => None,
            };
        }
    }
    let mut walker = accessor.walk();
    for child in accessor.children(&mut walker) {
        if child.is_named() {
            continue;
        }
        if let Ok(text) = child.utf8_text(source) {
            match text.trim() {
                "get" => return Some(AccessorKind::Get),
                "set" => return Some(AccessorKind::Set),
                _ => {}
            }
        }
    }
    None
}

// Manual `Iterator::find` is required here — the tree-sitter TreeCursor
// returned by `walk()` would be dropped before the `Node` we return,
// and `named_children` borrows it.
#[allow(clippy::manual_find)]
fn find_accessor_list<'a>(field_decl: &Node<'a>) -> Option<Node<'a>> {
    let mut walker = field_decl.walk();
    for child in field_decl.named_children(&mut walker) {
        if child.kind() == "accessor_list" {
            return Some(child);
        }
    }
    None
}

fn primary_declarator_name(field_decl: &Node, source: &[u8]) -> Option<String> {
    let mut walker = field_decl.walk();
    for child in field_decl.named_children(&mut walker) {
        if child.kind() != "variable_declarator" {
            continue;
        }
        if let Some(name_node) = child.child_by_field_name("name") {
            if let Ok(text) = name_node.utf8_text(source) {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// Cheap structural pre-filter: does the initializer expression contain
/// any node that could generate a `Call` edge? Emitting a synthetic
/// function for a pure-literal initializer (`private Integer N = 5;`,
/// `private static final String S = 'hello';`) is wasteful and pollutes
/// `no_callers_high_confidence` with never-populated Function nodes.
///
/// We treat `method_invocation` and `object_creation_expression` as
/// call-producers. Any call-site-generating subtree anywhere under the
/// initializer causes synthesis. Anything else is skipped.
fn expression_may_contain_call(expr: &Node) -> bool {
    match expr.kind() {
        "method_invocation" | "object_creation_expression" => return true,
        _ => {}
    }
    let mut cursor = expr.walk();
    if !cursor.goto_first_child() {
        return false;
    }
    loop {
        let child = cursor.node();
        if expression_may_contain_call(&child) {
            return true;
        }
        if !cursor.goto_next_sibling() {
            return false;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse(src: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(tree_sitter_sfapex_vendored::apex::language())
            .expect("load apex grammar");
        parser.parse(src, None).expect("parse ok")
    }

    fn first_class(tree: &tree_sitter::Tree) -> tree_sitter::Node<'_> {
        fn find<'a>(n: tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
            if n.kind() == "class_declaration" {
                return Some(n);
            }
            let mut w = n.walk();
            for c in n.named_children(&mut w) {
                if let Some(hit) = find(c) {
                    return Some(hit);
                }
            }
            None
        }
        find(tree.root_node()).expect("class_declaration")
    }

    fn fake_parent() -> DomainNode {
        DomainNode::new(
            NodeKind::Struct,
            "parent::Foo".into(),
            crate::domain::Range::with_file(1, 0, 1, 0, "/ws/Foo.cls"),
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        )
    }

    #[test]
    fn r41_map_literal_initializer_with_method_call_emits_one_function() {
        let src = r#"
            public class Foo {
                private Map<Id, Integer> cache = new Map<Id, Integer>{
                    Id.valueOf('x') => getCount('x')
                };
                public Integer getCount(String s) { return 1; }
            }
        "#;
        let tree = parse(src);
        let class_decl = first_class(&tree);
        let parent = fake_parent();
        let nodes = synthesize_field_body_functions(
            &class_decl,
            src.as_bytes(),
            &parent,
            "/ws/Foo.cls",
            Some("/ws"),
        );
        assert_eq!(
            nodes.len(),
            1,
            "expected one synthetic __init__ for the cache field"
        );
        assert_eq!(nodes[0].kind, NodeKind::Function);
        assert!(
            nodes[0].fqn.ends_with("::Foo::cache.__init__()"),
            "got FQN {}",
            nodes[0].fqn
        );
        assert_eq!(
            nodes[0]
                .properties
                .get("synthetic_kind")
                .and_then(|v| v.as_str()),
            Some("apex_field_initializer")
        );
    }

    #[test]
    fn r41_literal_only_field_is_not_synthesized() {
        let src = r#"
            public class Foo {
                private Integer n = 5;
                private String s = 'hello';
            }
        "#;
        let tree = parse(src);
        let class_decl = first_class(&tree);
        let parent = fake_parent();
        let nodes = synthesize_field_body_functions(
            &class_decl,
            src.as_bytes(),
            &parent,
            "/ws/Foo.cls",
            Some("/ws"),
        );
        assert!(
            nodes.is_empty(),
            "literal-only initializers must not synthesize Function nodes, got {}",
            nodes.len()
        );
    }

    #[test]
    fn r39_property_getter_body_emits_synthetic_get_function() {
        let src = r#"
            public class Foo {
                public Integer cached { get { return compute(); } set; }
                public Integer compute() { return 42; }
            }
        "#;
        let tree = parse(src);
        let class_decl = first_class(&tree);
        let parent = fake_parent();
        let nodes = synthesize_field_body_functions(
            &class_decl,
            src.as_bytes(),
            &parent,
            "/ws/Foo.cls",
            Some("/ws"),
        );
        // Only the `get { ... }` has a body; `set;` is auto-implemented.
        assert_eq!(nodes.len(), 1, "expected one accessor function");
        assert_eq!(nodes[0].kind, NodeKind::Function);
        assert!(
            nodes[0].fqn.ends_with("::Foo::cached.__get__()"),
            "got FQN {}",
            nodes[0].fqn
        );
        assert_eq!(
            nodes[0]
                .properties
                .get("synthetic_kind")
                .and_then(|v| v.as_str()),
            Some("apex_property_get")
        );
    }

    #[test]
    fn r39_property_setter_body_emits_synthetic_set_function() {
        let src = r#"
            public class Foo {
                public Integer cached {
                    get;
                    set {
                        invalidate();
                    }
                }
                public void invalidate() {}
            }
        "#;
        let tree = parse(src);
        let class_decl = first_class(&tree);
        let parent = fake_parent();
        let nodes = synthesize_field_body_functions(
            &class_decl,
            src.as_bytes(),
            &parent,
            "/ws/Foo.cls",
            Some("/ws"),
        );
        assert_eq!(nodes.len(), 1);
        assert!(
            nodes[0].fqn.ends_with("::Foo::cached.__set__()"),
            "got FQN {}",
            nodes[0].fqn
        );
        assert_eq!(
            nodes[0]
                .properties
                .get("synthetic_kind")
                .and_then(|v| v.as_str()),
            Some("apex_property_set")
        );
    }

    #[test]
    fn inner_class_field_initializer_uses_dotted_outer_fqn() {
        let src = r#"
            public class Outer {
                public class Inner {
                    private Map<Id, Integer> cache = new Map<Id, Integer>{ Id.valueOf('x') => go('x') };
                    public Integer go(String s) { return 1; }
                }
            }
        "#;
        let tree = parse(src);
        // Walk to the Inner class_declaration specifically.
        fn find_named<'a>(
            node: tree_sitter::Node<'a>,
            name: &str,
            src: &[u8],
        ) -> Option<tree_sitter::Node<'a>> {
            if node.kind() == "class_declaration" {
                if let Some(n) = node.child_by_field_name("name") {
                    if n.utf8_text(src).ok() == Some(name) {
                        return Some(node);
                    }
                }
            }
            let mut w = node.walk();
            for c in node.named_children(&mut w) {
                if let Some(hit) = find_named(c, name, src) {
                    return Some(hit);
                }
            }
            None
        }
        let inner = find_named(tree.root_node(), "Inner", src.as_bytes()).expect("inner");
        let parent = fake_parent();
        let nodes = synthesize_field_body_functions(
            &inner,
            src.as_bytes(),
            &parent,
            "/ws/Outer.cls",
            Some("/ws"),
        );
        assert_eq!(nodes.len(), 1);
        assert!(
            nodes[0].fqn.ends_with("::Outer.Inner::cache.__init__()"),
            "inner-class FQN must carry dotted outer path: {}",
            nodes[0].fqn
        );
    }
}
