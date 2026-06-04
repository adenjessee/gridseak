//! Apex-specific fully-qualified name composition.
//!
//! Apex (being Java-like) permits two declarations that the generic
//! `path + simple_name` FQN builder cannot distinguish:
//!
//! 1. **Inner-class methods with sibling collisions.** Two inner classes
//!    inside the same top-level class can declare methods with the same
//!    name (e.g. `GeocodingServiceTest.MockSuccess.respond` and
//!    `GeocodingServiceTest.MockFailure.respond`). Without the enclosing
//!    type path, both collapse to `path::GeocodingServiceTest::respond`.
//! 2. **Overloaded methods and constructors.** Apex allows method /
//!    constructor overloading by parameter types. `MetadataTriggerHandler()`
//!    and `MetadataTriggerHandler(MetadataTriggerService)` must receive
//!    distinct FQNs or call resolution loses the ability to pick the
//!    right target.
//!
//! This module mirrors Java/jorje FQN conventions so that when the LSP
//! path produces a `definition` for a symbol, its FQN can be compared to
//! the heuristic-built FQN without reconciliation work.
//!
//! ## FQN shape
//!
//! - Type declaration (class / interface / enum / trigger):
//!   `<workspace_path>::<Outer>.<Inner>`
//! - Method / constructor declaration:
//!   `<workspace_path>::<Outer>.<Inner>::<method_name>(<Type1>,<Type2>,...)`
//!
//! The `<workspace_path>` prefix is produced by the shared
//! [`build_simple_fqn`] so non-Apex languages continue to use the same
//! path encoding. Only the trailing `::<name>` component differs.
//!
//! Parameter types are written as *erased* canonical forms: type
//! arguments are dropped (`List<String>` → `List`), dimensions are
//! preserved (`String[]` → `String[]`), and all whitespace is stripped.
//! This matches Apex's JVM-like overload-resolution rules (generics are
//! not part of the method signature for overloading purposes).

use tree_sitter::Node;

use crate::syntax::utils::fqn_builder::build_simple_fqn;

/// Build an Apex FQN for a type declaration (class / interface / enum /
/// trigger). The `simple_name` is the declared name as it appears in
/// source; the enclosing-type walk is performed from `node`'s parent.
pub fn build_type_fqn(
    node: &Node,
    source: &[u8],
    simple_name: &str,
    file_path: &str,
    workspace_root: Option<&str>,
) -> String {
    let mut enclosing = enclosing_type_path(node, source);
    enclosing.push(simple_name.to_string());
    compose_type_fqn(file_path, workspace_root, &enclosing)
}

/// Build an Apex FQN for the synthetic `__trigger__` function that
/// represents the top-level statement block of a trigger body.
///
/// Shape: `<workspace_path>::<TriggerName>::__trigger__()`.
///
/// The synthetic function exists so call sites inside a trigger body
/// have a real caller node (Function) to attach `Call` edges to — Apex
/// triggers otherwise have no enclosing method and their top-level
/// statements would never produce Call edges. The empty `()` signature
/// mirrors Java/jorje's nullary-method FQN shape so the LSP definition
/// path can reconcile with heuristic output.
pub fn build_trigger_body_fqn(
    trigger_node: &Node,
    source: &[u8],
    file_path: &str,
    workspace_root: Option<&str>,
) -> String {
    let trigger_name = declaration_name(trigger_node, source).unwrap_or_default();
    compose_method_fqn(
        file_path,
        workspace_root,
        std::slice::from_ref(&trigger_name),
        "__trigger__",
        &[],
    )
}

/// Build an Apex FQN for a synthetic R41 field-initializer body or R39
/// property-accessor body.
///
/// Shape: `<path>::<Outer>.<Inner>::<member>.<marker>()`
///
/// - `class_decl` is the enclosing `class_declaration` node; the
///   enclosing-type walk is performed from its parent to pick up any
///   outer classes.
/// - `class_name` is the declared name of `class_decl` itself (read
///   out by the caller who already had it for the Struct node).
/// - `member_name` is the field or property name.
/// - `marker` is one of `__init__`, `__get__`, `__set__`.
///
/// The member-name and marker are composed into a single
/// `<member>.<marker>` simple-name segment so the 2-segment
/// `<path>::<type>::<method>()` shape used by `__trigger__` and every
/// other Apex method FQN stays intact. `.<marker>` uses the
/// double-underscore-bracketed convention (`__init__` / `__get__` /
/// `__set__`) which cannot collide with any legal Apex identifier, so
/// the composed simple name is guaranteed unique relative to any
/// user-authored method or constructor in the same class.
pub fn build_field_body_fqn(
    class_decl: &Node,
    source: &[u8],
    member_name: &str,
    marker: &str,
    file_path: &str,
    workspace_root: Option<&str>,
) -> String {
    let mut enclosing = enclosing_type_path(class_decl, source);
    if let Some(name) = declaration_name(class_decl, source) {
        enclosing.push(name);
    }
    let simple_name = format!("{member_name}.{marker}");
    compose_method_fqn(file_path, workspace_root, &enclosing, &simple_name, &[])
}

/// Build an Apex FQN for the Visualforce-page container node (TR-A.5).
///
/// Shape: `<workspace_path>::<PageName>`.
///
/// Mirrors the Struct-level FQN of a `trigger_declaration` so that the
/// page container lives at the same stratum of the graph as
/// triggers/classes. The `PageName` is derived from the source filename
/// by [`super::vf_page_reader::page_name_from_path`] (which is
/// case-preserving: `UTIL_JobProgress.page` -> `UTIL_JobProgress`).
///
/// This function takes `page_name` by reference rather than walking an
/// AST because `.page` files have no tree-sitter tree.
pub fn build_vf_page_container_fqn(
    page_name: &str,
    file_path: &str,
    workspace_root: Option<&str>,
) -> String {
    compose_type_fqn(
        file_path,
        workspace_root,
        std::slice::from_ref(&page_name.to_string()),
    )
}

/// Build an Apex FQN for the synthetic `__vf_page__` function that
/// represents the attribute-binding call surface of a Visualforce page
/// (TR-A.5).
///
/// Shape: `<workspace_path>::<PageName>::__vf_page__()`.
///
/// Mirrors [`build_trigger_body_fqn`]. The plan text in
/// `PHASE_A_EXECUTION_PLAN.md` §7.2 item 4 describes this node with two
/// forms that disagree — a literal FQN `<repo_path>::__vf_page__::<PageName>`
/// and an instruction to "mirror the existing `__trigger__` convention".
/// We follow the mirror instruction because (a) the trigger pattern is
/// load-bearing: Phase C TR-C.3 will extend this node with additional
/// VF call surfaces (rerender, actionFunction, text-node expressions)
/// which need a stable container per PageName; (b) the literal form
/// reverses the container/method order relative to every other Apex
/// synthetic node, which would break every downstream query written
/// against the trigger shape; (c) the container Struct is emitted at
/// `<path>::<PageName>` so the `__vf_page__` function reports up to
/// the same container-name hierarchy every other Apex node uses.
pub fn build_vf_page_body_fqn(
    page_name: &str,
    file_path: &str,
    workspace_root: Option<&str>,
) -> String {
    compose_method_fqn(
        file_path,
        workspace_root,
        std::slice::from_ref(&page_name.to_string()),
        "__vf_page__",
        &[],
    )
}

/// Build an Apex FQN for a method or constructor declaration.
///
/// The enclosing-type walk produces the `<Outer>.<Inner>` dotted path
/// of the enclosing type(s); `simple_name` is the method/constructor
/// name; parameter types are extracted from the `formal_parameters`
/// field of the function node.
pub fn build_method_fqn(
    func_node: &Node,
    source: &[u8],
    simple_name: &str,
    file_path: &str,
    workspace_root: Option<&str>,
) -> String {
    let enclosing = enclosing_type_path(func_node, source);
    let params = parameter_types(func_node, source);
    compose_method_fqn(file_path, workspace_root, &enclosing, simple_name, &params)
}

// -------------------------------------------------------------------------
// Composition helpers
// -------------------------------------------------------------------------

fn compose_type_fqn(
    file_path: &str,
    workspace_root: Option<&str>,
    enclosing_including_self: &[String],
) -> String {
    let prefix = path_prefix(file_path, workspace_root);
    if enclosing_including_self.is_empty() {
        // Defensive: falling back to the shared builder keeps the graph
        // well-formed even if the AST walk produces no type names.
        return prefix;
    }
    let tail = enclosing_including_self.join(".");
    if prefix.is_empty() {
        tail
    } else {
        format!("{prefix}::{tail}")
    }
}

fn compose_method_fqn(
    file_path: &str,
    workspace_root: Option<&str>,
    enclosing: &[String],
    simple_name: &str,
    params: &[String],
) -> String {
    let prefix = path_prefix(file_path, workspace_root);
    let sig = format!("{simple_name}({})", params.join(","));
    match (prefix.is_empty(), enclosing.is_empty()) {
        (true, true) => sig,
        (false, true) => format!("{prefix}::{sig}"),
        (true, false) => format!("{}::{sig}", enclosing.join(".")),
        (false, false) => format!("{prefix}::{}::{sig}", enclosing.join(".")),
    }
}

/// Compute the path-only prefix by running the shared FQN builder with
/// an empty name and stripping its trailing separator.
fn path_prefix(file_path: &str, workspace_root: Option<&str>) -> String {
    let mut prefix = build_simple_fqn("", file_path, workspace_root);
    while prefix.ends_with("::") {
        prefix.truncate(prefix.len() - 2);
    }
    prefix
}

// -------------------------------------------------------------------------
// AST walkers
// -------------------------------------------------------------------------

/// Walk up from `node` collecting the names of all enclosing type
/// declarations (class / interface / enum / trigger), outermost first.
/// The `node` itself is never included.
fn enclosing_type_path(node: &Node, source: &[u8]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut cursor = node.parent();
    while let Some(parent) = cursor {
        if is_type_declaration(parent.kind()) {
            if let Some(name) = declaration_name(&parent, source) {
                names.push(name);
            }
        }
        cursor = parent.parent();
    }
    names.reverse();
    names
}

fn is_type_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration" | "interface_declaration" | "enum_declaration" | "trigger_declaration"
    )
}

fn declaration_name(node: &Node, source: &[u8]) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    name_node.utf8_text(source).ok().map(|s| s.to_string())
}

/// Extract an ordered, canonical list of parameter type strings from a
/// `method_declaration` or `constructor_declaration` node.
fn parameter_types(func_node: &Node, source: &[u8]) -> Vec<String> {
    let Some(params) = func_node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut walker = params.walk();
    for child in params.named_children(&mut walker) {
        if child.kind() != "formal_parameter" {
            continue;
        }
        if let Some(ty) = child.child_by_field_name("type") {
            out.push(canonical_type(&ty, source));
        }
    }
    out
}

/// Canonical textual form of a type node for signature purposes.
///
/// - `generic_type`: drop `type_arguments`, recurse on the underlying
///   `type_identifier` / `scoped_type_identifier` so `List<String>`
///   becomes `List`.
/// - `array_type`: recurse on the `element` field, append `[]`.
/// - everything else: raw node text with all whitespace stripped.
fn canonical_type(node: &Node, source: &[u8]) -> String {
    match node.kind() {
        "generic_type" => {
            let mut walker = node.walk();
            for child in node.named_children(&mut walker) {
                if child.kind() != "type_arguments" {
                    return canonical_type(&child, source);
                }
            }
            compact_text(node, source)
        }
        "array_type" => {
            if let Some(element) = node.child_by_field_name("element") {
                return format!("{}[]", canonical_type(&element, source));
            }
            compact_text(node, source)
        }
        _ => compact_text(node, source),
    }
}

fn compact_text(node: &Node, source: &[u8]) -> String {
    node.utf8_text(source)
        .ok()
        .map(|s| s.split_whitespace().collect::<String>())
        .unwrap_or_default()
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse(source: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(tree_sitter_sfapex_vendored::apex::language())
            .expect("load apex grammar");
        parser.parse(source, None).expect("parse ok")
    }

    fn find_first<'a>(node: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut walker = node.walk();
        for child in node.named_children(&mut walker) {
            if let Some(hit) = find_first(child, kind) {
                return Some(hit);
            }
        }
        None
    }

    fn find_by_name<'a>(
        node: tree_sitter::Node<'a>,
        kind: &str,
        name: &str,
        source: &[u8],
    ) -> Option<tree_sitter::Node<'a>> {
        if node.kind() == kind {
            if let Some(n) = node.child_by_field_name("name") {
                if n.utf8_text(source).ok() == Some(name) {
                    return Some(node);
                }
            }
        }
        let mut walker = node.walk();
        for child in node.named_children(&mut walker) {
            if let Some(hit) = find_by_name(child, kind, name, source) {
                return Some(hit);
            }
        }
        None
    }

    #[test]
    fn top_level_class_fqn_matches_path_prefix_plus_name() {
        let src = "public class Foo { }";
        let tree = parse(src);
        let class_node = find_first(tree.root_node(), "class_declaration").unwrap();
        let fqn = build_type_fqn(
            &class_node,
            src.as_bytes(),
            "Foo",
            "/ws/force-app/Foo.cls",
            Some("/ws"),
        );
        assert!(fqn.ends_with("::Foo"), "got {fqn}");
    }

    #[test]
    fn inner_class_gains_outer_dotted_prefix() {
        let src = "public class Outer { public class Inner { } }";
        let tree = parse(src);
        let inner = find_by_name(
            tree.root_node(),
            "class_declaration",
            "Inner",
            src.as_bytes(),
        )
        .expect("inner class");
        let fqn = build_type_fqn(
            &inner,
            src.as_bytes(),
            "Inner",
            "/ws/force-app/Outer.cls",
            Some("/ws"),
        );
        assert!(
            fqn.ends_with("::Outer.Inner"),
            "inner-class FQN must use dotted outer path: {fqn}"
        );
    }

    #[test]
    fn overloaded_constructors_get_distinct_signatures() {
        let src = r#"
            public class Handler {
                public Handler() {}
                public Handler(Integer x) {}
                public Handler(Integer x, String y) {}
            }
        "#;
        let tree = parse(src);
        let bytes = src.as_bytes();
        let class_node = find_first(tree.root_node(), "class_declaration").unwrap();
        let mut walker = class_node.walk();
        let mut sigs = Vec::new();
        let body = class_node.child_by_field_name("body").unwrap();
        for child in body.named_children(&mut walker) {
            if child.kind() == "constructor_declaration" {
                let fqn = build_method_fqn(
                    &child,
                    bytes,
                    "Handler",
                    "/ws/force-app/Handler.cls",
                    Some("/ws"),
                );
                sigs.push(fqn);
            }
        }
        assert_eq!(sigs.len(), 3, "three constructors expected");
        let mut dedup = sigs.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(
            dedup.len(),
            3,
            "constructor FQNs must be distinct: {sigs:?}"
        );
        assert!(sigs.iter().any(|s| s.ends_with("::Handler::Handler()")));
        assert!(sigs
            .iter()
            .any(|s| s.ends_with("::Handler::Handler(Integer)")));
        assert!(sigs
            .iter()
            .any(|s| s.ends_with("::Handler::Handler(Integer,String)")));
    }

    #[test]
    fn generic_and_array_parameters_are_erased_to_base_type() {
        let src = r#"
            public class Svc {
                public void doWork(List<String> ids, Map<Id, Account> m, String[] names) {}
            }
        "#;
        let tree = parse(src);
        let bytes = src.as_bytes();
        let method = find_by_name(tree.root_node(), "method_declaration", "doWork", bytes).unwrap();
        let fqn = build_method_fqn(
            &method,
            bytes,
            "doWork",
            "/ws/force-app/Svc.cls",
            Some("/ws"),
        );
        assert!(
            fqn.ends_with("::Svc::doWork(List,Map,String[])"),
            "generics must be erased, arrays kept: {fqn}"
        );
    }

    #[test]
    fn sibling_inner_methods_share_simple_name_but_diverge_on_outer_path() {
        let src = r#"
            public class Outer {
                public class A {
                    public void respond() {}
                }
                public class B {
                    public void respond() {}
                }
            }
        "#;
        let tree = parse(src);
        let bytes = src.as_bytes();
        let mut respond_fqns = Vec::new();
        fn collect<'a>(
            node: tree_sitter::Node<'a>,
            bytes: &'a [u8],
            out: &mut Vec<String>,
            file_path: &str,
            ws: Option<&str>,
        ) {
            if node.kind() == "method_declaration" {
                if let Some(n) = node.child_by_field_name("name") {
                    if n.utf8_text(bytes).ok() == Some("respond") {
                        out.push(build_method_fqn(&node, bytes, "respond", file_path, ws));
                    }
                }
            }
            let mut walker = node.walk();
            for child in node.named_children(&mut walker) {
                collect(child, bytes, out, file_path, ws);
            }
        }
        collect(
            tree.root_node(),
            bytes,
            &mut respond_fqns,
            "/ws/Outer.cls",
            Some("/ws"),
        );
        assert_eq!(respond_fqns.len(), 2);
        assert!(respond_fqns[0] != respond_fqns[1], "got {respond_fqns:?}");
        assert!(respond_fqns
            .iter()
            .any(|f| f.ends_with("::Outer.A::respond()")));
        assert!(respond_fqns
            .iter()
            .any(|f| f.ends_with("::Outer.B::respond()")));
    }

    #[test]
    fn method_with_no_params_gets_empty_parentheses() {
        let src = "public class S { public void go() {} }";
        let tree = parse(src);
        let bytes = src.as_bytes();
        let method = find_by_name(tree.root_node(), "method_declaration", "go", bytes).unwrap();
        let fqn = build_method_fqn(&method, bytes, "go", "/ws/S.cls", Some("/ws"));
        assert!(fqn.ends_with("::S::go()"), "got {fqn}");
    }

    #[test]
    fn trigger_body_fqn_encodes_trigger_name_and_nullary_signature() {
        let src = r#"
            trigger AccountTrigger on Account (before insert, after update) {
                System.debug('hi');
            }
        "#;
        let tree = parse(src);
        let trigger = find_first(tree.root_node(), "trigger_declaration").unwrap();
        let fqn = build_trigger_body_fqn(
            &trigger,
            src.as_bytes(),
            "/ws/force-app/AccountTrigger.trigger",
            Some("/ws"),
        );
        assert!(
            fqn.ends_with("::AccountTrigger::__trigger__()"),
            "got {fqn}"
        );
    }

    #[test]
    fn trigger_body_fqn_survives_missing_workspace_root() {
        let src = r#"trigger LeadT on Lead (before insert) { Integer x = 1; }"#;
        let tree = parse(src);
        let trigger = find_first(tree.root_node(), "trigger_declaration").unwrap();
        let fqn = build_trigger_body_fqn(&trigger, src.as_bytes(), "LeadT.trigger", None);
        assert!(fqn.ends_with("::LeadT::__trigger__()"), "got {fqn}");
    }

    #[test]
    fn scoped_parameter_type_is_preserved_verbatim() {
        let src = r#"
            public class B implements Database.Batchable {
                public void start(Database.BatchableContext ctx) {}
            }
        "#;
        let tree = parse(src);
        let bytes = src.as_bytes();
        let method = find_by_name(tree.root_node(), "method_declaration", "start", bytes).unwrap();
        let fqn = build_method_fqn(&method, bytes, "start", "/ws/B.cls", Some("/ws"));
        assert!(
            fqn.ends_with("::B::start(Database.BatchableContext)"),
            "scoped type must retain dotted form: {fqn}"
        );
    }
}
