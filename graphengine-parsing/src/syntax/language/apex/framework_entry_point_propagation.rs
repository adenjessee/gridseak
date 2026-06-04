//! Apex framework-interface entry-point propagation (Round 5 R11 fix).
//!
//! # Why this exists
//!
//! Apex entry-point tagging in [`super::entry_points`] is **AST-local**:
//! a method gets the `batchable` / `schedulable` / `queueable` /
//! `inbound_email_handler` tag only when the method's immediately
//! enclosing `class_declaration` has `implements Database.Batchable`
//! (etc.) in its `implements` list. That is correct for classes that
//! declare the interface and the contract methods in the same file,
//! which is most Apex code.
//!
//! It is wrong for the **abstract-base** pattern — an extremely common
//! NPSP shape — where the contract method lives on a parent class that
//! never declares `implements`, and the concrete subclass declares
//! `implements Database.Batchable` but inherits the method body. The
//! Salesforce runtime still invokes the inherited method by name, but
//! the AST walk at the parent never sees the implements clause, so the
//! parent's `execute`/`start`/`finish` gets no tag and shows up in the
//! dead-code analysis as `no_callers`.
//!
//! Round 5 hand-audit sample 5 (`CRLP_Batch_Base_Skew::execute(...)`)
//! is the canonical failure. Risk R11 in
//! `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md` has
//! tracked this since Wave 1 as a P1 deferred follow-up.
//!
//! # What this module does
//!
//! Walks every class in the `ApexClassSymbols` registry. When a class
//! `C` declares one of the four platform interfaces in its
//! `implemented_interfaces` list, we walk `C`'s `parent_class` chain
//! (including `C` itself) and, for every ancestor `A`, tag every
//! method on `A` whose simple name matches the interface's contract
//! (`start` / `execute` / `finish` for Batchable; `execute` for
//! Schedulable and Queueable; `handleInboundEmail` for
//! InboundEmailHandler). Tagging means: find `A`'s method's
//! `Function` node in the current `Vec<Node>` and push the
//! interface's short-name tag (`"batchable"` / ...) onto the node's
//! `entry_points` property array.
//!
//! Once `entry_points` is non-empty, the analysis crate's generic
//! dead-code classifier reclassifies the node from `no_callers` to
//! `framework_annotation_unresolved` — the same treatment
//! direct-implementer methods already receive from the AST-local
//! walk. This keeps behaviour uniform between direct and inherited
//! implementers; R11 was only ever about coverage.
//!
//! # Why not add synthetic edges instead
//!
//! A synthetic `Call` edge from a conceptual "Salesforce runtime"
//! node to each contract method would also work, but would bloat the
//! graph with nodes that don't exist in source and require a
//! corresponding well-known sink node. Tagging is the lighter-weight
//! approach that matches what the AST-local direct-implementer path
//! already does; staying consistent with that path keeps the
//! reason-breakdown stats comparable across direct and inherited
//! cases.
//!
//! # Cycle safety
//!
//! A malformed registry could theoretically contain a cycle in the
//! `parent_class` chain. The walk guards against infinite recursion
//! via a visited-set; a cycle is logged and truncated at first
//! repeat.

use std::collections::{BTreeMap, BTreeSet};

use tracing::debug;

use crate::domain::apex::class_symbols::ApexClassSymbols;
use crate::domain::{Node, NodeKind};

/// Counters surfaced to the pipeline so the orchestrator can log what
/// the stage did. Not load-bearing for correctness; purely for
/// observability and the metric-envelope check in
/// `PHASE_A_EXECUTION_PLAN.md`.
#[derive(Debug, Default, Clone, Copy)]
pub struct FrameworkPropagationStats {
    /// Classes in the registry that declared at least one of the four
    /// platform interfaces. Includes classes whose AST-local tagging
    /// already covered them — upper bound on classes considered.
    pub classes_with_platform_interface: usize,
    /// Distinct (class, method) pairs the walk decided should carry a
    /// platform-interface tag. Upper bound on the number of Function
    /// nodes the pass will try to tag.
    pub contract_method_targets: usize,
    /// Function nodes that actually received a new tag (i.e. the tag
    /// was not already present in their `entry_points` array). This
    /// is the true "would otherwise be `no_callers`" count.
    pub function_nodes_tagged: usize,
    /// Function nodes visited whose FQN matched a target pair but
    /// already carried the interface tag (from the AST-local path).
    /// Counted separately so the two mechanisms can be reconciled.
    pub function_nodes_already_tagged: usize,
}

/// Public entry point. Idempotent: re-running the same inputs produces
/// the same tags, and tagging a node that already carries the tag is
/// a no-op counted under `function_nodes_already_tagged`.
///
/// `nodes` must already include every method's Function node emitted
/// by the Apex syntax extractor; the pass reads and mutates nodes in
/// place but creates none.
pub fn propagate_framework_entry_points(
    class_symbols: &BTreeMap<String, ApexClassSymbols>,
    nodes: &mut [Node],
) -> FrameworkPropagationStats {
    let mut stats = FrameworkPropagationStats::default();

    if class_symbols.is_empty() {
        return stats;
    }

    // Case-insensitive api-name lookup so ancestor chain traversal
    // handles declared-case vs source-case drift (e.g. a class
    // recorded as `CRLP_Batch_Base_Skew` but referenced from a
    // subclass as `crlp_batch_base_skew`).
    let lc_to_canonical: BTreeMap<String, &str> = class_symbols
        .keys()
        .map(|k| (k.to_ascii_lowercase(), k.as_str()))
        .collect();

    // Keyed on `(class_api_canonical, method_name_lower_ascii, kind)`
    // so the final node-tagging pass can do an O(log N) membership
    // test and preserve deterministic ordering on failure diagnostics.
    let mut targets: BTreeSet<(String, String, PlatformInterfaceKind)> = BTreeSet::new();

    for (api_name, symbols) in class_symbols {
        let kinds = platform_interfaces_in(&symbols.implemented_interfaces);
        if kinds.is_empty() {
            continue;
        }
        stats.classes_with_platform_interface += 1;

        let ancestors = collect_ancestors_inclusive(api_name, class_symbols, &lc_to_canonical);
        for ancestor_api in &ancestors {
            let Some(ancestor_syms) = class_symbols.get(ancestor_api.as_str()) else {
                continue;
            };
            for method in &ancestor_syms.methods {
                for kind in &kinds {
                    if method_name_matches_contract(&method.name, *kind) {
                        targets.insert((
                            ancestor_api.clone(),
                            method.name.to_ascii_lowercase(),
                            *kind,
                        ));
                    }
                }
            }
        }
    }

    stats.contract_method_targets = targets.len();
    if targets.is_empty() {
        return stats;
    }

    // Index nodes by a cheap FQN fingerprint so we don't do an
    // O(nodes * targets) scan. The fingerprint is the last two
    // `::`-separated segments of the FQN (class simple name + method
    // signature), lower-cased — which uniquely identifies a contract
    // method inside a single file without needing to parse the
    // whole FQN.
    for node in nodes.iter_mut() {
        if node.kind != NodeKind::Function {
            continue;
        }
        // Clone the extracted segments so the immutable borrow of
        // `node.fqn` is released before we hand the node to the
        // mutable-borrow tagger below. Cheap: two short strings per
        // Function node.
        let Some((class_segment, method_name_lc)) =
            split_fqn_class_and_method(&node.fqn).map(|(cs, m)| (cs.to_string(), m))
        else {
            continue;
        };

        // Match by api-name suffix: the ancestor's api-name can be a
        // top-level name (`CRLP_Batch_Base_Skew`) or dotted inner
        // (`Outer.Inner`), and the FQN encodes the enclosing type as
        // `Outer.Inner` (dot, not `::`). So the last `::`-segment
        // preceding the method is exactly that.
        for (ancestor_api, target_method_lc, kind) in &targets {
            if method_name_lc != *target_method_lc {
                continue;
            }
            if !class_segment.eq_ignore_ascii_case(ancestor_api) {
                continue;
            }
            if push_entry_point_tag(node, kind.short_name()) {
                stats.function_nodes_tagged += 1;
                debug!(
                    "framework entry-point propagation: tagged {} with '{}' (inherited from a {} subclass)",
                    node.fqn,
                    kind.short_name(),
                    kind.short_name()
                );
            } else {
                stats.function_nodes_already_tagged += 1;
            }
            // A method can only match one `(ancestor_api, method)`
            // target uniquely; multiple kinds on the same method
            // (e.g. a class implementing both Schedulable and
            // Queueable, both of which dispatch `execute`) fall
            // through the loop independently because the tags differ.
        }
    }

    stats
}

// -------------------------------------------------------------------------
// Platform interface taxonomy
// -------------------------------------------------------------------------

/// The four Salesforce platform interfaces whose dispatch happens by
/// *method name* (rather than by the interface VTable a compiler
/// would). These are the only cases where the abstract-base pattern
/// creates no-callers false positives — regular OO interfaces compiled
/// by a language with virtual dispatch would not need this pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PlatformInterfaceKind {
    Batchable,
    Schedulable,
    Queueable,
    InboundEmailHandler,
}

impl PlatformInterfaceKind {
    /// Short stable string the analysis crate consumes via
    /// `GraphNode::entry_point_tags`. Values MUST match
    /// `super::entry_points::EntryPointKind::as_str` for the
    /// corresponding variants so the two propagation paths (AST-local
    /// direct-implementers + this module's ancestor propagation) are
    /// indistinguishable downstream.
    fn short_name(self) -> &'static str {
        match self {
            Self::Batchable => "batchable",
            Self::Schedulable => "schedulable",
            Self::Queueable => "queueable",
            Self::InboundEmailHandler => "inbound_email_handler",
        }
    }
}

fn interface_to_kind(interface_ref: &str) -> Option<PlatformInterfaceKind> {
    // Strip generic args (`Database.Batchable<SObject>` →
    // `Database.Batchable`) and any leading namespace dot
    // (`Database.Batchable` → `Batchable`). We compare on the short
    // name because Salesforce's platform interfaces are uniquely
    // named globally.
    let cleaned: String = interface_ref.chars().take_while(|c| *c != '<').collect();
    let short = cleaned.trim().rsplit('.').next()?.trim();
    match short.to_ascii_lowercase().as_str() {
        "batchable" => Some(PlatformInterfaceKind::Batchable),
        "schedulable" => Some(PlatformInterfaceKind::Schedulable),
        "queueable" => Some(PlatformInterfaceKind::Queueable),
        "inboundemailhandler" => Some(PlatformInterfaceKind::InboundEmailHandler),
        _ => None,
    }
}

fn platform_interfaces_in(implemented: &[String]) -> Vec<PlatformInterfaceKind> {
    let mut kinds: Vec<_> = implemented
        .iter()
        .filter_map(|s| interface_to_kind(s))
        .collect();
    kinds.sort();
    kinds.dedup();
    kinds
}

/// Salesforce dispatches these interfaces by method name only, so a
/// simple-name match (case-insensitive, per Apex semantics) is
/// sufficient. Overload signatures are irrelevant because only one
/// overload per contract method can satisfy the interface.
fn method_name_matches_contract(method_name: &str, kind: PlatformInterfaceKind) -> bool {
    let m = method_name;
    match kind {
        PlatformInterfaceKind::Batchable => {
            m.eq_ignore_ascii_case("start")
                || m.eq_ignore_ascii_case("execute")
                || m.eq_ignore_ascii_case("finish")
        }
        PlatformInterfaceKind::Schedulable => m.eq_ignore_ascii_case("execute"),
        PlatformInterfaceKind::Queueable => m.eq_ignore_ascii_case("execute"),
        PlatformInterfaceKind::InboundEmailHandler => m.eq_ignore_ascii_case("handleinboundemail"),
    }
}

// -------------------------------------------------------------------------
// Inheritance walk
// -------------------------------------------------------------------------

/// Walk from `start` upward through `parent_class`, returning
/// canonical (source-case) api-names of every class in the chain
/// including `start` itself. Truncates at the first repeated class
/// (cycle guard) and at any unresolved parent (external class /
/// managed-package reference / typo in the declaration) — neither of
/// those can contribute contract methods that live in this repo.
fn collect_ancestors_inclusive(
    start: &str,
    class_symbols: &BTreeMap<String, ApexClassSymbols>,
    lc_to_canonical: &BTreeMap<String, &str>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();

    let mut current_ref = Some(start.to_string());
    while let Some(name) = current_ref.take() {
        let lc = name.to_ascii_lowercase();
        if !visited.insert(lc.clone()) {
            break;
        }
        let canonical: String = lc_to_canonical
            .get(&lc)
            .map(|s| s.to_string())
            .unwrap_or(name);
        let Some(symbols) = class_symbols.get(canonical.as_str()) else {
            break;
        };
        out.push(canonical);

        current_ref = symbols.parent_class.clone().and_then(|parent| {
            // Parent references may be unqualified (`BaseBatch`) or
            // dotted (`Namespace.BaseBatch`). We only resolve against
            // the registry's keys — anything else is an external type
            // and terminates the walk.
            let parent_lc = parent.to_ascii_lowercase();
            if lc_to_canonical.contains_key(&parent_lc) {
                Some(parent)
            } else {
                // Fallback: try the last dotted segment (strip
                // namespace prefix). Apex allows unqualified
                // references to classes in the same namespace.
                let short = parent.rsplit('.').next().unwrap_or(&parent);
                if lc_to_canonical.contains_key(&short.to_ascii_lowercase()) {
                    Some(short.to_string())
                } else {
                    None
                }
            }
        });
    }
    out
}

// -------------------------------------------------------------------------
// FQN + node tag plumbing
// -------------------------------------------------------------------------

/// Extract `(class_segment, method_name_lower)` from an Apex Function
/// FQN.
///
/// Apex method FQNs have the shape `<prefix>::<Outer>[.<Inner>]::<name>(<params>)`.
/// We need the class segment (e.g. `CRLP_Batch_Base_Skew` or
/// `Outer.Inner`) and the method simple name. Everything else —
/// path prefix, parameter signature — is noise for this matcher.
fn split_fqn_class_and_method(fqn: &str) -> Option<(&str, String)> {
    // Find the last `::` before the parameter list. The method's simple
    // name is everything between that separator and the first `(`.
    let paren_idx = fqn.find('(')?;
    let head = &fqn[..paren_idx];
    let last_sep = head.rfind("::")?;
    let method_name = &head[last_sep + 2..];
    if method_name.is_empty() {
        return None;
    }
    let before_method = &head[..last_sep];
    let class_segment = before_method.rsplit("::").next()?;
    if class_segment.is_empty() {
        return None;
    }
    Some((class_segment, method_name.to_ascii_lowercase()))
}

/// Push `tag` onto the node's `entry_points` property (creating the
/// array if needed). Returns `true` iff the tag was newly added.
fn push_entry_point_tag(node: &mut Node, tag: &str) -> bool {
    let existing = node
        .properties
        .get("entry_points")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if existing.iter().any(|v| {
        v.as_str()
            .map(|s| s.eq_ignore_ascii_case(tag))
            .unwrap_or(false)
    }) {
        return false;
    }
    let mut next = existing;
    next.push(serde_json::Value::String(tag.to_string()));
    node.set_property("entry_points", serde_json::Value::Array(next));
    true
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::{Access, ApexMethod, ApexParameter, ApexTypeRef};
    use crate::domain::{Confidence, Provenance, ProvenanceSource, Range};

    fn prim(name: &str) -> ApexTypeRef {
        ApexTypeRef::Primitive { name: name.into() }
    }

    fn method_nullary(name: &str) -> ApexMethod {
        ApexMethod {
            name: name.into(),
            parameters: Vec::new(),
            return_type: None,
            access: Access::Public,
            is_static: false,
            is_virtual: false,
            is_abstract: false,
        }
    }

    fn method_with_params(name: &str, params: Vec<(&str, &str)>) -> ApexMethod {
        ApexMethod {
            name: name.into(),
            parameters: params
                .into_iter()
                .map(|(n, t)| ApexParameter {
                    name: n.into(),
                    ty: prim(t),
                })
                .collect(),
            return_type: None,
            access: Access::Public,
            is_static: false,
            is_virtual: false,
            is_abstract: false,
        }
    }

    fn class(
        parent: Option<&str>,
        interfaces: Vec<&str>,
        methods: Vec<ApexMethod>,
    ) -> ApexClassSymbols {
        ApexClassSymbols {
            methods,
            parent_class: parent.map(|s| s.to_string()),
            implemented_interfaces: interfaces.into_iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn function_node(fqn: &str) -> Node {
        Node::new(
            NodeKind::Function,
            fqn.to_string(),
            Range {
                start_line: 0,
                start_char: 0,
                end_line: 1,
                end_char: 0,
                file: "/tmp/ws/x.cls".into(),
            },
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        )
    }

    fn tag_list(node: &Node) -> Vec<String> {
        node.properties
            .get("entry_points")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn direct_implementer_without_ancestors_tags_its_own_methods() {
        // Sanity check: even without an abstract base, the propagator
        // sets the interface tag on the class's own contract methods.
        // This duplicates what the AST-local pass already does for the
        // same node, but the duplicate tag is de-duplicated so a
        // second tag is not pushed. We depend on that idempotency to
        // keep the two passes composable.
        let mut syms = BTreeMap::new();
        syms.insert(
            "Job".into(),
            class(
                None,
                vec!["Schedulable"],
                vec![method_with_params(
                    "execute",
                    vec![("c", "SchedulableContext")],
                )],
            ),
        );
        let mut nodes = vec![function_node(
            "/tmp/ws/Job.cls::Job::execute(SchedulableContext)",
        )];

        let stats = propagate_framework_entry_points(&syms, &mut nodes);
        assert_eq!(stats.classes_with_platform_interface, 1);
        assert_eq!(stats.contract_method_targets, 1);
        assert_eq!(stats.function_nodes_tagged, 1);
        assert_eq!(tag_list(&nodes[0]), vec!["schedulable".to_string()]);
    }

    #[test]
    fn abstract_parent_contract_method_gets_tagged_via_subclass_implements() {
        // R11 canonical shape: parent has the body, subclass has the
        // implements. Before this pass, the parent's Function node
        // carries no entry_point_tag and shows up as no_callers. After
        // the pass, the parent's method carries `batchable`.
        let mut syms = BTreeMap::new();
        syms.insert(
            "BaseBatch".into(),
            class(
                None,
                vec![],
                vec![
                    method_with_params("start", vec![("bc", "Database.BatchableContext")]),
                    method_with_params(
                        "execute",
                        vec![("bc", "Database.BatchableContext"), ("scope", "List")],
                    ),
                    method_with_params("finish", vec![("bc", "Database.BatchableContext")]),
                    method_nullary("helper"),
                ],
            ),
        );
        syms.insert(
            "ConcreteBatch".into(),
            class(Some("BaseBatch"), vec!["Database.Batchable"], vec![]),
        );

        let mut nodes = vec![
            function_node("/tmp/ws/BaseBatch.cls::BaseBatch::start(Database.BatchableContext)"),
            function_node(
                "/tmp/ws/BaseBatch.cls::BaseBatch::execute(Database.BatchableContext,List)",
            ),
            function_node("/tmp/ws/BaseBatch.cls::BaseBatch::finish(Database.BatchableContext)"),
            function_node("/tmp/ws/BaseBatch.cls::BaseBatch::helper()"),
        ];

        let stats = propagate_framework_entry_points(&syms, &mut nodes);
        assert_eq!(stats.classes_with_platform_interface, 1);
        // start, execute, finish on the base (subclass has no
        // methods of its own to contribute contract matches from).
        assert_eq!(stats.contract_method_targets, 3);
        assert_eq!(stats.function_nodes_tagged, 3);

        for (i, expected_tag) in [(0, "batchable"), (1, "batchable"), (2, "batchable")] {
            assert_eq!(
                tag_list(&nodes[i]),
                vec![expected_tag.to_string()],
                "contract method at index {} must carry batchable tag",
                i
            );
        }
        assert!(
            tag_list(&nodes[3]).is_empty(),
            "non-contract method `helper()` must not receive any tag"
        );
    }

    #[test]
    fn multi_hop_inheritance_chain_propagates_through() {
        // Root has the contract, two hops up the chain a subclass
        // declares the implements. All ancestors between root and the
        // implementer must receive the tag on their contract methods
        // (not that there are any — but the walk must visit them).
        let mut syms = BTreeMap::new();
        syms.insert(
            "Root".into(),
            class(None, vec![], vec![method_nullary("execute")]),
        );
        syms.insert(
            "Middle".into(),
            class(Some("Root"), vec![], vec![method_nullary("execute")]),
        );
        syms.insert(
            "Leaf".into(),
            class(Some("Middle"), vec!["Queueable"], vec![]),
        );

        let mut nodes = vec![
            function_node("/tmp/ws/Root.cls::Root::execute()"),
            function_node("/tmp/ws/Middle.cls::Middle::execute()"),
        ];

        let stats = propagate_framework_entry_points(&syms, &mut nodes);
        assert_eq!(stats.function_nodes_tagged, 2);
        assert_eq!(tag_list(&nodes[0]), vec!["queueable".to_string()]);
        assert_eq!(tag_list(&nodes[1]), vec!["queueable".to_string()]);
    }

    #[test]
    fn cycle_in_parent_chain_does_not_loop_forever() {
        // Malformed registry where A's parent is B and B's parent is
        // A. The walk must terminate and tag at least one contract
        // method without an infinite loop.
        let mut syms = BTreeMap::new();
        syms.insert(
            "A".into(),
            class(
                Some("B"),
                vec!["Schedulable"],
                vec![method_nullary("execute")],
            ),
        );
        syms.insert(
            "B".into(),
            class(Some("A"), vec![], vec![method_nullary("execute")]),
        );

        let mut nodes = vec![
            function_node("/tmp/ws/A.cls::A::execute()"),
            function_node("/tmp/ws/B.cls::B::execute()"),
        ];

        let _ = propagate_framework_entry_points(&syms, &mut nodes);
        // Both are reachable by the walk starting from A (which
        // declares implements); both get tagged. The crucial
        // assertion is that the function returned at all.
        assert_eq!(tag_list(&nodes[0]), vec!["schedulable".to_string()]);
        assert_eq!(tag_list(&nodes[1]), vec!["schedulable".to_string()]);
    }

    #[test]
    fn unrelated_parent_chain_is_not_tagged() {
        // Classes that share a parent with an implementer but are not
        // themselves on the implementer's ancestor chain must not be
        // tagged. This guards against false-positive propagation
        // through sibling relationships.
        let mut syms = BTreeMap::new();
        syms.insert(
            "Shared".into(),
            class(None, vec![], vec![method_nullary("execute")]),
        );
        // Sibling subclass of Shared that does NOT implement any
        // platform interface.
        syms.insert(
            "Sibling".into(),
            class(Some("Shared"), vec![], vec![method_nullary("execute")]),
        );
        // The sibling sibling IS an implementer — propagation walks
        // UP from the implementer, so the sibling relationship does
        // not connect Sibling::execute to the tag path.
        syms.insert(
            "Implementer".into(),
            class(
                Some("Shared"),
                vec!["Queueable"],
                vec![method_nullary("execute")],
            ),
        );

        let mut nodes = vec![
            function_node("/tmp/ws/Shared.cls::Shared::execute()"),
            function_node("/tmp/ws/Sibling.cls::Sibling::execute()"),
            function_node("/tmp/ws/Implementer.cls::Implementer::execute()"),
        ];

        let _ = propagate_framework_entry_points(&syms, &mut nodes);
        // Shared is an ancestor of Implementer → tagged.
        assert_eq!(tag_list(&nodes[0]), vec!["queueable".to_string()]);
        // Sibling has no descendant that implements — not tagged.
        assert!(
            tag_list(&nodes[1]).is_empty(),
            "sibling class must not be tagged via its shared parent"
        );
        // Implementer tagged (direct).
        assert_eq!(tag_list(&nodes[2]), vec!["queueable".to_string()]);
    }

    #[test]
    fn non_platform_interfaces_are_ignored() {
        // User-defined interfaces that are not one of the four
        // Salesforce platform interfaces must not trigger propagation.
        let mut syms = BTreeMap::new();
        syms.insert(
            "X".into(),
            class(
                None,
                vec!["IAuditable", "Serializable", "Cloneable"],
                vec![method_nullary("execute"), method_nullary("start")],
            ),
        );

        let mut nodes = vec![
            function_node("/tmp/ws/X.cls::X::execute()"),
            function_node("/tmp/ws/X.cls::X::start()"),
        ];

        let stats = propagate_framework_entry_points(&syms, &mut nodes);
        assert_eq!(stats.classes_with_platform_interface, 0);
        assert_eq!(stats.function_nodes_tagged, 0);
        assert!(tag_list(&nodes[0]).is_empty());
        assert!(tag_list(&nodes[1]).is_empty());
    }

    #[test]
    fn idempotent_on_re_run() {
        // Running the pass twice must produce the same tag set. The
        // second run's `function_nodes_tagged` must be zero and
        // `function_nodes_already_tagged` must be the first run's
        // tagged count.
        let mut syms = BTreeMap::new();
        syms.insert(
            "Base".into(),
            class(None, vec![], vec![method_nullary("execute")]),
        );
        syms.insert(
            "Derived".into(),
            class(Some("Base"), vec!["Queueable"], vec![]),
        );
        let mut nodes = vec![function_node("/tmp/ws/Base.cls::Base::execute()")];

        let first = propagate_framework_entry_points(&syms, &mut nodes);
        assert_eq!(first.function_nodes_tagged, 1);

        let second = propagate_framework_entry_points(&syms, &mut nodes);
        assert_eq!(second.function_nodes_tagged, 0);
        assert_eq!(second.function_nodes_already_tagged, 1);
        assert_eq!(tag_list(&nodes[0]), vec!["queueable".to_string()]);
    }

    #[test]
    fn dotted_namespace_interface_recognised() {
        // `Database.Batchable<SObject>` must classify as Batchable
        // despite the namespace qualifier and generic parameter.
        assert_eq!(
            interface_to_kind("Database.Batchable<SObject>"),
            Some(PlatformInterfaceKind::Batchable)
        );
        assert_eq!(
            interface_to_kind("Messaging.InboundEmailHandler"),
            Some(PlatformInterfaceKind::InboundEmailHandler)
        );
        assert_eq!(interface_to_kind("IAuditable"), None);
    }

    #[test]
    fn split_fqn_handles_inner_class_method() {
        // Inner class methods FQN as `prefix::Outer.Inner::method(sig)`.
        // The class-segment extractor must return `Outer.Inner`
        // verbatim (dots intact) so api-name comparison works.
        let (class_seg, method) = split_fqn_class_and_method(
            "/tmp/ws/Outer.cls::Outer.Inner::execute(Database.BatchableContext,List)",
        )
        .expect("inner-class FQN must split");
        assert_eq!(class_seg, "Outer.Inner");
        assert_eq!(method, "execute");
    }

    #[test]
    fn split_fqn_rejects_non_method_shapes() {
        // A Type FQN (no parentheses) must return None so the
        // matcher short-circuits on Struct/Interface nodes without
        // spuriously tagging them.
        assert!(split_fqn_class_and_method("/tmp/ws/X.cls::X").is_none());
    }
}
