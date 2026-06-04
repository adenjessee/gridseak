//! Managed-package namespace reference extraction.
//!
//! Salesforce orgs typically depend on managed packages installed from
//! AppExchange or shared between business units. Each managed package
//! ships under a unique namespace prefix (e.g. `npsp`, `npe01`, `fflib`,
//! `pse`). Apex code in the consumer org references managed package
//! types via:
//!
//! - **Qualified type names**: `npsp.OppPaymentService`, `fflib.SObjectUnitOfWork`.
//! - **Custom-object API names**: `npsp__Allocation__c`, `pse__Project__c`.
//! - **Custom-field API names**: `Account.npsp__External_Id__c`.
//! - **Annotations or modifiers** that surface as identifiers in source.
//!
//! These references matter because:
//!
//! 1. The customer **cannot modify** managed package code. It is a black
//!    box. Heavy coupling to a managed namespace is a structural liability
//!    — if the package is upgraded, deprecated, or has a breaking change,
//!    every consumer site must be revisited.
//! 2. Coupling concentration to a single namespace is an architectural
//!    smell distinct from intra-codebase coupling: the customer has zero
//!    refactoring leverage to reduce it. Refactoring requires either
//!    abstracting the dependency behind an internal facade, paying for
//!    a different package, or building the capability in-house.
//! 3. Visualizing each namespace as a virtual `Module` node in the graph
//!    surfaces the dependency as first-class structural information so
//!    the dashboard, blast-radius, and coupling reports treat it as an
//!    edge target rather than silently dropping the reference.
//!
//! # Design contract
//!
//! This module is the *detection layer*. It walks an Apex parse tree and
//! returns a flat list of [`ManagedReferenceSite`]s — one per occurrence
//! in source. Aggregation into per-namespace inventories (consumer file
//! lists, fan-in counts) is the caller's job, mirroring how
//! [`super::trigger_framework`] returns facts and the orchestrator owns
//! graph mutation.
//!
//! Detection rules:
//!
//! Detection rules (intentionally conservative after the NPSP
//! false-positive post-mortem — see `docs/workstreams/apex/NEXT_STEPS_PLAN.md`
//! Sprint B.7-P0):
//!
//! 1. **Canonical `<namespace>__<name>` form** is the only unambiguous
//!    signal and fires on any identifier-shaped token (bare identifier,
//!    type identifier, annotation). Delegated to
//!    [`super::class_registry::extract_managed_namespace`], which
//!    already rejects the custom-object suffix markers (`__c`, `__mdt`,
//!    `__e`, `__b`, `__x`).
//!
//! 2. **Bare-dotted `<namespace>.<TypeName>` form** fires only inside
//!    explicit type contexts — `scoped_type_identifier` (variable
//!    declarations, return types, generic parameters, `new`
//!    expressions) and `annotation` nodes. The Apex grammar emits those
//!    kinds only when the parser has committed to a type
//!    interpretation, so false positives from local-variable receiver
//!    chains are structurally impossible there. As a belt-and-braces
//!    check we also require the token after the dot to be PascalCase.
//!
//! 3. **Expression-level chains** (`field_access`, `method_invocation`)
//!    are **deliberately not walked**. They span shapes like
//!    `this.field`, `acc.Name`,
//!    `opp.Account.Owner.Name.toString()` and
//!    `fflib.Application.UnitOfWork.commitWork()`; the first three are
//!    local-variable / SObject-relationship traversals and the last is
//!    a managed-package call, but the parse tree does not distinguish
//!    them — only type information (LSP) can. Refusing to guess keeps
//!    the coupling inventory honest and actionable. Consumers that use
//!    a managed type almost always declare a variable of that type
//!    first, so rule (2) still captures the coupling.
//!
//! 4. Apex **system namespaces** (`System`, `Database`, `Schema`, …)
//!    are filtered out so first-party Salesforce APIs don't pollute
//!    the inventory.
//!
//! 5. Results are **deduplicated per `(namespace, byte_range)`** so a
//!    multi-walk over the same source span counts once.

use std::collections::{BTreeSet, HashSet};

use tree_sitter::{Node, Tree, TreeCursor};

use crate::domain::{
    Confidence, Edge, EdgeKind, Node as DomainNode, NodeKind, Provenance, ProvenanceSource, Range,
};

use super::class_registry::extract_managed_namespace;

/// A single occurrence of a managed-package reference in Apex source.
///
/// One physical token in the source produces one `ManagedReferenceSite`,
/// even if the same namespace appears dozens of times in the same file.
/// Aggregation across sites is the caller's job — keeping this layer flat
/// makes the detector trivial to test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedReferenceSite {
    /// The managed-package namespace, lowercased for case-insensitive
    /// grouping. Apex is case-insensitive at the language level, and
    /// Salesforce installs namespaces case-preserved but matches them
    /// case-insensitively. We canonicalise on lowercase here so two
    /// references to `NPSP` and `npsp` collapse to one bucket.
    pub namespace: String,
    /// The full reference text as it appeared in source (e.g.
    /// `npsp__Allocation__c`, `npsp.OppPaymentService`). Preserved with
    /// original casing for finding messages and UI display.
    pub reference_text: String,
    /// What kind of reference shape produced this hit — useful for
    /// telemetry and for downstream consumers who want to weigh
    /// type-level (`SObject`) vs API-call (`QualifiedType`) references
    /// differently.
    pub kind: ReferenceKind,
    /// Source location of the reference. Lets the orchestrator attach
    /// the site to a specific class/method node for blast-radius and
    /// fan-in computation.
    pub range: Range,
}

/// The structural shape that produced the reference. Distinguishes
/// SObject (`npsp__Foo__c`) from qualified type reference (`npsp.Foo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// `<namespace>__<name>__<suffix>` — managed-package custom SObject,
    /// custom field, custom metadata, custom setting, or platform event.
    SObjectOrCustomEntity,
    /// `<namespace>.<name>[.<more>]` — managed-package class, interface,
    /// enum, or annotation.
    QualifiedType,
}

/// Extract every managed-package reference from one Apex parse tree.
///
/// Returns a deduplicated list — references that resolve to the same
/// `(namespace, byte_range)` are collapsed to a single entry, so callers
/// can feed the result straight into per-file fan-in counts without
/// double-counting.
///
/// The function is read-only with respect to the tree and source.
pub fn extract(tree: &Tree, source: &[u8], file_path: &str) -> Vec<ManagedReferenceSite> {
    let mut out = Vec::new();
    let mut seen: HashSet<(String, usize, usize)> = HashSet::new();
    let mut cursor = tree.walk();
    walk(&mut cursor, source, file_path, &mut out, &mut seen);
    out
}

fn walk(
    cursor: &mut TreeCursor,
    source: &[u8],
    file_path: &str,
    out: &mut Vec<ManagedReferenceSite>,
    seen: &mut HashSet<(String, usize, usize)>,
) {
    let node = cursor.node();
    inspect_node(&node, source, file_path, out, seen);

    if cursor.goto_first_child() {
        loop {
            walk(cursor, source, file_path, out, seen);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn inspect_node(
    node: &Node,
    source: &[u8],
    file_path: &str,
    out: &mut Vec<ManagedReferenceSite>,
    seen: &mut HashSet<(String, usize, usize)>,
) {
    // Cheap early exit: only walk token shapes that can carry namespace
    // signal. `field_access` and `method_invocation` are **intentionally
    // excluded**. Their text spans `receiver.field` / `receiver.call(...)`
    // patterns whose leftmost segment is almost always a local variable
    // (`this.x`, `acc.Name`, `response.body`) — walking them and then
    // running the dotted-leftseg fallback was the root cause of the
    // NPSP false-positive explosion where common variable names like
    // `acc`, `opp`, `this`, `result` were reported as managed packages.
    //
    // Bare identifier tokens (the children of those nodes we dropped)
    // are still walked; they just don't trigger the dotted-leftseg path
    // because their text has no `.`. That keeps real `__`-shaped hits
    // (e.g. `npsp__Household__c`) while eliminating the false-positive
    // class.
    //
    // The remaining kinds are the ones where the parser has committed
    // to a type/annotation interpretation of the token:
    //   * `type_identifier` — bare type name reference.
    //   * `scoped_type_identifier` — qualified type (`pi.FormHelper`).
    //   * `annotation` — `@npsp.Foo`.
    //   * `identifier` — picks up `__`-shaped tokens inside any context.
    let kind = node.kind();
    let interesting = matches!(
        kind,
        "identifier" | "type_identifier" | "scoped_type_identifier" | "annotation"
    );
    if !interesting {
        return;
    }

    let Ok(text) = node.utf8_text(source) else {
        return;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }

    let mut candidates: Vec<(String, ReferenceKind)> = Vec::new();

    if let Some(ns) = extract_managed_namespace(trimmed) {
        // Rule 1: canonical `<namespace>__<name>` form — fires on
        // `type_identifier`, `identifier`, and the leftmost segment of
        // a `scoped_type_identifier` (via `extract_managed_namespace`'s
        // built-in dotted-leftseg handling).
        candidates.push((ns, classify_kind(trimmed)));
    } else if matches!(kind, "scoped_type_identifier" | "annotation") {
        // Rule 2: bare-dotted form (`pi.FormHelper`) — **only** allowed
        // inside explicit type/annotation contexts. Scoped_type_identifier
        // is emitted by the Apex grammar specifically when the parser
        // knows the token names a type, so false positives from local
        // variable access chains are not possible here. Additionally
        // require the token after the dot to start with uppercase
        // (PascalCase) to stay robust against grammar surprises.
        if let Some((left, rest)) = trimmed.split_once('.') {
            let looks_like_type = rest
                .trim_start()
                .chars()
                .next()
                .map(|c| c.is_ascii_uppercase())
                .unwrap_or(false);
            if looks_like_type {
                if let Some(ns) = qualified_left_segment_namespace(left) {
                    candidates.push((ns, ReferenceKind::QualifiedType));
                }
            }
        }
    }

    for (namespace, kind) in candidates {
        if is_system_namespace(&namespace) {
            continue;
        }
        let span = node.byte_range();
        let key = (namespace.clone(), span.start, span.end);
        if !seen.insert(key) {
            continue;
        }
        out.push(ManagedReferenceSite {
            namespace,
            reference_text: trimmed.to_string(),
            kind,
            range: Range::with_file(
                node.start_position().row as u32 + 1,
                node.start_position().column as u32,
                node.end_position().row as u32 + 1,
                node.end_position().column as u32,
                file_path.to_string(),
            ),
        });
    }
}

/// Classify the structural shape of a token whose text passed
/// `extract_managed_namespace`. SObject-style markers retain the
/// `__<name>` suffix; everything else is treated as a qualified type
/// reference.
fn classify_kind(text: &str) -> ReferenceKind {
    if text.contains("__") {
        ReferenceKind::SObjectOrCustomEntity
    } else {
        ReferenceKind::QualifiedType
    }
}

/// Recognise the `<namespace>` left of a dotted reference like
/// `npsp.Foo`. Stricter than `extract_managed_namespace` because we are
/// inferring namespace from a single bare segment without a `__` marker
/// — the only signal is that it is not a known system namespace and has
/// the conservative shape of an installed package prefix (alphanumeric,
/// short, lowercase-ish).
///
/// Returns `None` for ambiguous segments to avoid false-positives from
/// common Apex patterns like `MyClass.method()` or `Foo.Bar.baz`.
fn qualified_left_segment_namespace(left: &str) -> Option<String> {
    let trimmed = left.trim();
    if trimmed.is_empty() || trimmed.len() > 15 {
        return None;
    }
    if !trimmed.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if is_system_namespace(&lower) {
        return None;
    }
    // Salesforce managed namespaces are typically lowercase or mixed
    // case. A single PascalCase token (`Foo`) almost always names a
    // user class, not a managed namespace. Require either lowercase
    // start or presence of a digit to avoid the high false-positive
    // rate of `UserClass.method` patterns.
    let first = trimmed.chars().next()?;
    if first.is_ascii_uppercase() && !trimmed.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(lower)
}

/// True when the namespace is a built-in Salesforce / Apex system
/// namespace and therefore not a managed-package coupling concern.
///
/// Source: Apex Developer Guide → "System namespace" reference list.
/// Compared lowercased because Apex matches namespace prefixes
/// case-insensitively at the language level.
fn is_system_namespace(ns_lower: &str) -> bool {
    APEX_SYSTEM_NAMESPACES.contains(&ns_lower)
}

/// Lowercased list of Apex system namespaces. Anything not in this set
/// and matching the managed-namespace shape is treated as a managed
/// package reference. Keep this list tightly scoped — the cost of
/// listing a real namespace here is silently dropping coupling signal,
/// while the cost of *omitting* a system namespace is over-counting.
///
/// Sourced from <https://developer.salesforce.com/docs/atlas.en-us.apexref.meta/apexref/apex_namespace_overview.htm>.
const APEX_SYSTEM_NAMESPACES: &[&str] = &[
    "apex",
    "appauth",
    "approval",
    "auth",
    "cache",
    "canvas",
    "chatteranswers",
    "commercepayments",
    "communitiesgroups",
    "connectapi",
    "database",
    "datacloud",
    "datasource",
    "dataweave",
    "dom",
    "eventbus",
    "exception",
    "external",
    "externalservice",
    "flow",
    "functions",
    "invocable",
    "kbmanagement",
    "limits",
    "messaging",
    "metadata",
    "pref",
    "process",
    "quickaction",
    "reports",
    "schema",
    "search",
    "sfc",
    "sfdc_checkout",
    "site",
    "support",
    "system",
    "territorymgmt",
    "test",
    "trigger",
    "txnsecurity",
    "userinfo",
    "userprovisioning",
    "visualeditor",
    "wave",
];

/// Lightweight aggregation helper: collapse a flat list of sites into
/// the unique namespaces present, sorted for deterministic output.
///
/// Useful for callers that only need the set of namespaces touched by a
/// given file (e.g., for emitting one `IMPORTS → npsp` edge per consumer
/// rather than one per occurrence).
pub fn unique_namespaces(sites: &[ManagedReferenceSite]) -> Vec<String> {
    let mut set: BTreeSet<&str> = BTreeSet::new();
    for s in sites {
        set.insert(&s.namespace);
    }
    set.into_iter().map(str::to_string).collect()
}

// -----------------------------------------------------------------------------
// Virtual-module synthesis
// -----------------------------------------------------------------------------

/// FQN prefix used for synthesized external Module nodes representing
/// managed-package namespaces. Stable across runs because the
/// downstream graph store keys on FQN; downstream readers can filter on
/// this prefix to identify virtual external dependencies vs. real
/// in-repo modules.
pub const VIRTUAL_MANAGED_MODULE_FQN_PREFIX: &str = "external::salesforce::managed_package::";

/// Source-file marker placed on synthesized Module nodes so reports and
/// the validation UI can distinguish virtual external dependencies from
/// real in-repo files.
pub const VIRTUAL_MANAGED_MODULE_FILE_SENTINEL: &str = "<external:managed_package>";

/// Build a synthetic external [`Module`](crate::domain::NodeKind::Module)
/// node representing one managed-package namespace.
///
/// The returned node has:
/// - A stable FQN of the form `external::salesforce::managed_package::<ns>`
///   (lowercased namespace) so that re-runs produce the same node id.
/// - A placeholder [`Range`] anchored to a sentinel file path
///   ([`VIRTUAL_MANAGED_MODULE_FILE_SENTINEL`]) — these nodes do not
///   correspond to source the customer owns, so any "real" file path
///   would be a lie.
/// - [`ProvenanceSource::Heuristic`] with [`Confidence::High`] —
///   high confidence because namespace detection itself is deterministic
///   from source, but it is *heuristic* because the package is opaque
///   to the parser (no LSP can introspect a managed package's internals).
pub fn synthesize_module_node(namespace: &str) -> DomainNode {
    let ns = namespace.to_ascii_lowercase();
    let fqn = format!("{VIRTUAL_MANAGED_MODULE_FQN_PREFIX}{ns}");
    let location = Range::with_file(0, 0, 0, 0, VIRTUAL_MANAGED_MODULE_FILE_SENTINEL.to_string());
    let provenance = Provenance::new(ProvenanceSource::Heuristic, Confidence::High);
    let mut node = DomainNode::new(NodeKind::Module, fqn, location, provenance);
    node.set_property("is_external", true);
    node.set_property("external_kind", "salesforce_managed_package");
    node.set_property("namespace", ns.clone());

    // Sprint H.2 — curated ecosystem-package metadata. Known namespaces
    // get stable `display_name`, `vendor`, and `category` properties so
    // downstream risk scoring and UI can differentiate NPSP (a
    // Salesforce.org nonprofit product) from an unknown third-party
    // package. Unknown namespaces surface as `is_known_ecosystem_package
    // = false` with no labels — consumers that read only `namespace`
    // continue to work unchanged.
    //
    // Forward-compatibility TODO: if customer demand emerges to
    // register their own managed packages, add a
    // `managed_package_overrides.yaml` loader that merges over
    // `managed_package_registry::KNOWN_PACKAGES`, preserving the
    // `lookup(&ns)` signature used here.
    match super::managed_package_registry::lookup(&ns) {
        Some(pkg) => {
            node.set_property("is_known_ecosystem_package", true);
            node.set_property("display_name", pkg.display_name.to_string());
            node.set_property("vendor", pkg.vendor.as_property_str().to_string());
            node.set_property("category", pkg.category.as_property_str().to_string());
        }
        None => {
            node.set_property("is_known_ecosystem_package", false);
        }
    }

    node
}

/// Build the `Import` edge that records a single consumer's dependency
/// on a managed-package namespace.
///
/// `consumer_node_id` is the graph id of the importing Apex node
/// (typically the class or trigger). `external_module_node_id` is the
/// id returned by [`synthesize_module_node`] for the same namespace.
///
/// Provenance mirrors the synthesized-node provenance:
/// `Heuristic / High`. Edges between in-repo nodes will continue to be
/// produced by the LSP / heuristic resolvers separately — this helper
/// is exclusively for the consumer→external-module link.
pub fn synthesize_import_edge(consumer_node_id: String, external_module_node_id: String) -> Edge {
    Edge::new(
        consumer_node_id,
        external_module_node_id,
        EdgeKind::Import,
        Provenance::new(ProvenanceSource::Heuristic, Confidence::High),
    )
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_sfapex_vendored::apex::language())
            .expect("set apex grammar");
        parser.parse(source, None).expect("parse apex")
    }

    fn extract_for(source: &str) -> Vec<ManagedReferenceSite> {
        let tree = parse(source);
        extract(&tree, source.as_bytes(), "Test.cls")
    }

    #[test]
    fn detects_managed_sobject_reference() {
        let src = r#"
            public class Demo {
                public void run(npsp__Allocation__c rec) {}
            }
        "#;
        let sites = extract_for(src);
        assert!(
            sites
                .iter()
                .any(|s| s.namespace == "npsp" && s.kind == ReferenceKind::SObjectOrCustomEntity),
            "should detect npsp__Allocation__c as SObject reference: {sites:?}",
        );
    }

    #[test]
    fn detects_qualified_type_reference() {
        let src = r#"
            public class Demo {
                public void run() {
                    npsp.OppPaymentService svc = new npsp.OppPaymentService();
                    svc.allocate();
                }
            }
        "#;
        let sites = extract_for(src);
        assert!(
            sites
                .iter()
                .any(|s| s.namespace == "npsp" && s.kind == ReferenceKind::QualifiedType),
            "should detect npsp.OppPaymentService qualified reference: {sites:?}",
        );
    }

    #[test]
    fn rejects_apex_system_namespaces() {
        let src = r#"
            public class Demo {
                public void run() {
                    Database.insert(new Account());
                    System.debug('hello');
                    Schema.SObjectType t = Account.SObjectType;
                    Test.startTest();
                }
            }
        "#;
        let sites = extract_for(src);
        assert!(
            sites.is_empty(),
            "system namespaces (Database/System/Schema/Test) must not be reported as managed: {sites:?}",
        );
    }

    #[test]
    fn rejects_user_class_qualified_calls() {
        let src = r#"
            public class Demo {
                public void run() {
                    MyService svc = MyService.getInstance();
                    svc.doSomething();
                }
            }
        "#;
        let sites = extract_for(src);
        assert!(
            !sites.iter().any(|s| s.namespace == "myservice"),
            "user PascalCase class must not be mistaken for managed namespace: {sites:?}",
        );
    }

    #[test]
    fn rejects_unmanaged_custom_objects() {
        let src = r#"
            public class Demo {
                public void run(Allocation__c rec) {}
            }
        "#;
        let sites = extract_for(src);
        assert!(
            sites.is_empty(),
            "Allocation__c (no namespace) must not produce a managed reference: {sites:?}",
        );
    }

    #[test]
    fn deduplicates_repeated_references_in_same_span() {
        let src = r#"
            public class Demo {
                public void run() {
                    npsp.Foo a = new npsp.Foo();
                    npsp.Foo b = new npsp.Foo();
                }
            }
        "#;
        let sites = extract_for(src);
        // Each occurrence has its own byte range, so two declarations
        // produce at least two sites (constructor + type), but the
        // dedup must collapse identical (namespace, byte_range) pairs.
        let occurrences = sites.iter().filter(|s| s.namespace == "npsp").count();
        assert!(
            occurrences >= 2,
            "expected multiple distinct npsp sites across the source: {sites:?}",
        );
        // Each emitted site has a unique source position.
        let mut spans: Vec<(u32, u32, u32, u32)> = sites
            .iter()
            .map(|s| {
                (
                    s.range.start_line,
                    s.range.start_char,
                    s.range.end_line,
                    s.range.end_char,
                )
            })
            .collect();
        spans.sort();
        spans.dedup();
        assert_eq!(
            spans.len(),
            sites.len(),
            "no two emitted sites share a source position",
        );
    }

    #[test]
    fn unique_namespaces_collapses_multi_namespace_file() {
        let src = r#"
            public class Demo {
                public void run(npsp__Allocation__c a, fflib__Application app) {
                    npsp.Service.run();
                    fflib.Application.SomeOption opt;
                }
            }
        "#;
        let sites = extract_for(src);
        let ns = unique_namespaces(&sites);
        assert_eq!(
            ns,
            vec!["fflib".to_string(), "npsp".to_string()],
            "should report exactly the unique namespaces, sorted",
        );
    }

    #[test]
    fn case_insensitive_namespace_grouping() {
        let src = r#"
            public class Demo {
                public NPSP.Foo a;
                public npsp__Bar__c b;
            }
        "#;
        let sites = extract_for(src);
        let unique = unique_namespaces(&sites);
        assert_eq!(
            unique,
            vec!["npsp".to_string()],
            "case variants must collapse to a single namespace bucket: {sites:?}",
        );
    }

    #[test]
    fn empty_tree_produces_no_sites() {
        let sites = extract_for("");
        assert!(sites.is_empty());
    }

    #[test]
    fn synthesized_module_is_stable_and_marked_external() {
        let a = synthesize_module_node("npsp");
        let b = synthesize_module_node("NPSP");
        assert_eq!(
            a.id, b.id,
            "case differences in namespace must yield same node id"
        );
        assert_eq!(a.kind, NodeKind::Module);
        assert!(
            a.fqn.starts_with(VIRTUAL_MANAGED_MODULE_FQN_PREFIX),
            "fqn must use the documented external prefix: {}",
            a.fqn,
        );
        assert_eq!(a.location.file, VIRTUAL_MANAGED_MODULE_FILE_SENTINEL);
        assert_eq!(
            a.properties.get("is_external"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            a.properties.get("external_kind"),
            Some(&serde_json::json!("salesforce_managed_package")),
        );
        assert_eq!(
            a.properties.get("namespace"),
            Some(&serde_json::json!("npsp"))
        );
    }

    #[test]
    fn synthesized_module_carries_curated_registry_metadata_for_known_namespace() {
        // Sprint H.2 — known Salesforce ecosystem packages get
        // display_name / vendor / category labels attached.
        let node = synthesize_module_node("npsp");
        assert_eq!(
            node.properties.get("is_known_ecosystem_package"),
            Some(&serde_json::json!(true)),
        );
        assert_eq!(
            node.properties.get("display_name"),
            Some(&serde_json::json!("Nonprofit Success Pack")),
        );
        assert_eq!(
            node.properties.get("vendor"),
            Some(&serde_json::json!("salesforce_org")),
        );
        assert_eq!(
            node.properties.get("category"),
            Some(&serde_json::json!("nonprofit")),
        );
    }

    #[test]
    fn synthesized_module_for_unknown_namespace_marks_not_known_and_omits_labels() {
        // Unknown namespaces must preserve backward compatibility:
        // namespace + is_external + external_kind stay, and the new
        // flag is explicitly false so downstream risk scoring can
        // distinguish "unknown" from "absent field".
        let node = synthesize_module_node("totallyunknownpkg");
        assert_eq!(
            node.properties.get("is_known_ecosystem_package"),
            Some(&serde_json::json!(false)),
        );
        assert!(
            !node.properties.contains_key("display_name"),
            "display_name must be absent for unknown namespaces, not a blank string"
        );
        assert!(!node.properties.contains_key("vendor"));
        assert!(!node.properties.contains_key("category"));
        // Core properties still present.
        assert_eq!(
            node.properties.get("namespace"),
            Some(&serde_json::json!("totallyunknownpkg")),
        );
    }

    #[test]
    fn synthesized_import_edge_targets_the_external_module() {
        let module = synthesize_module_node("fflib");
        let edge = synthesize_import_edge("file::classes/MyClass.cls".into(), module.id.clone());
        assert_eq!(edge.kind, EdgeKind::Import);
        assert_eq!(edge.from_id, "file::classes/MyClass.cls");
        assert_eq!(edge.to_id, module.id);
    }

    #[test]
    fn detects_namespaces_through_type_declaration_before_method_use() {
        // Realistic managed-package usage: the consumer declares a
        // variable with the managed type first, then invokes methods on
        // it. The type declaration surfaces as `scoped_type_identifier`,
        // which is a structurally unambiguous signal (the Apex parser
        // emits it only in type contexts). We detect via that node.
        //
        // The pure expression-chain form without a prior declaration
        // (e.g. `fflib.Application.UnitOfWork.commitWork();` as a
        // fire-and-forget statement) is *not* supported: those chains
        // are syntactically indistinguishable from SObject relationship
        // traversal (`opp.Account.Owner.Name.toString()`), and without
        // type information we refuse to guess. See
        // `pure_method_chain_without_type_declaration_is_not_detected`
        // below — that tradeoff is intentional.
        let src = r#"
            public class Demo {
                public void run() {
                    fflib.Application.UnitOfWork uow = new fflib.Application.UnitOfWork();
                    uow.commitWork();
                }
            }
        "#;
        let sites = extract_for(src);
        assert!(
            sites.iter().any(|s| s.namespace == "fflib"),
            "should detect fflib namespace from type declaration: {sites:?}",
        );
    }

    #[test]
    fn pure_method_chain_without_type_declaration_is_not_detected() {
        // Documents the intentional gap — see the sibling test above
        // for the rationale. If you reach for a fix that makes this
        // test pass, make sure you also stop the detector from firing
        // on `opp.Account.Owner.Name.toString()`, which has the same
        // syntactic shape. The only reliable disambiguation is type
        // information, which lives in the LSP layer.
        let src = r#"
            public class Demo {
                public void run() {
                    fflib.Application.UnitOfWork.commitWork();
                }
            }
        "#;
        let sites = extract_for(src);
        assert!(
            sites.is_empty(),
            "pure method-chain form must not fire (false-positive risk from SObject relationship traversal): {sites:?}",
        );
    }

    #[test]
    fn sobject_relationship_traversal_on_local_var_is_not_detected() {
        // Root cause of the pre-fix NPSP false-positive explosion:
        // `acc.Name`, `this.field`, `response.body`, `opp.Account.Name`
        // etc. are local-variable / relationship-traversal chains, NOT
        // managed-package references. The detector must stay silent on
        // them so the coupling inventory remains actionable.
        let src = r#"
            public class Demo {
                public void run(Account acc, Opportunity opp) {
                    String n1 = acc.Name;
                    String n2 = this.toString();
                    String n3 = opp.Account.Owner.Name;
                    acc.save();
                    String body = response.body;
                }
            }
        "#;
        let sites = extract_for(src);
        assert!(
            sites.is_empty(),
            "local variable and SObject relationship chains must not produce managed-package references: {sites:?}",
        );
    }
}
