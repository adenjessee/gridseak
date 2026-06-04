//! Constructor-call resolution for the Apex heuristic resolver
//! (TR-A.1 + TR-A.2).
//!
//! Called from the heuristic resolver's main loop for every call site
//! whose `function_name` carries the `constructor_call:` prefix emitted
//! by [`crate::syntax::extractors::call_site_extractor`]. This module
//! owns:
//!
//! 1. Decoding the target class from the emitted name shape —
//!    `constructor_call:X::new`, `constructor_call:__self::new`, and
//!    `constructor_call:__super::new`.
//! 2. Resolving the target class's [`ApexClassSymbols`] through the
//!    [`ApexClassRegistry`], including the parent-class hop for
//!    `__super` and the enclosing-type lookup for `__self`.
//! 3. Disambiguating overloaded constructors via
//!    [`super::signature_matcher::rank_candidates`], with
//!    `CallSite.arg_types` feeding the arity → exact → widening ladder.
//! 4. Mapping the matched [`ApexConstructor`] back to its indexed
//!    `Function` [`Node`] so the resolver can emit the edge against
//!    the correct overload.
//!
//! The module deliberately does not touch `SymbolIndex`'s internals —
//! callers pass in the slices they hold. That keeps
//! [`super::heuristic_resolver`] free of ctor-specific plumbing and
//! leaves this arm testable in isolation.
//!
//! # Implicit default constructor
//!
//! A class with no declared constructors exposes an implicit zero-arg
//! constructor (see PHASE_A_EXECUTION_PLAN §3.1 step 2). No
//! `constructor_declaration` AST node exists for that shape, so the
//! graph has no `Function` node to attach the edge to. In that case
//! the resolver emits the `Call` edge to the class's **type** node
//! (the `Struct`/`Interface`/etc. indexed in `types_by_name_lower`).
//! Consumers reading the graph see "caller → class X", which is the
//! most truthful available representation without synthesising a
//! phantom Function node.
//!
//! # Signature-to-node pairing
//!
//! `ApexClassSymbols.constructors` and the graph's `Function` nodes
//! are populated by the same extraction pass but stored in disjoint
//! structures; the matching key across them is the parameter
//! signature. Apex FQN signatures drop generic arguments
//! (`List<String>` → `List`, per `fqn::canonical_type`), so this
//! module renders [`ApexTypeRef`] with the identical lossy convention
//! via [`canonical_apex_param_sig`]. Both sides collapse to the same
//! comma-joined string, giving an unambiguous pairing for the single-
//! class cohort we look at per call site.
//!
//! ## List vs. array source-form duality
//!
//! Apex accepts two interchangeable syntactic spellings for a
//! list-shaped parameter: `List<String>` (generic) and `String[]`
//! (array). `ApexTypeRef` normalises both to `Collection::List<String>`,
//! but `fqn::canonical_type` preserves whichever spelling appears in
//! source (`(List)` vs `(String[])`). A single-sig lookup would
//! silently miss every ctor whose source uses the `String[]` form —
//! the NPSP Sample 8 root cause we isolated and fixed in PR 8. The
//! pairing helper now computes the Cartesian product of per-param
//! sig alternatives (see [`param_sig_alternatives`]) and tries each
//! form against the FQN-side index.
//!
//! This pairing replaces the tempting but brittle "zip by declaration
//! order" shortcut — order alignment holds today but would silently
//! break the moment the extractor and symbols pipeline emit into
//! different orders.

use std::collections::HashMap;

use crate::application::ports::{CallSite, LocalVarScope};
use crate::domain::apex::class_symbols::{ApexConstructor, ApexParameter, ApexTypeRef};
use crate::domain::{Confidence, Node};
use crate::syntax::extractors::call_site_extractor::{
    CONSTRUCTOR_CALL_PREFIX, SELF_CTOR_SENTINEL, SUPER_CTOR_SENTINEL,
};

use super::arg_type_narrower::narrow_arg_types;
use super::class_registry::ApexClassRegistry;
use super::signature_matcher::{rank_candidates, CtorLike, SigMatchResult};

/// Emission returned by [`resolve_constructor_call`]. Callers lift
/// the `callees` into graph edges at the resolver's assigned
/// provenance.
pub(super) struct CtorResolution<'a> {
    /// Target node(s). Single entry for [`Confidence::Medium`],
    /// multiple entries for [`Confidence::Low`] fanout.
    pub callees: Vec<&'a Node>,
    pub confidence: Confidence,
}

/// Return value encoding the per-call-site outcome. `None` means the
/// site is dropped (unresolved target class, symbols not attached,
/// arity mismatch, or an edge that would be a self-loop). `Some` is
/// the edge set the caller must emit.
pub(super) type CtorOutcome<'a> = Option<CtorResolution<'a>>;

/// Resolve a `constructor_call:` call site. See module docs for the
/// lookup order and the implicit-default-ctor fallback.
///
/// `caller` is the resolved enclosing-function node already validated
/// by the main resolver loop; it is threaded through so the self-loop
/// drop mirrors the plain-method path.
///
/// `enclosing_type_fqn` comes from
/// [`super::heuristic_resolver::SymbolIndex::find_enclosing_type`] on
/// the call-site range. Required only for `__self` / `__super`;
/// plain `new X(...)` does not consult it.
pub(super) fn resolve_constructor_call<'a>(
    call_site: &CallSite,
    caller: &Node,
    registry: &'a ApexClassRegistry,
    functions_by_name_lower: &HashMap<String, Vec<&'a Node>>,
    types_by_name_lower: &HashMap<String, Vec<&'a Node>>,
    enclosing_type_fqn: Option<&str>,
    local_var_scopes: &[LocalVarScope],
) -> CtorOutcome<'a> {
    let bare = strip_constructor_prefix(&call_site.function_name)?;

    let target_api = match bare {
        _ if bare.eq_ignore_ascii_case(SELF_CTOR_SENTINEL) => {
            // `this(...)` delegates to a sibling ctor on the same class.
            // The enclosing type's api name IS the target.
            api_name_from_type_fqn(enclosing_type_fqn?).to_string()
        }
        _ if bare.eq_ignore_ascii_case(SUPER_CTOR_SENTINEL) => {
            // `super(...)` delegates to the direct parent class's ctor.
            let self_api = api_name_from_type_fqn(enclosing_type_fqn?);
            let self_syms = registry.symbols_for(self_api)?;
            self_syms.parent_class.as_deref()?.to_string()
        }
        other => {
            // `new X(...)` shape — extractor emits `X::new` as the
            // bare name. Strip the sentinel suffix.
            let raw = other.strip_suffix("::new").unwrap_or(other);
            // PHASE_A_EXECUTION_PLAN §3.1 step 1 — sibling-inner fast
            // path. If the call happens inside a class whose
            // `inner_classes` list carries this bare name, bind the
            // target to `Outer.Bare` so a sibling inner-class ctor
            // is found before a registry-wide match on the short
            // name (which under the short-name → dotted one-way
            // `registry::lookup` fallback would otherwise miss the
            // inner entirely, since the inner is keyed as
            // `Outer.Bare`, not `Bare`). Only runs when the raw
            // name is unqualified — a dotted input means the caller
            // already said exactly which class they meant.
            resolve_sibling_inner(raw, enclosing_type_fqn, registry)
                .unwrap_or_else(|| raw.to_string())
        }
    };

    let target_syms = registry.symbols_for(&target_api)?;
    let target_type_node = find_type_node(types_by_name_lower, &target_api);

    // Pair each declared ctor with its graph Function node. Ctors
    // present in symbols but missing a Function node fall out with
    // `node = None` — which still participates in ranking, but the
    // caller drops the emission for that ctor (see `into_resolution`).
    let ctor_nodes = collect_ctor_nodes(target_type_node, &target_api, functions_by_name_lower);
    let pairings = pair_ctors_with_nodes(&target_syms.constructors, &ctor_nodes);

    // Implicit default-constructor fallback: no declared ctors AND
    // the call is zero-arg. Emit one edge to the class type node.
    if target_syms.constructors.is_empty() && call_site.arg_types.is_empty() {
        let node = target_type_node?;
        if node.id == caller.id {
            return None;
        }
        return Some(CtorResolution {
            callees: vec![node],
            confidence: Confidence::Medium,
        });
    }

    // TR-A.4: narrow identifier arguments (`new Foo(bar)` where
    // `bar` is a typed local / enclosing-class field) to their
    // declared types so the matcher's Exact / Widening tiers can
    // fire. The narrower is a no-op on literal / `new`-expression
    // args so pre-TR-A.4 call sites are unchanged.
    let narrowed_args = if let Some(fqn) = enclosing_type_fqn {
        narrow_arg_types(
            &call_site.arg_types,
            &call_site.location,
            api_name_from_type_fqn(fqn),
            registry,
            local_var_scopes,
        )
    } else {
        call_site.arg_types.clone()
    };

    // `rank_candidates` returns references borrowed from `pairings`.
    // Eagerly extract the graph nodes before `pairings` falls out of
    // scope so the returned [`CtorResolution`] doesn't capture a
    // local borrow.
    let ranked = rank_candidates(&pairings, &narrowed_args);
    into_resolution(ranked, caller)
}

/// Public only for unit tests inside `heuristic_resolver`. The
/// production main loop pre-filters on the prefix before calling
/// [`resolve_constructor_call`].
pub(super) fn is_constructor_call_name(function_name: &str) -> bool {
    function_name.starts_with(CONSTRUCTOR_CALL_PREFIX)
        && function_name.as_bytes().get(CONSTRUCTOR_CALL_PREFIX.len()) == Some(&b':')
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Wrapping pair so [`rank_candidates`] can return the graph node
/// directly alongside the matched ctor. `node = None` slots carry an
/// `ApexConstructor` for which no indexed `Function` node exists —
/// they still rank so arity / exact-match correctness is preserved,
/// but the caller elides the edge.
struct CtorNodeRef<'a> {
    ctor: &'a ApexConstructor,
    node: Option<&'a Node>,
}

impl<'a> CtorLike for CtorNodeRef<'a> {
    fn parameters(&self) -> &[ApexParameter] {
        &self.ctor.parameters
    }
}

fn strip_constructor_prefix(name: &str) -> Option<&str> {
    name.strip_prefix(CONSTRUCTOR_CALL_PREFIX)
        .and_then(|rest| rest.strip_prefix(':'))
}

/// PHASE_A_EXECUTION_PLAN §3.1 step 1 — sibling-inner fast path.
///
/// When the call site writes `new Bare(...)` from inside a class whose
/// `ApexClassSymbols.inner_classes` list carries `Bare`, rewrite the
/// target to the dotted `Outer.Bare` form so the registry's
/// primary-key lookup (dotted) finds the inner class. Returns `None`
/// when there is no enclosing type, the enclosing type has no symbols
/// attached, the bare name is already dotted, or no class in the
/// outer chain lists `Bare` as an inner.
///
/// Walks the outer chain from the immediate enclosing class upward so
/// that `new SiblingInner()` written from inside `Outer.Inner` (where
/// `SiblingInner` is declared alongside `Inner` at the `Outer` level)
/// also rewrites to `Outer.SiblingInner`. Apex only supports one
/// level of nesting, but the walk is expressed in general terms so
/// future nested-class-relaxation in the language does not require
/// another patch here.
///
/// Intentionally case-sensitive on the inner-class name: Apex inner
/// class names are case-preserving in declarations, and matching
/// case-insensitively here would cause spurious rewrites when a
/// top-level class in another file happens to share a simple name
/// with an inner.
fn resolve_sibling_inner(
    bare: &str,
    enclosing_type_fqn: Option<&str>,
    registry: &ApexClassRegistry,
) -> Option<String> {
    if bare.contains('.') {
        return None;
    }
    let outer_fqn = enclosing_type_fqn?;
    let mut current: &str = api_name_from_type_fqn(outer_fqn);
    loop {
        if let Some(syms) = registry.symbols_for(current) {
            if syms
                .inner_classes
                .iter()
                .any(|n| n.eq_ignore_ascii_case(bare))
            {
                return Some(format!("{current}.{bare}"));
            }
        }
        // Walk one level outward: `A.B.C` -> `A.B` -> `A` -> stop.
        match current.rsplit_once('.') {
            Some((parent, _)) if !parent.is_empty() => current = parent,
            _ => return None,
        }
    }
}

/// Type FQNs from `apex_fqn::build_type_fqn` take the shape
/// `<workspace_path>::Outer.Inner`. The api-name key used by the
/// registry is the dotted tail after the final `::`. Mirrors the
/// sibling `heuristic_resolver::api_name_from_type_fqn` — duplicated
/// here so this module does not force that helper to leak into
/// private-sibling scope beyond its own file.
fn api_name_from_type_fqn(fqn: &str) -> &str {
    fqn.rsplit_once("::").map(|(_, tail)| tail).unwrap_or(fqn)
}

fn find_type_node<'a>(
    types_by_name_lower: &HashMap<String, Vec<&'a Node>>,
    target_api: &str,
) -> Option<&'a Node> {
    let key = target_api.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    // Exact hit first (supports dotted `Outer.Inner`).
    if let Some(hits) = types_by_name_lower.get(&key) {
        if hits.len() == 1 {
            return Some(hits[0]);
        }
        // Ambiguous — refuse to pick arbitrarily; the edge drops
        // rather than bind to the wrong class.
        if !hits.is_empty() {
            return None;
        }
    }
    // Short-name fallback for a dotted input where only the short
    // form happens to be indexed (test-shaped fixtures).
    if let Some((_, last)) = key.rsplit_once('.') {
        if let Some(hits) = types_by_name_lower.get(last) {
            if hits.len() == 1 {
                return Some(hits[0]);
            }
        }
    }
    None
}

/// Collect the `Function` nodes that represent the declared
/// constructors on `target_api`. Shape of an Apex ctor Function FQN:
/// `<workspace>::<class>::<class_short>(<sig>)`. We look up by short
/// class name (ctors' simple name equals the class's simple name per
/// `apex_fqn::build_method_fqn`), then filter by enclosing-class FQN
/// to drop cross-class name collisions.
fn collect_ctor_nodes<'a>(
    target_type_node: Option<&'a Node>,
    target_api: &str,
    functions_by_name_lower: &HashMap<String, Vec<&'a Node>>,
) -> Vec<&'a Node> {
    // Ctors carry the class's simple name. For `Outer.Inner`, the
    // simple name is `Inner`; for `Outer`, it is `Outer`.
    let short = target_api
        .rsplit('.')
        .next()
        .unwrap_or(target_api)
        .to_ascii_lowercase();
    let Some(candidates) = functions_by_name_lower.get(&short) else {
        return Vec::new();
    };

    let type_fqn = target_type_node.map(|n| n.fqn.as_str());

    candidates
        .iter()
        .copied()
        .filter(|n| match (type_fqn, enclosing_class_fqn(&n.fqn)) {
            // Enclosing-class FQN match: production shape produces a
            // complete type FQN for the ctor's owner; require strict
            // (case-insensitive) equality against the target type's
            // FQN. This is what keeps `Foo::Foo(Int)` from binding to
            // `path::Other::Foo(Int)` in a different class.
            (Some(target), Some(enclosing)) => target.eq_ignore_ascii_case(&enclosing),
            // Test fixtures sometimes register function nodes with
            // bare FQNs that carry no enclosing-class prefix. Fall
            // back to requiring that the candidate's simple name
            // matches the target's short name — which is already
            // guaranteed by the HashMap key, so accept.
            (_, None) => true,
            // Production target FQN missing means the class is
            // registry-known but no graph Type node exists
            // (preloaded SObject, managed-package stub). No ctor
            // Function nodes should pair with these; skip.
            (None, _) => false,
        })
        .collect()
}

/// Return the class portion of a function FQN. Mirrors
/// `super::heuristic_resolver::enclosing_class_fqn` — kept local to
/// avoid widening that module's private surface. If the upstream
/// implementation evolves, keep this helper in sync.
fn enclosing_class_fqn(fqn: &str) -> Option<String> {
    let without_sig = match fqn.find('(') {
        Some(idx) => &fqn[..idx],
        None => fqn,
    };
    if let Some(pos) = without_sig.rfind("::") {
        let prefix = &without_sig[..pos];
        if !prefix.is_empty() {
            return Some(prefix.to_string());
        }
    }
    if let Some((class_part, _)) = without_sig.rsplit_once('.') {
        if !class_part.is_empty() {
            return Some(class_part.to_string());
        }
    }
    None
}

/// Pair each `ApexConstructor` with the Function node whose parameter
/// signature matches under the FQN canonical rendering. See module
/// docs for why this is preferred over index-zip.
///
/// # Array vs. generic-list dual rendering
///
/// Apex source can declare a `List<String>`-equivalent parameter in
/// two interchangeable syntactic forms: `List<String>` (generic) or
/// `String[]` (array). The class-symbols extractor normalises both to
/// `ApexTypeRef::Collection { kind: List, element: String }`, but the
/// Function FQN preserves the source form (`(List)` vs `(String[])`
/// — see `apex_fqn::canonical_type`). A single-sig lookup here would
/// silently miss every ctor whose source uses the `String[]` spelling
/// — which is the root cause we isolated for NPSP Sample 8. To stay
/// robust against source-form drift we compute the Cartesian product
/// of per-param sig alternatives and try each until one hits the
/// FQN-side index.
fn pair_ctors_with_nodes<'a>(
    ctors: &'a [ApexConstructor],
    candidate_nodes: &[&'a Node],
) -> Vec<CtorNodeRef<'a>> {
    // Build a lookup `canonical_sig (lowercase) -> node`. Multiple
    // nodes with the same canonical sig should be impossible per
    // Apex compile rules, but guard against it by keeping the first
    // and ignoring duplicates (first wins matches the registry
    // insertion convention).
    let mut by_sig: HashMap<String, &Node> = HashMap::new();
    for node in candidate_nodes {
        if let Some(sig) = fqn_param_sig(&node.fqn) {
            by_sig.entry(sig.to_ascii_lowercase()).or_insert(*node);
        }
    }

    ctors
        .iter()
        .map(|ctor| {
            let node = param_sig_alternatives(&ctor.parameters)
                .iter()
                .find_map(|sig| by_sig.get(&sig.to_ascii_lowercase()).copied());
            CtorNodeRef { ctor, node }
        })
        .collect()
}

/// Extract the parameter-signature substring from a ctor Function's
/// FQN. Shape: `...::Name(p1,p2,...)` → returns `"p1,p2,..."`. When
/// the FQN carries no signature (unexpected for ctor nodes) returns
/// `None`.
fn fqn_param_sig(fqn: &str) -> Option<&str> {
    let open = fqn.rfind('(')?;
    let close = fqn[open..].find(')')?;
    Some(&fqn[open + 1..open + close])
}

/// Render an [`ApexTypeRef`] to the same lossy canonical form that
/// `apex_fqn::canonical_type` emits for ctor/method parameter FQN
/// signatures. Generic args are dropped (`List<String>` → `List`)
/// so the two sources collapse to identical keys for the generic
/// source form. See the module doc's "Signature-to-node pairing"
/// section for rationale and [`param_sig_alternatives`] for the
/// array-bracket source form handled in tandem.
fn canonical_apex_param_sig(ty: &ApexTypeRef) -> String {
    match ty {
        ApexTypeRef::Primitive { name } => name.clone(),
        ApexTypeRef::Sobject { api_name } | ApexTypeRef::UserDefined { api_name } => {
            api_name.clone()
        }
        // `List<X>` / `Set<X>` / `Map<K,V>` all drop type args in the
        // FQN canonical form — base name only.
        ApexTypeRef::Collection { kind, .. } => kind.as_str().to_string(),
        ApexTypeRef::Map { .. } => "Map".to_string(),
        ApexTypeRef::Generic { base, .. } => base.clone(),
        ApexTypeRef::Unresolved { raw } => {
            // Strip whitespace to match `compact_text` in `fqn.rs`.
            raw.split_whitespace().collect()
        }
    }
}

/// Render all plausible canonical forms of a parameter. Most types
/// collapse to a single form (e.g. `String` → `["String"]`); lists
/// produce two — the generic-erased base (`"List"`) and the array-
/// bracket form with element rendered recursively (`"String[]"`).
///
/// This mirrors the two syntactic spellings Apex source can use for
/// the same type (`List<String>` vs `String[]`) and which
/// `apex_fqn::canonical_type` preserves in the Function FQN verbatim,
/// while `class_symbols_extractor` normalises both to
/// `Collection::List`. See [`pair_ctors_with_nodes`] for the root
/// cause and trade-off.
fn canonical_apex_param_sig_alternatives(ty: &ApexTypeRef) -> Vec<String> {
    match ty {
        ApexTypeRef::Collection {
            kind: crate::domain::apex::class_symbols::CollectionKind::List,
            element,
        } => {
            // Array form uses the element's FIRST canonical form only:
            // `String[]` pairs with `"String[]"`, never with `"List[]"`.
            // Nested lists (`List<String>[]`) are vanishingly rare in
            // Apex and the extractor's current shape already collapses
            // them into the base-only form via `canonical_apex_param_sig`
            // — accept the same narrowing here.
            let element_sig = canonical_apex_param_sig(element);
            vec!["List".to_string(), format!("{element_sig}[]")]
        }
        other => vec![canonical_apex_param_sig(other)],
    }
}

/// Compute the Cartesian product of per-parameter sig alternatives.
/// With K `Collection::List` params we return 2^K candidate sigs;
/// with no lists we return a single sig identical to
/// `canonical_apex_param_sig_list`. Order is deterministic
/// (generic-first per param, left-to-right params) so the first
/// `HashMap` hit in [`pair_ctors_with_nodes`] is stable.
fn param_sig_alternatives(params: &[ApexParameter]) -> Vec<String> {
    let mut combos: Vec<Vec<String>> = vec![Vec::new()];
    for p in params {
        let alts = canonical_apex_param_sig_alternatives(&p.ty);
        let mut next: Vec<Vec<String>> = Vec::with_capacity(combos.len() * alts.len());
        for existing in &combos {
            for a in &alts {
                let mut cloned = existing.clone();
                cloned.push(a.clone());
                next.push(cloned);
            }
        }
        combos = next;
    }
    combos.into_iter().map(|parts| parts.join(",")).collect()
}

/// Convert the signature-matcher's [`SigMatchResult`] into the
/// graph-emission shape. Unique → Medium (one edge); Fanout → Low
/// (multi-edge emission, caller applies the fanout cap); None →
/// drop. Candidates whose Function node is missing (implicit ctor
/// nodes pre-synthesis) are elided at edge-emission time.
///
/// The graph-node references are extracted eagerly here: the
/// `SigMatchResult` borrows from `pairings` which is local to
/// [`resolve_constructor_call`], but each `Option<&'a Node>` field
/// is a plain copy of the longer-lived Node reference the pairing
/// closed over. Copying it out before returning lets the outcome
/// outlive the pairings vector.
fn into_resolution<'a>(
    result: SigMatchResult<'_, CtorNodeRef<'a>>,
    caller: &Node,
) -> CtorOutcome<'a> {
    match result {
        SigMatchResult::None => None,
        SigMatchResult::Unique(winner) => {
            let node = winner.node?;
            if node.id == caller.id {
                return None;
            }
            Some(CtorResolution {
                callees: vec![node],
                confidence: Confidence::Medium,
            })
        }
        SigMatchResult::Fanout(candidates) => {
            let callees: Vec<&'a Node> = candidates
                .into_iter()
                .filter_map(|c| c.node)
                .filter(|n| n.id != caller.id)
                .collect();
            if callees.is_empty() {
                return None;
            }
            Some(CtorResolution {
                callees,
                confidence: Confidence::Low,
            })
        }
        // TR-A.4 implicit tier: a unique survivor at the
        // Object-widening / autoboxing tier still emits an edge,
        // but at Low confidence instead of Medium because the
        // match only survived via the universal-`Object` sink. A
        // Low-unique is still strictly more informative than
        // dropping the edge entirely — it records the fact that
        // the call has exactly one structurally-compatible ctor
        // among the candidate pool.
        SigMatchResult::UniqueLow(winner) => {
            let node = winner.node?;
            if node.id == caller.id {
                return None;
            }
            Some(CtorResolution {
                callees: vec![node],
                confidence: Confidence::Low,
            })
        }
        SigMatchResult::FanoutLow(candidates) => {
            let callees: Vec<&'a Node> = candidates
                .into_iter()
                .filter_map(|c| c.node)
                .filter(|n| n.id != caller.id)
                .collect();
            if callees.is_empty() {
                return None;
            }
            Some(CtorResolution {
                callees,
                confidence: Confidence::Low,
            })
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::{
        Access, ApexConstructor, ApexParameter, ApexTypeRef, CollectionKind,
    };
    use crate::domain::Range;

    fn prim(name: &str) -> ApexTypeRef {
        ApexTypeRef::Primitive {
            name: name.to_string(),
        }
    }

    fn ud(name: &str) -> ApexTypeRef {
        ApexTypeRef::UserDefined {
            api_name: name.to_string(),
        }
    }

    fn sobj(name: &str) -> ApexTypeRef {
        ApexTypeRef::Sobject {
            api_name: name.to_string(),
        }
    }

    fn list_of(inner: ApexTypeRef) -> ApexTypeRef {
        ApexTypeRef::Collection {
            kind: CollectionKind::List,
            element: Box::new(inner),
        }
    }

    fn map_of(k: ApexTypeRef, v: ApexTypeRef) -> ApexTypeRef {
        ApexTypeRef::Map {
            key: Box::new(k),
            value: Box::new(v),
        }
    }

    fn param(name: &str, ty: ApexTypeRef) -> ApexParameter {
        ApexParameter {
            name: name.to_string(),
            ty,
        }
    }

    fn ctor(params: Vec<ApexParameter>) -> ApexConstructor {
        ApexConstructor {
            parameters: params,
            access: Access::Public,
        }
    }

    fn fn_node_with_fqn(fqn: &str) -> Node {
        Node::function(
            fqn.to_string(),
            Range::with_file(1, 0, 10, 0, "test.cls".to_string()),
        )
    }

    #[test]
    fn is_constructor_call_name_recognises_only_exact_prefix() {
        assert!(is_constructor_call_name("constructor_call:Foo::new"));
        assert!(is_constructor_call_name("constructor_call:__self::new"));
        assert!(is_constructor_call_name("constructor_call:__super::new"));
        assert!(!is_constructor_call_name("method_call:foo"));
        assert!(!is_constructor_call_name("constructor_call"));
        // Substring match without colon separator is rejected so a
        // hypothetical `constructor_call_foo` prefix never fires.
        assert!(!is_constructor_call_name("constructor_callFoo"));
    }

    #[test]
    fn strip_constructor_prefix_returns_bare_target() {
        assert_eq!(
            strip_constructor_prefix("constructor_call:Foo::new"),
            Some("Foo::new")
        );
        assert_eq!(
            strip_constructor_prefix("constructor_call:__self::new"),
            Some("__self::new")
        );
        assert_eq!(strip_constructor_prefix("method_call:foo"), None);
    }

    #[test]
    fn api_name_from_type_fqn_matches_resolver_behaviour() {
        assert_eq!(
            api_name_from_type_fqn("path::Outer::Outer.Inner"),
            "Outer.Inner"
        );
        assert_eq!(api_name_from_type_fqn("Standalone"), "Standalone");
    }

    #[test]
    fn canonical_sig_drops_generics_like_fqn_canonical_type() {
        // Primitive + Sobject + UserDefined render as raw api name.
        assert_eq!(canonical_apex_param_sig(&prim("Integer")), "Integer");
        assert_eq!(canonical_apex_param_sig(&sobj("Account")), "Account");
        assert_eq!(canonical_apex_param_sig(&ud("Gift.GiftId")), "Gift.GiftId");
        // Collections drop type args — `List<Account>` → `List`.
        assert_eq!(canonical_apex_param_sig(&list_of(sobj("Account"))), "List");
        // Map collapses to the base name too.
        assert_eq!(
            canonical_apex_param_sig(&map_of(prim("Id"), sobj("Account"))),
            "Map"
        );
    }

    #[test]
    fn canonical_sig_list_joins_with_commas() {
        // The generic-form alternative is the first entry of
        // `param_sig_alternatives`; the test here asserts that it
        // matches the historical `canonical_apex_param_sig_list`
        // output for non-array parameters (regression guard against
        // accidental rename/renumber of Cartesian alternatives).
        let params = vec![
            param("acc", sobj("Account")),
            param("items", list_of(sobj("Account"))),
            param("tag", prim("String")),
        ];
        let alts = param_sig_alternatives(&params);
        assert!(
            alts.iter().any(|s| s == "Account,List,String"),
            "generic-form sig missing from alternatives: {alts:?}",
        );
    }

    #[test]
    fn param_sig_alternatives_emits_list_and_array_forms_for_list_params() {
        // Single list param → 2 alternatives.
        let params = vec![param("xs", list_of(prim("String")))];
        let alts = param_sig_alternatives(&params);
        assert_eq!(alts.len(), 2);
        assert!(alts.iter().any(|s| s == "List"));
        assert!(alts.iter().any(|s| s == "String[]"));
    }

    #[test]
    fn param_sig_alternatives_cartesian_product_on_multiple_lists() {
        // Two list params → 2 * 2 = 4 alternatives covering every
        // source-spelling combination.
        let params = vec![
            param("xs", list_of(prim("String"))),
            param("ys", list_of(prim("Integer"))),
        ];
        let alts = param_sig_alternatives(&params);
        assert_eq!(alts.len(), 4);
        assert!(alts.contains(&"List,List".to_string()));
        assert!(alts.contains(&"List,Integer[]".to_string()));
        assert!(alts.contains(&"String[],List".to_string()));
        assert!(alts.contains(&"String[],Integer[]".to_string()));
    }

    #[test]
    fn param_sig_alternatives_single_form_for_scalar_params() {
        // No list params → 1 alternative identical to the generic
        // canonical sig.
        let params = vec![param("a", prim("String")), param("b", prim("Integer"))];
        let alts = param_sig_alternatives(&params);
        assert_eq!(alts, vec!["String,Integer".to_string()]);
    }

    #[test]
    fn fqn_param_sig_extracts_signature_body() {
        assert_eq!(
            fqn_param_sig("path::Gift::Gift::Gift(Gift.GiftId)"),
            Some("Gift.GiftId")
        );
        assert_eq!(
            fqn_param_sig("path::Foo::Foo::Foo(List,Map,String)"),
            Some("List,Map,String")
        );
        // Zero-arg ctor produces an empty-string signature body.
        assert_eq!(fqn_param_sig("path::Foo::Foo::Foo()"), Some(""));
        // FQNs without a signature (type FQNs) return None.
        assert_eq!(fqn_param_sig("path::Foo::Foo"), None);
    }

    #[test]
    fn enclosing_class_fqn_mirrors_resolver_helper() {
        assert_eq!(
            enclosing_class_fqn("path::Foo::Foo::Foo(Integer)"),
            Some("path::Foo::Foo".to_string())
        );
        assert_eq!(
            enclosing_class_fqn("path::Outer::Outer.Inner::Inner(Id)"),
            Some("path::Outer::Outer.Inner".to_string())
        );
        assert_eq!(enclosing_class_fqn("Foo.bar"), Some("Foo".to_string()));
        assert_eq!(enclosing_class_fqn("bare"), None);
    }

    #[test]
    fn pair_ctors_matches_by_canonical_signature_not_order() {
        // Two ctors declared, nodes registered in REVERSE order.
        // Pairing by signature must correct for that.
        let ctors = vec![
            ctor(vec![param("id", ud("Gift.GiftId"))]),
            ctor(vec![param("items", list_of(ud("Gift")))]),
        ];
        let n_list = fn_node_with_fqn("path::Gift::Gift::Gift(List)");
        let n_id = fn_node_with_fqn("path::Gift::Gift::Gift(Gift.GiftId)");
        let nodes = vec![&n_list, &n_id];

        let pairings = pair_ctors_with_nodes(&ctors, &nodes);
        assert_eq!(pairings.len(), 2);
        assert_eq!(
            pairings[0].node.map(|n| n.fqn.as_str()),
            Some(n_id.fqn.as_str())
        );
        assert_eq!(
            pairings[1].node.map(|n| n.fqn.as_str()),
            Some(n_list.fqn.as_str())
        );
    }

    #[test]
    fn pair_ctors_leaves_node_none_when_no_match() {
        let ctors = vec![ctor(vec![param("x", prim("Integer"))])];
        // Different signature — intentional mismatch.
        let n = fn_node_with_fqn("path::Foo::Foo::Foo(String)");
        let nodes = vec![&n];

        let pairings = pair_ctors_with_nodes(&ctors, &nodes);
        assert_eq!(pairings.len(), 1);
        assert!(pairings[0].node.is_none());
    }

    #[test]
    fn canonical_sig_is_case_insensitive_for_node_pairing() {
        // Ctor declares `Integer` (as-cased). FQN canonical_type
        // preserves original casing, but our pair lookup lowercases
        // both sides so case drift in the source doesn't desync.
        let ctors = vec![ctor(vec![param("x", prim("integer"))])];
        let n = fn_node_with_fqn("path::Foo::Foo::Foo(Integer)");
        let nodes = vec![&n];
        let pairings = pair_ctors_with_nodes(&ctors, &nodes);
        assert_eq!(
            pairings[0].node.map(|x| x.fqn.as_str()),
            Some(n.fqn.as_str())
        );
    }
}
