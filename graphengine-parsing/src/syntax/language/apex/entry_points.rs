//! Apex entry-point classification.
//!
//! In Apex, a substantial fraction of public API is reachable **from outside
//! the codebase** — LWC frontends, Flow/Process Builder, external REST or
//! SOAP clients, async job queues, scheduled jobs. The graph engine can't
//! see those callers, but it *can* tag the Apex symbols that act as the
//! external-facing entry points. That tag matters for:
//!
//! - **Blast radius**: entry points are sinks, not sources. A coupling
//!   metric that treats "@AuraEnabled getAccount()" as uncalled
//!   (because nothing in the Apex repo calls it) mis-reports isolation.
//! - **Security review surface**: every @RestResource, @HttpGet,
//!   @AuraEnabled, and `global` method is a trust boundary.
//! - **Dead-code detection**: methods tagged as entry points must never
//!   be flagged as dead, regardless of in-repo call count.
//!
//! Classification tiers (any match is sufficient):
//!
//! 1. **Annotation-based** (methods or classes):
//!    `@AuraEnabled`, `@InvocableMethod`, `@HttpGet`/`Post`/`Put`/`Delete`/`Patch`,
//!    `@RestResource`, `@Future`, `@RemoteAction`.
//! 2. **Modifier-keyword-based** (methods/classes): `global`, `webservice`.
//! 3. **Interface-implementation-based** (classes only): implementing
//!    `Schedulable`, `Database.Batchable`, `Queueable`, or `Messaging.InboundEmailHandler`.

use tree_sitter::{Node, Query, QueryCursor};

/// All Apex entry-point categories the graph engine recognises.
///
/// This list is the union of every way a symbol becomes externally reachable
/// from outside the Apex codebase. Extend only when you can point to a
/// concrete Salesforce mechanism that calls it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryPointKind {
    /// `@AuraEnabled` — callable from LWC / Aura components.
    AuraEnabled,
    /// `@InvocableMethod` — callable from Flow / Process Builder.
    InvocableMethod,
    /// `@HttpGet` — HTTP GET handler in an `@RestResource` class.
    HttpGet,
    /// `@HttpPost` — HTTP POST handler.
    HttpPost,
    /// `@HttpPut` — HTTP PUT handler.
    HttpPut,
    /// `@HttpDelete` — HTTP DELETE handler.
    HttpDelete,
    /// `@HttpPatch` — HTTP PATCH handler.
    HttpPatch,
    /// `@RestResource` — class-level REST endpoint declaration.
    RestResource,
    /// `@Future` — async fire-and-forget execution.
    Future,
    /// `@RemoteAction` — legacy Visualforce remoting endpoint.
    RemoteAction,
    /// `global` visibility keyword — callable from any namespace.
    Global,
    /// `webservice` modifier — SOAP endpoint.
    Webservice,
    /// Class `implements Schedulable` — scheduled job entry point.
    Schedulable,
    /// Class `implements Database.Batchable` — batch job entry point.
    Batchable,
    /// Class `implements Queueable` — queueable job entry point.
    Queueable,
    /// Class `implements Messaging.InboundEmailHandler` — email-to-Apex.
    InboundEmailHandler,
}

impl EntryPointKind {
    /// Short, stable identifier for use in node properties, logs, and tests.
    pub fn as_str(self) -> &'static str {
        match self {
            EntryPointKind::AuraEnabled => "aura_enabled",
            EntryPointKind::InvocableMethod => "invocable_method",
            EntryPointKind::HttpGet => "http_get",
            EntryPointKind::HttpPost => "http_post",
            EntryPointKind::HttpPut => "http_put",
            EntryPointKind::HttpDelete => "http_delete",
            EntryPointKind::HttpPatch => "http_patch",
            EntryPointKind::RestResource => "rest_resource",
            EntryPointKind::Future => "future",
            EntryPointKind::RemoteAction => "remote_action",
            EntryPointKind::Global => "global",
            EntryPointKind::Webservice => "webservice",
            EntryPointKind::Schedulable => "schedulable",
            EntryPointKind::Batchable => "batchable",
            EntryPointKind::Queueable => "queueable",
            EntryPointKind::InboundEmailHandler => "inbound_email_handler",
        }
    }
}

/// Classify an Apex `class_declaration` / `method_declaration` /
/// `constructor_declaration` AST node into the set of entry-point markers
/// it carries.
///
/// Returns an empty Vec for a regular (non-entry-point) symbol. Multiple
/// markers can coexist (e.g. a `global webservice` method is both Global
/// and Webservice).
pub fn classify(node: &Node, source: &[u8]) -> Vec<EntryPointKind> {
    classify_with_annotation_query(node, source, None)
}

/// Variant of [`classify`] that accepts the YAML-defined
/// `annotations` query string so grammar shapes stay authoritative
/// in `configs/apex.yaml` rather than duplicated in Rust.
///
/// When `annotation_query` is `Some`, annotation detection runs
/// through the query; modifier and interface scanning stay on the
/// direct AST walk (those concerns have no YAML analogue). `None`
/// degrades to the fully manual walk so a missing YAML binding
/// never silently drops entry-point tagging.
pub fn classify_with_annotation_query(
    node: &Node,
    source: &[u8],
    annotation_query: Option<&str>,
) -> Vec<EntryPointKind> {
    let mut kinds = Vec::new();

    match annotation_query.and_then(compile_annotation_query) {
        Some(query) => {
            collect_annotations_via_query(node, source, &query, &mut kinds);
            collect_modifier_markers(node, source, &mut kinds);
        }
        None => collect_annotation_and_modifier_markers(node, source, &mut kinds),
    }

    if node.kind() == "class_declaration" {
        collect_implemented_interface_markers(node, source, &mut kinds);
    }

    // Propagate interface entry-point markers from the enclosing class
    // to the contract method names the Salesforce platform dispatches
    // by name. Without this step, `Database.Batchable.start()` and
    // peers appear uncalled in the static graph (the Layer-5 Wave 1
    // hand-audit caught `CRLP_Batch_Base_NonSkew.start` misclassified
    // as `no_callers`). See `collect_interface_method_markers` for the
    // interface→method-name contract and the inheritance caveat.
    if node.kind() == "method_declaration" || node.kind() == "constructor_declaration" {
        collect_interface_method_markers(node, source, &mut kinds);
    }

    kinds.sort_by_key(|k| *k as u8);
    kinds.dedup();
    kinds
}

fn compile_annotation_query(query_str: &str) -> Option<Query> {
    Query::new(tree_sitter_sfapex_vendored::apex::language(), query_str).ok()
}

/// Query-based annotation scan. Mirrors the manual walk's
/// annotation-name handling but sources matches from
/// `configs/apex.yaml:annotations`, keeping YAML as the single
/// grammar-shape source.
fn collect_annotations_via_query(
    node: &Node,
    source: &[u8],
    query: &Query,
    out: &mut Vec<EntryPointKind>,
) {
    // Apex annotations only live directly on the declaration (method,
    // class, etc.). Scoping the query to the declaration node plus its
    // direct modifiers avoids spuriously picking annotations on nested
    // types inside a class body.
    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(query, *node, source);
    let name_capture_idx = query
        .capture_names()
        .iter()
        .position(|n| n == "annotation_name")
        .map(|i| i as u32);
    let annotation_capture_idx = query
        .capture_names()
        .iter()
        .position(|n| n == "annotation")
        .map(|i| i as u32);
    for mat in matches {
        // Only accept annotations whose parent chain terminates at the
        // target declaration without crossing another nested
        // declaration. This guards against the fact that tree-sitter
        // queries descend arbitrarily deep.
        let Some(ann_cap_idx) = annotation_capture_idx else {
            continue;
        };
        let annotation_node = match mat.captures.iter().find(|c| c.index == ann_cap_idx) {
            Some(c) => c.node,
            None => continue,
        };
        if !annotation_directly_on(&annotation_node, node) {
            continue;
        }
        let name_cap_idx = match name_capture_idx {
            Some(i) => i,
            None => continue,
        };
        if let Some(name_cap) = mat.captures.iter().find(|c| c.index == name_cap_idx) {
            if let Ok(text) = name_cap.node.utf8_text(source) {
                let short = text.rsplit('.').next().unwrap_or(text);
                if let Some(kind) = annotation_name_to_kind(short) {
                    out.push(kind);
                }
            }
        }
    }
}

/// True when `annotation_node` is a direct child of `target` or of
/// `target`'s `modifiers` wrapper. Anything deeper (nested class,
/// inner method body) is rejected — those annotations belong to a
/// different declaration and must not leak into this one's tag set.
fn annotation_directly_on(annotation_node: &Node, target: &Node) -> bool {
    let Some(parent) = annotation_node.parent() else {
        return false;
    };
    if parent.id() == target.id() {
        return true;
    }
    if parent.kind() == "modifiers" {
        if let Some(grandparent) = parent.parent() {
            return grandparent.id() == target.id();
        }
    }
    false
}

/// True if the node carries any entry-point marker.
pub fn is_entry_point(node: &Node, source: &[u8]) -> bool {
    !classify(node, source).is_empty()
}

// ---------------------------------------------------------------------------
// Annotation + modifier scanning
// ---------------------------------------------------------------------------

fn collect_annotation_and_modifier_markers(
    node: &Node,
    source: &[u8],
    out: &mut Vec<EntryPointKind>,
) {
    // Annotations and modifiers live either directly as children of the
    // declaration, or inside a `modifiers` wrapper. The grammar permits both
    // shapes, so we scan both.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "annotation" => push_annotation_marker(&child, source, out),
            "modifier" => push_modifier_marker(&child, source, out),
            "modifiers" => {
                let mut inner = child.walk();
                for m in child.children(&mut inner) {
                    match m.kind() {
                        "annotation" => push_annotation_marker(&m, source, out),
                        "modifier" => push_modifier_marker(&m, source, out),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// Modifier-only walk (used by the query-based path, which handles
/// annotations via YAML but has no grammar analogue for `global` /
/// `webservice` modifiers — those live as plain identifiers in the
/// modifier list).
fn collect_modifier_markers(node: &Node, source: &[u8], out: &mut Vec<EntryPointKind>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "modifier" => push_modifier_marker(&child, source, out),
            "modifiers" => {
                let mut inner = child.walk();
                for m in child.children(&mut inner) {
                    if m.kind() == "modifier" {
                        push_modifier_marker(&m, source, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn push_annotation_marker(annotation: &Node, source: &[u8], out: &mut Vec<EntryPointKind>) {
    if let Some(name) = annotation_short_name(annotation, source) {
        if let Some(kind) = annotation_name_to_kind(&name) {
            out.push(kind);
        }
    }
}

fn push_modifier_marker(modifier: &Node, source: &[u8], out: &mut Vec<EntryPointKind>) {
    if let Ok(text) = modifier.utf8_text(source) {
        match text.trim() {
            s if s.eq_ignore_ascii_case("global") => out.push(EntryPointKind::Global),
            s if s.eq_ignore_ascii_case("webservice") => out.push(EntryPointKind::Webservice),
            _ => {}
        }
    }
}

fn annotation_short_name(annotation: &Node, source: &[u8]) -> Option<String> {
    if let Some(name_node) = annotation.child_by_field_name("name") {
        if let Ok(text) = name_node.utf8_text(source) {
            let short = text.rsplit('.').next().unwrap_or(text);
            return Some(short.to_string());
        }
    }
    // Fallback: first identifier child.
    let mut cursor = annotation.walk();
    for child in annotation.children(&mut cursor) {
        if child.kind() == "identifier" {
            if let Ok(text) = child.utf8_text(source) {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// Map an annotation short name (case-insensitive) to an entry-point kind.
///
/// Returns `None` for annotations that do not designate an entry point
/// (e.g. `@IsTest`, `@SuppressWarnings`, custom user annotations).
fn annotation_name_to_kind(name: &str) -> Option<EntryPointKind> {
    // Keep this list dense and exhaustive. A missing entry here = a silent
    // false negative in entry-point detection, so new Apex annotations must
    // be reviewed and added explicitly.
    match name.to_ascii_lowercase().as_str() {
        "auraenabled" => Some(EntryPointKind::AuraEnabled),
        "invocablemethod" => Some(EntryPointKind::InvocableMethod),
        "httpget" => Some(EntryPointKind::HttpGet),
        "httppost" => Some(EntryPointKind::HttpPost),
        "httpput" => Some(EntryPointKind::HttpPut),
        "httpdelete" => Some(EntryPointKind::HttpDelete),
        "httppatch" => Some(EntryPointKind::HttpPatch),
        "restresource" => Some(EntryPointKind::RestResource),
        "future" => Some(EntryPointKind::Future),
        "remoteaction" => Some(EntryPointKind::RemoteAction),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Interface implementation scanning (class-level only)
// ---------------------------------------------------------------------------

fn collect_implemented_interface_markers(
    class_node: &Node,
    source: &[u8],
    out: &mut Vec<EntryPointKind>,
) {
    let Some(interfaces) = class_node.child_by_field_name("interfaces") else {
        return;
    };

    // `interfaces` -> `type_list` -> one or more type identifiers
    let mut cursor = interfaces.walk();
    for child in interfaces.children(&mut cursor) {
        if child.kind() == "type_list" {
            let mut type_cursor = child.walk();
            for type_node in child.children(&mut type_cursor) {
                if let Ok(text) = type_node.utf8_text(source) {
                    if let Some(kind) = interface_name_to_kind(text.trim()) {
                        out.push(kind);
                    }
                }
            }
        }
    }
}

/// When `method_node` sits inside a `class_declaration` whose
/// `implements` list contains a known platform interface, and the
/// method's simple name matches the interface contract, push the
/// interface's entry-point kind onto `out`.
///
/// Interface → platform-dispatched method names:
///
/// - `Database.Batchable`  → `start`, `execute`, `finish`
/// - `Schedulable`         → `execute`
/// - `Queueable`           → `execute`
/// - `Messaging.InboundEmailHandler` → `handleInboundEmail`
///
/// The platform calls these by name; overloads are irrelevant
/// (only one overload can satisfy the interface). Matching by
/// simple method name is therefore sufficient and avoids parsing
/// the parameter-type signature.
///
/// Scope limitation: we only inspect the *immediate* enclosing
/// `class_declaration`. A method on an abstract base class whose
/// concrete subclass declares the `implements` (e.g. NPSP's
/// `CRLP_Batch_Base_NonSkew.start`, subclassed by
/// `CRLP_Account_BATCH implements Database.Batchable`) is NOT
/// tagged here — that requires cross-class inheritance resolution
/// and is deferred to the Apex Framework Resolver (Wave 3).
/// Documented as a known gap in `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`.
fn collect_interface_method_markers(
    method_node: &Node,
    source: &[u8],
    out: &mut Vec<EntryPointKind>,
) {
    let Some(class_node) = enclosing_class(method_node) else {
        return;
    };
    let mut class_interface_kinds: Vec<EntryPointKind> = Vec::new();
    collect_implemented_interface_markers(&class_node, source, &mut class_interface_kinds);
    if class_interface_kinds.is_empty() {
        return;
    }

    let Some(name_node) = method_node.child_by_field_name("name") else {
        return;
    };
    let Ok(method_name) = name_node.utf8_text(source) else {
        return;
    };

    for kind in class_interface_kinds {
        if method_name_matches_interface_contract(method_name, kind) {
            out.push(kind);
        }
    }
}

/// Walk upward from `node` and return the first ancestor that is a
/// `class_declaration`, stopping if we hit a file-level node.
fn enclosing_class<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == "class_declaration" {
            return Some(n);
        }
        current = n.parent();
    }
    None
}

/// True when `method_name` is a method the Salesforce platform
/// invokes for the given implemented interface `kind`. See the
/// doc-comment on `collect_interface_method_markers` for the
/// per-interface contract.
fn method_name_matches_interface_contract(method_name: &str, kind: EntryPointKind) -> bool {
    match kind {
        EntryPointKind::Batchable => {
            matches!(method_name, "start" | "execute" | "finish")
        }
        EntryPointKind::Schedulable => method_name == "execute",
        EntryPointKind::Queueable => method_name == "execute",
        EntryPointKind::InboundEmailHandler => method_name == "handleInboundEmail",
        _ => false,
    }
}

/// Map an implemented-interface type string to an entry-point kind.
///
/// Accepts both bare names (`Schedulable`) and namespaced forms
/// (`Database.Batchable`, `Messaging.InboundEmailHandler`) — the Salesforce
/// platform namespaces these distinctively, so comparisons are exact on the
/// short name.
fn interface_name_to_kind(type_text: &str) -> Option<EntryPointKind> {
    // Strip any generic arguments: `Database.Batchable<SObject>` -> `Database.Batchable`
    let cleaned: String = type_text.chars().take_while(|c| *c != '<').collect();
    let short = cleaned.trim().rsplit('.').next()?.trim();
    match short {
        "Schedulable" => Some(EntryPointKind::Schedulable),
        "Batchable" => Some(EntryPointKind::Batchable),
        "Queueable" => Some(EntryPointKind::Queueable),
        "InboundEmailHandler" => Some(EntryPointKind::InboundEmailHandler),
        _ => None,
    }
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

    fn find<'a>(node: Node<'a>, kind: &str, name: &str, source: &[u8]) -> Option<Node<'a>> {
        if node.kind() == kind {
            if let Some(n) = node.child_by_field_name("name") {
                if n.utf8_text(source).ok() == Some(name) {
                    return Some(node);
                }
            }
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if let Some(found) = find(child, kind, name, source) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn method_with_auraenabled_is_classified() {
        let src =
            r#"public class C { @AuraEnabled public static String hello() { return 'hi'; } }"#;
        let tree = parse(src);
        let m = find(
            tree.root_node(),
            "method_declaration",
            "hello",
            src.as_bytes(),
        )
        .unwrap();
        assert_eq!(
            classify(&m, src.as_bytes()),
            vec![EntryPointKind::AuraEnabled]
        );
        assert!(is_entry_point(&m, src.as_bytes()));
    }

    #[test]
    fn method_with_invocable_is_classified() {
        let src = r#"public class C { @InvocableMethod public static void run() {} }"#;
        let tree = parse(src);
        let m = find(
            tree.root_node(),
            "method_declaration",
            "run",
            src.as_bytes(),
        )
        .unwrap();
        assert_eq!(
            classify(&m, src.as_bytes()),
            vec![EntryPointKind::InvocableMethod]
        );
    }

    #[test]
    fn method_with_httpget_is_classified() {
        let src = r#"public class C { @HttpGet public static String get() { return ''; } }"#;
        let tree = parse(src);
        let m = find(
            tree.root_node(),
            "method_declaration",
            "get",
            src.as_bytes(),
        )
        .unwrap();
        assert_eq!(classify(&m, src.as_bytes()), vec![EntryPointKind::HttpGet]);
    }

    #[test]
    fn class_with_restresource_is_classified() {
        let src = r#"@RestResource(urlMapping='/foo') global class FooApi {}"#;
        let tree = parse(src);
        let c = find(
            tree.root_node(),
            "class_declaration",
            "FooApi",
            src.as_bytes(),
        )
        .unwrap();
        let kinds = classify(&c, src.as_bytes());
        assert!(kinds.contains(&EntryPointKind::RestResource));
        assert!(kinds.contains(&EntryPointKind::Global));
    }

    #[test]
    fn global_webservice_method_gets_both_markers() {
        let src = r#"global class X { global webservice static Integer f() { return 1; } }"#;
        let tree = parse(src);
        let m = find(tree.root_node(), "method_declaration", "f", src.as_bytes()).unwrap();
        let kinds = classify(&m, src.as_bytes());
        assert!(kinds.contains(&EntryPointKind::Global));
        assert!(kinds.contains(&EntryPointKind::Webservice));
    }

    #[test]
    fn class_implementing_schedulable_is_classified() {
        let src = r#"public class Job implements Schedulable { public void execute(SchedulableContext c) {} }"#;
        let tree = parse(src);
        let c = find(tree.root_node(), "class_declaration", "Job", src.as_bytes()).unwrap();
        assert_eq!(
            classify(&c, src.as_bytes()),
            vec![EntryPointKind::Schedulable]
        );
    }

    #[test]
    fn class_implementing_database_batchable_is_classified() {
        let src = r#"public class Batch implements Database.Batchable<SObject> {
            public Database.QueryLocator start(Database.BatchableContext bc) { return null; }
            public void execute(Database.BatchableContext bc, List<SObject> scope) {}
            public void finish(Database.BatchableContext bc) {}
        }"#;
        let tree = parse(src);
        let c = find(
            tree.root_node(),
            "class_declaration",
            "Batch",
            src.as_bytes(),
        )
        .unwrap();
        assert_eq!(
            classify(&c, src.as_bytes()),
            vec![EntryPointKind::Batchable]
        );
    }

    #[test]
    fn class_implementing_queueable_is_classified() {
        let src =
            r#"public class Q implements Queueable { public void execute(QueueableContext c) {} }"#;
        let tree = parse(src);
        let c = find(tree.root_node(), "class_declaration", "Q", src.as_bytes()).unwrap();
        assert_eq!(
            classify(&c, src.as_bytes()),
            vec![EntryPointKind::Queueable]
        );
    }

    #[test]
    fn class_implementing_multiple_interfaces_gets_all_markers() {
        let src = r#"public class Both implements Schedulable, Queueable {
            public void execute(SchedulableContext c) {}
        }"#;
        let tree = parse(src);
        let c = find(
            tree.root_node(),
            "class_declaration",
            "Both",
            src.as_bytes(),
        )
        .unwrap();
        let kinds = classify(&c, src.as_bytes());
        assert!(kinds.contains(&EntryPointKind::Schedulable));
        assert!(kinds.contains(&EntryPointKind::Queueable));
    }

    #[test]
    fn regular_method_is_not_entry_point() {
        let src = r#"public class C { public static void helper() {} }"#;
        let tree = parse(src);
        let m = find(
            tree.root_node(),
            "method_declaration",
            "helper",
            src.as_bytes(),
        )
        .unwrap();
        assert!(classify(&m, src.as_bytes()).is_empty());
        assert!(!is_entry_point(&m, src.as_bytes()));
    }

    #[test]
    fn batchable_class_propagates_tag_to_start_execute_finish_methods() {
        // R18 / R20 hand-audit gap: the platform dispatches
        // Database.Batchable via the class's `start`, `execute`,
        // `finish` methods by NAME. The graph engine sees no caller
        // for these methods otherwise and flags them as no_callers.
        // Tagging the methods directly fixes this at the source.
        let src = r#"public class Batch implements Database.Batchable<SObject> {
            public Database.QueryLocator start(Database.BatchableContext bc) { return null; }
            public void execute(Database.BatchableContext bc, List<SObject> scope) {}
            public void finish(Database.BatchableContext bc) {}
            public void helper() {}
        }"#;
        let tree = parse(src);
        let source = src.as_bytes();
        for method_name in ["start", "execute", "finish"] {
            let m = find(tree.root_node(), "method_declaration", method_name, source).unwrap();
            let kinds = classify(&m, source);
            assert!(
                kinds.contains(&EntryPointKind::Batchable),
                "method {} on Batchable class must carry the Batchable tag; got {:?}",
                method_name,
                kinds
            );
        }
        // And non-contract methods must NOT be tagged.
        let helper = find(tree.root_node(), "method_declaration", "helper", source).unwrap();
        assert!(
            !classify(&helper, source).contains(&EntryPointKind::Batchable),
            "helper method must not receive the Batchable tag"
        );
    }

    #[test]
    fn schedulable_class_propagates_tag_to_execute_only() {
        let src = r#"public class Job implements Schedulable {
            public void execute(SchedulableContext c) {}
            public void helper() {}
        }"#;
        let tree = parse(src);
        let source = src.as_bytes();
        let exec = find(tree.root_node(), "method_declaration", "execute", source).unwrap();
        assert!(classify(&exec, source).contains(&EntryPointKind::Schedulable));
        let helper = find(tree.root_node(), "method_declaration", "helper", source).unwrap();
        assert!(!classify(&helper, source).contains(&EntryPointKind::Schedulable));
    }

    #[test]
    fn queueable_class_propagates_tag_to_execute_only() {
        let src = r#"public class Q implements Queueable {
            public void execute(QueueableContext c) {}
        }"#;
        let tree = parse(src);
        let source = src.as_bytes();
        let exec = find(tree.root_node(), "method_declaration", "execute", source).unwrap();
        assert!(classify(&exec, source).contains(&EntryPointKind::Queueable));
    }

    #[test]
    fn inbound_email_handler_propagates_tag_to_handleinboundemail() {
        let src = r#"global class Inbound implements Messaging.InboundEmailHandler {
            global Messaging.InboundEmailResult handleInboundEmail(Messaging.InboundEmail email, Messaging.InboundEnvelope envelope) { return null; }
        }"#;
        let tree = parse(src);
        let source = src.as_bytes();
        let h = find(
            tree.root_node(),
            "method_declaration",
            "handleInboundEmail",
            source,
        )
        .unwrap();
        assert!(classify(&h, source).contains(&EntryPointKind::InboundEmailHandler));
    }

    #[test]
    fn interface_method_propagation_does_not_apply_to_non_implementing_classes() {
        // Regression: a class that does NOT implement Batchable must
        // not have its `start`/`execute`/`finish` methods mis-tagged.
        let src = r#"public class Plain {
            public void start() {}
            public void execute() {}
        }"#;
        let tree = parse(src);
        let source = src.as_bytes();
        for method_name in ["start", "execute"] {
            let m = find(tree.root_node(), "method_declaration", method_name, source).unwrap();
            let kinds = classify(&m, source);
            assert!(
                !kinds.contains(&EntryPointKind::Batchable),
                "method {} on a non-Batchable class must NOT be tagged",
                method_name
            );
        }
    }

    #[test]
    fn interface_method_propagation_ignores_abstract_base_without_direct_implements() {
        // Documents the known scope limitation: when a subclass
        // declares `implements Database.Batchable` but the `start`
        // method lives on its abstract parent class, the parent's
        // `start` is NOT tagged here. Deferred to the Apex Framework
        // Resolver (Wave 3). NPSP's CRLP_Batch_Base_NonSkew.start
        // falls into this bucket.
        let src = r#"public abstract class BaseBatch {
            public Database.QueryLocator start(Database.BatchableContext bc) { return null; }
        }"#;
        let tree = parse(src);
        let source = src.as_bytes();
        let m = find(tree.root_node(), "method_declaration", "start", source).unwrap();
        assert!(
            !classify(&m, source).contains(&EntryPointKind::Batchable),
            "abstract base start() must not be tagged without direct `implements` on its class"
        );
    }

    #[test]
    fn annotation_case_insensitive() {
        for variant in ["@AuraEnabled", "@auraenabled", "@AURAENABLED"] {
            let src = format!("public class C {{ {variant} public static void f() {{}} }}");
            let tree = parse(&src);
            let m = find(tree.root_node(), "method_declaration", "f", src.as_bytes()).unwrap();
            assert_eq!(
                classify(&m, src.as_bytes()),
                vec![EntryPointKind::AuraEnabled],
                "variant {variant} should classify"
            );
        }
    }
}
