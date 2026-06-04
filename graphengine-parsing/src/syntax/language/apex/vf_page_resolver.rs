//! Visualforce page → Apex method dispatch (TR-A.5).
//!
//! # Role in the pipeline
//!
//! For each `.page` file discovered by the SFDX layout walker, this
//! module synthesises:
//!
//! 1. A `Struct` node for the page container — FQN
//!    `<workspace_path>::<PageName>`, tagged `subtype = "vf_page"`.
//!    This mirrors the `trigger_declaration`-level node that Apex
//!    triggers get via the tree-sitter extractor; Phase C TR-C.3 will
//!    attach further VF surfaces (rerender, actionFunction, text-node
//!    expressions) to this container.
//! 2. A `Function` node for the page body — FQN
//!    `<workspace_path>::<PageName>::__vf_page__()`, tagged
//!    `synthetic = true`, `synthetic_kind = "apex_vf_page_body"`.
//!    Covers the entire source range of the page so every synthesised
//!    call site inside the page spatially resolves to this function as
//!    its enclosing caller.
//! 3. One `Contains` edge from the container Struct to the synthetic
//!    Function.
//! 4. For each `{!identifier}` binding whose identifier names a method
//!    on the page's controller or one of its extensions (in Salesforce's
//!    actual resolution order — controller first, then extensions in
//!    declared order, first match wins), a `CallSite` is appended to
//!    the pipeline's `SyntaxResults::call_sites` with `function_name =
//!    "<ResolvedClass>::<method>"`. The existing semantic-resolution
//!    stage binds these CallSites to real Apex Function nodes via the
//!    engine's FQN-suffix strategy (Medium confidence — see
//!    `ResolutionStrategy::FqnSuffix` in `lsp::call_resolver`).
//!
//! # Why emit CallSites instead of pre-resolved Call edges
//!
//! The target Apex Function node is extracted by the tree-sitter pass
//! with a location-dependent node id (`sha256(fqn, range)`). This
//! module only sees `ApexClassSymbols` (class_symbols JSON), which
//! carries the method's name and parameter types but not its node's
//! `Range`. Reconstructing the target node id locally would either
//! require a second pass over the `.cls` AST or carrying ranges inside
//! `ApexClassSymbols` (breaking the pure-data contract of the
//! type oracle). Funneling the call through the existing resolver lets
//! the spatial index do the node-id lookup uniformly.
//!
//! The loss of control that accepting suffix-matched resolution implies
//! is bounded: VF candidate classes have already been narrowed to one
//! (controller-first, then extensions, first method-name match wins),
//! so we emit `ClassName::methodName` and rely on the resolver to find
//! the Function whose FQN ends with exactly that. For ambiguous class
//! names (two classes with the same simple name in different managed
//! namespaces), the resolver picks the first — this matches the
//! "first match wins" contract of §7.2 item 3.

use std::collections::BTreeMap;
use std::path::Path;

use crate::domain::apex::class_symbols::ApexClassSymbols;
use crate::domain::{Confidence, Edge, Node, NodeKind, Provenance, ProvenanceSource, Range};

use super::fqn::{build_vf_page_body_fqn, build_vf_page_container_fqn};
use super::vf_page_reader::{VfBinding, VfPage};

/// Outcome of resolving a single `.page` file. Each field is
/// independently consumable by the pipeline hook — see
/// `syntax::language::apex::vf_extraction_stage` (post-T5 location;
/// previously at `application::use_cases::parse_repo::pipeline::vf_extraction`
/// before the orchestrator trait-method collapse).
#[derive(Debug, Default, Clone)]
pub struct VfPageResolution {
    /// The synthetic Struct (container) and Function (body) nodes.
    /// Emitted even when no bindings resolve, because the container
    /// nodes are needed for Phase C to attach further surfaces and
    /// for report consumers that count `__vf_page__` presence.
    pub synthetic_nodes: Vec<Node>,
    /// Exactly one `Contains` edge: container → `__vf_page__`. In a
    /// `Vec` (not a scalar) so the caller can merge with other pages'
    /// edges via a single extend.
    pub synthetic_edges: Vec<Edge>,
    /// One entry per resolved binding. The pipeline hook pushes each
    /// into `syntax_results.call_sites` so the existing semantic
    /// resolver binds them to real Apex Function nodes.
    pub resolved_bindings: Vec<ResolvedBinding>,
    /// Bindings whose identifier didn't match any method on the
    /// controller or any extension. Retained for diagnostics only;
    /// the pipeline hook does not forward these anywhere.
    pub unresolved_bindings: Vec<UnresolvedBinding>,
}

/// A successfully-resolved VF → Apex binding. The tuple `(class,
/// method)` uniquely identifies the target before node-id hashing;
/// `caller_fqn` + `caller_range` identify the synthetic `__vf_page__`
/// caller the pipeline hook builds a `CallSite` around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBinding {
    pub target_class: String,
    pub target_method: String,
    pub caller_fqn: String,
    pub caller_range: Range,
    /// Unique sub-range inside the synthetic function. Derived from
    /// the binding's per-page ordinal so two bindings to the same
    /// method produce distinct CallSite node ids.
    pub call_site_location: Range,
    /// Propagated from `VfBinding::is_invocation` for downstream
    /// diagnostics; the resolver itself treats invocation and plain
    /// reference forms identically.
    pub is_invocation: bool,
}

/// A binding that named no method on any candidate class. Reported
/// only for tooling / tests; the pipeline hook drops these silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedBinding {
    pub identifier: String,
    pub is_invocation: bool,
    pub reason: UnresolvedReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// Page has neither a controller nor any extensions.
    NoCandidateClasses,
    /// At least one candidate class was declared but none is known to
    /// the symbol table (missing `.cls` file, managed-package class,
    /// etc.).
    NoKnownCandidateClass,
    /// Candidate classes exist but none declares a method with the
    /// binding's identifier.
    NoMatchingMethod,
}

/// Resolve one page. Idempotent and pure — no I/O. `class_symbols` is
/// the post-syntax-extraction view of every known user-declared Apex
/// class keyed by case-preserving api-name. `workspace_root` is passed
/// through to the FQN builders so the synthetic node FQNs carry the
/// same path prefix as real Apex types in the same repo.
pub fn resolve_vf_page(
    page: &VfPage,
    class_symbols: &BTreeMap<String, ApexClassSymbols>,
    workspace_root: Option<&str>,
) -> VfPageResolution {
    let file_path = path_str(&page.source_path);

    let container_fqn = build_vf_page_container_fqn(&page.name, &file_path, workspace_root);
    let body_fqn = build_vf_page_body_fqn(&page.name, &file_path, workspace_root);

    // The synthetic container's range has to be distinct from the
    // function's range — otherwise their node ids collide (both hash
    // from fqn+range). Container covers the notional "declaration
    // line"; body covers the full document. Values are arbitrary but
    // deterministic and non-overlapping.
    let container_range = synthetic_range(&file_path, 0, 0, 0, 1);
    let body_range = synthetic_range(&file_path, 1, 0, SYNTHETIC_BODY_END_LINE, 0);

    let provenance = Provenance::new(ProvenanceSource::Heuristic, Confidence::Medium);
    let mut container = Node::new(
        NodeKind::Struct,
        container_fqn.clone(),
        container_range,
        provenance,
    );
    container.set_property("synthetic", true);
    container.set_property("synthetic_kind", "apex_vf_page_container");
    container.set_property("subtype", "vf_page");
    container.set_property("vf_page_name", page.name.clone());
    if let Some(ref ctrl) = page.controller {
        container.set_property("vf_controller", ctrl.clone());
    }
    if !page.extensions.is_empty() {
        container.set_property(
            "vf_extensions",
            serde_json::Value::Array(
                page.extensions
                    .iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect(),
            ),
        );
    }

    let mut body = Node::new(
        NodeKind::Function,
        body_fqn.clone(),
        body_range.clone(),
        provenance,
    );
    body.set_property("synthetic", true);
    body.set_property("synthetic_kind", "apex_vf_page_body");
    body.set_property("parent_container_id", container.id.clone());
    body.set_property("vf_page_name", page.name.clone());
    // The Salesforce runtime — not in-repo Apex code — renders a
    // Visualforce page, which is what invokes every `{!binding}`
    // expression inside the page. The synthetic `__vf_page__()`
    // function is the in-graph caller for those bindings, so by
    // construction it has no in-repo callers of its own. Without an
    // entry-point marker it would appear in dead-code analysis as
    // `no_callers` for every `.page` in the repo — a Phase-A
    // self-inflicted false positive surfaced by the Round 5
    // hand-audit (sample 9, `ALLO_RollupBTN::__vf_page__()`).
    // `is_attribute_invoked` is the existing "framework invokes this
    // from outside the codebase" flag; classifying here mirrors the
    // treatment of `@AuraEnabled` / `@RemoteAction` entry points.
    body.set_property("is_attribute_invoked", true);
    body.set_property("entry_point_reason", "visualforce_page_body");

    let contains_edge = Edge::contains(container.id.clone(), body.id.clone(), provenance);

    let mut resolution = VfPageResolution {
        synthetic_nodes: vec![container, body],
        synthetic_edges: vec![contains_edge],
        resolved_bindings: Vec::new(),
        unresolved_bindings: Vec::new(),
    };

    // The candidate chain: controller first, then extensions in
    // declared order. Short-circuit early if the page declares none
    // so we don't touch the symbol table at all.
    let candidates: Vec<&str> = page.candidate_chain().collect();
    if candidates.is_empty() {
        for b in &page.bindings {
            resolution.unresolved_bindings.push(UnresolvedBinding {
                identifier: b.identifier.clone(),
                is_invocation: b.is_invocation,
                reason: UnresolvedReason::NoCandidateClasses,
            });
        }
        return resolution;
    }

    let known_candidates: Vec<(&str, &ApexClassSymbols)> = candidates
        .iter()
        .filter_map(|class_name| find_symbols_case_insensitive(class_symbols, class_name))
        .collect();

    // No symbol table rows at all for any candidate → every binding
    // on this page is unresolved for the same reason. Surface that
    // uniformly instead of running the per-binding loop just to
    // attribute the same diagnostic to each binding separately.
    if known_candidates.is_empty() {
        for b in &page.bindings {
            resolution.unresolved_bindings.push(UnresolvedBinding {
                identifier: b.identifier.clone(),
                is_invocation: b.is_invocation,
                reason: UnresolvedReason::NoKnownCandidateClass,
            });
        }
        return resolution;
    }

    for (ord, binding) in page.bindings.iter().enumerate() {
        match find_first_matching_class(&known_candidates, &binding.identifier) {
            Some((class_name, canonical_method_name)) => {
                resolution.resolved_bindings.push(ResolvedBinding {
                    target_class: class_name.to_string(),
                    target_method: canonical_method_name,
                    caller_fqn: body_fqn.clone(),
                    caller_range: body_range.clone(),
                    call_site_location: binding_call_site_range(&file_path, ord, binding),
                    is_invocation: binding.is_invocation,
                });
            }
            None => {
                resolution.unresolved_bindings.push(UnresolvedBinding {
                    identifier: binding.identifier.clone(),
                    is_invocation: binding.is_invocation,
                    reason: UnresolvedReason::NoMatchingMethod,
                });
            }
        }
    }

    resolution
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Upper bound for the synthetic body's end-line. Any CallSite the
/// resolver emits for this page sits on a line in `[1,
/// SYNTHETIC_BODY_END_LINE)` so its `find_containing_function` lookup
/// resolves to this body. Set to `u32::MAX - 1` so there is effectively
/// no ceiling on how many bindings one page can carry.
const SYNTHETIC_BODY_END_LINE: u32 = u32::MAX - 1;

fn synthetic_range(
    file_path: &str,
    start_line: u32,
    start_char: u32,
    end_line: u32,
    end_char: u32,
) -> Range {
    Range {
        start_line,
        start_char,
        end_line,
        end_char,
        file: file_path.to_string(),
    }
}

/// Build a unique Range for one VF binding's CallSite. The ordinal
/// (per-page declaration order) is used as the line number so two
/// bindings on the same page — even if they point to the same method
/// — produce distinct CallSite locations, and therefore distinct node
/// ids in downstream persistence.
fn binding_call_site_range(file_path: &str, ord: usize, _binding: &VfBinding) -> Range {
    // Line numbers are 1-indexed in the Range domain; ord is
    // 0-indexed. +1 keeps the first binding on line 1 — which sits
    // inside the synthetic body's [1, u32::MAX - 1) range.
    //
    // For the astronomical case of >u32::MAX bindings on one page,
    // saturate at SYNTHETIC_BODY_END_LINE - 1 rather than wrap.
    let raw = ord.saturating_add(1);
    let line = u32::try_from(raw)
        .unwrap_or(u32::MAX - 2)
        .min(SYNTHETIC_BODY_END_LINE - 1);
    Range {
        start_line: line,
        start_char: 0,
        end_line: line,
        end_char: 1,
        file: file_path.to_string(),
    }
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Case-insensitive lookup into the class-symbols map. Apex identifiers
/// are case-insensitive, but the map is keyed by case-preserving
/// api-name, so the caller's declared form (e.g. `UTIL_JobProgress_CTRL`
/// vs `util_jobprogress_ctrl`) may not match byte-for-byte.
fn find_symbols_case_insensitive<'a>(
    map: &'a BTreeMap<String, ApexClassSymbols>,
    name: &str,
) -> Option<(&'a str, &'a ApexClassSymbols)> {
    let target = name.trim();
    // Fast path: exact hit by ref — avoids the linear scan below on
    // the common case where the user's declared class name matches
    // source case exactly.
    if let Some((k, v)) = map.get_key_value(target) {
        return Some((k.as_str(), v));
    }
    map.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(target))
        .map(|(k, v)| (k.as_str(), v))
}

/// Walk candidates in the given order, returning the first class + the
/// method's canonical (source-case) name on a match. Apex methods are
/// resolved by simple name only — VF bindings carry no parameter-type
/// information so overload disambiguation can't happen here.
fn find_first_matching_class(
    candidates: &[(&str, &ApexClassSymbols)],
    binding_identifier: &str,
) -> Option<(String, String)> {
    for (class_name, symbols) in candidates {
        if let Some(method) = symbols.methods_named(binding_identifier).next() {
            return Some((class_name.to_string(), method.name.clone()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::{ApexMethod, ApexParameter, ApexTypeRef};
    use std::path::PathBuf;

    fn prim(name: &str) -> ApexTypeRef {
        ApexTypeRef::Primitive { name: name.into() }
    }

    fn method(name: &str, params: Vec<(&str, &str)>) -> ApexMethod {
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
            access: crate::domain::apex::class_symbols::Access::Public,
            is_static: false,
            is_virtual: false,
            is_abstract: false,
        }
    }

    fn page_with(
        name: &str,
        controller: Option<&str>,
        extensions: &[&str],
        bindings: &[(&str, bool)],
    ) -> VfPage {
        VfPage {
            name: name.into(),
            source_path: PathBuf::from(format!("/tmp/ws/pages/{name}.page")),
            controller: controller.map(|s| s.to_string()),
            extensions: extensions.iter().map(|s| s.to_string()).collect(),
            bindings: bindings
                .iter()
                .enumerate()
                .map(|(i, (id, inv))| VfBinding {
                    identifier: (*id).into(),
                    is_invocation: *inv,
                    offset: (i as u64) * 16,
                })
                .collect(),
        }
    }

    fn symbols_map(entries: Vec<(&str, Vec<ApexMethod>)>) -> BTreeMap<String, ApexClassSymbols> {
        entries
            .into_iter()
            .map(|(name, methods)| {
                (
                    name.to_string(),
                    ApexClassSymbols {
                        fields: Vec::new(),
                        methods,
                        constructors: Vec::new(),
                        inner_classes: Vec::new(),
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    #[test]
    fn synthesises_container_and_body_even_with_no_bindings() {
        let page = page_with("Empty", Some("Ctrl"), &[], &[]);
        let syms = symbols_map(vec![("Ctrl", Vec::new())]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        assert_eq!(out.synthetic_nodes.len(), 2);
        assert_eq!(out.synthetic_edges.len(), 1);
        assert!(out.resolved_bindings.is_empty());
        assert!(out.unresolved_bindings.is_empty());
    }

    #[test]
    fn controller_wins_over_extension_on_same_method_name() {
        let page = page_with("P", Some("Ctrl"), &["Ext"], &[("save", false)]);
        let syms = symbols_map(vec![
            ("Ctrl", vec![method("save", vec![])]),
            ("Ext", vec![method("save", vec![])]),
        ]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        assert_eq!(out.resolved_bindings.len(), 1);
        assert_eq!(out.resolved_bindings[0].target_class, "Ctrl");
        assert_eq!(out.resolved_bindings[0].target_method, "save");
    }

    #[test]
    fn falls_through_to_extension_when_controller_lacks_method() {
        let page = page_with("P", Some("Ctrl"), &["Ext"], &[("save", true)]);
        let syms = symbols_map(vec![
            ("Ctrl", vec![method("other", vec![])]),
            ("Ext", vec![method("save", vec![])]),
        ]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        assert_eq!(out.resolved_bindings.len(), 1);
        assert_eq!(out.resolved_bindings[0].target_class, "Ext");
        assert!(out.resolved_bindings[0].is_invocation);
    }

    #[test]
    fn respects_declared_extension_order_on_first_match() {
        let page = page_with("P", None, &["A", "B"], &[("m", false)]);
        let syms = symbols_map(vec![
            ("A", vec![method("other", vec![])]),
            ("B", vec![method("m", vec![])]),
        ]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        assert_eq!(out.resolved_bindings.len(), 1);
        assert_eq!(out.resolved_bindings[0].target_class, "B");
    }

    #[test]
    fn bindings_on_page_without_controller_or_extensions_are_unresolved() {
        let page = page_with("P", None, &[], &[("save", false)]);
        let syms = symbols_map(vec![("Unused", vec![method("save", vec![])])]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        assert_eq!(out.resolved_bindings.len(), 0);
        assert_eq!(out.unresolved_bindings.len(), 1);
        assert_eq!(
            out.unresolved_bindings[0].reason,
            UnresolvedReason::NoCandidateClasses
        );
    }

    #[test]
    fn unknown_candidate_classes_mark_bindings_as_no_known_candidate() {
        let page = page_with("P", Some("Ghost"), &["Phantom"], &[("save", false)]);
        // Symbol table knows neither class.
        let syms = symbols_map(vec![("Other", vec![method("save", vec![])])]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        assert_eq!(out.unresolved_bindings.len(), 1);
        assert_eq!(
            out.unresolved_bindings[0].reason,
            UnresolvedReason::NoKnownCandidateClass
        );
    }

    #[test]
    fn bindings_with_no_matching_method_are_flagged_no_matching_method() {
        let page = page_with("P", Some("Ctrl"), &[], &[("nope", false)]);
        let syms = symbols_map(vec![("Ctrl", vec![method("save", vec![])])]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        assert_eq!(out.unresolved_bindings.len(), 1);
        assert_eq!(
            out.unresolved_bindings[0].reason,
            UnresolvedReason::NoMatchingMethod
        );
    }

    #[test]
    fn case_insensitive_class_name_lookup() {
        let page = page_with("P", Some("Foo_Ctrl"), &[], &[("save", false)]);
        // Apex is case-insensitive; registry keyed on exact source case.
        let syms = symbols_map(vec![("FOO_CTRL", vec![method("save", vec![])])]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        assert_eq!(out.resolved_bindings.len(), 1);
        assert_eq!(out.resolved_bindings[0].target_class, "FOO_CTRL");
    }

    #[test]
    fn each_binding_gets_a_distinct_call_site_range() {
        let page = page_with(
            "P",
            Some("Ctrl"),
            &[],
            &[("save", false), ("save", true), ("load", false)],
        );
        let syms = symbols_map(vec![(
            "Ctrl",
            vec![method("save", vec![]), method("load", vec![])],
        )]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        assert_eq!(out.resolved_bindings.len(), 3);
        let lines: Vec<u32> = out
            .resolved_bindings
            .iter()
            .map(|b| b.call_site_location.start_line)
            .collect();
        assert_eq!(lines, vec![1, 2, 3]);
    }

    #[test]
    fn synthetic_container_and_body_have_distinct_node_ids() {
        let page = page_with("P", Some("Ctrl"), &[], &[]);
        let syms = symbols_map(vec![("Ctrl", Vec::new())]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        let ids: Vec<_> = out.synthetic_nodes.iter().map(|n| n.id.as_str()).collect();
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn synthetic_body_carries_entry_point_marker() {
        // Round 5 hand-audit sample 9 (ALLO_RollupBTN::__vf_page__())
        // exposed that the synthetic page body was being misclassified
        // as `no_callers` because no in-repo code calls it (the
        // Salesforce runtime does). The entry-point marker belongs on
        // the body at synthesis time so every downstream consumer
        // (dead-code reason classifier, audit pool) sees the same
        // truth without a second pass.
        let page = page_with("P", Some("Ctrl"), &[], &[]);
        let syms = symbols_map(vec![("Ctrl", Vec::new())]);
        let out = resolve_vf_page(&page, &syms, Some("/tmp/ws"));
        // Two nodes: [0] = container Struct, [1] = body Function.
        let body = out
            .synthetic_nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function)
            .expect("synth pack must include a body Function");
        let props = &body.properties;
        assert_eq!(
            props
                .get("is_attribute_invoked")
                .and_then(|v| v.as_bool()),
            Some(true),
            "synthetic body must be marked is_attribute_invoked so the analysis-side entry-point classifier exempts it from no_callers"
        );
        assert_eq!(
            props.get("entry_point_reason").and_then(|v| v.as_str()),
            Some("visualforce_page_body"),
            "entry_point_reason must document WHY the body is exempt"
        );
        // Container is a non-function node; the marker does not belong there.
        let container = out
            .synthetic_nodes
            .iter()
            .find(|n| n.kind == NodeKind::Struct)
            .expect("synth pack must include a container Struct");
        assert!(
            !container.properties.contains_key("is_attribute_invoked"),
            "container Struct must not carry is_attribute_invoked; the marker is a method-level concept"
        );
    }
}
