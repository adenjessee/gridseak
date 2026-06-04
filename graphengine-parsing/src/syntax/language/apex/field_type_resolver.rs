//! Field-type-aware dispatch for Apex (TR-A.3).
//!
//! Binds `receiver.method(args)` call sites to their correct target
//! class by walking the receiver's declared type, not the receiver's
//! identifier alone. Called from the heuristic resolver's main loop
//! for every call site whose [`CallSite::receiver_text`] is populated.
//!
//! # Resolution order
//!
//! Matches `PHASE_A_EXECUTION_PLAN.md` §4.1:
//!
//! 1. **`this` / bare self** — the call's target class is the
//!    enclosing class. `this.foo` collapses to "field `foo` on the
//!    enclosing class".
//! 2. **Local scope** — look up the receiver name in the method's
//!    [`LocalVarScope`]. A match gives us the local's declared
//!    [`ApexTypeRef`] → the target class.
//! 3. **Enclosing class fields** — `ApexClassSymbols.fields` on the
//!    enclosing class. Matching field → declared type → target
//!    class.
//! 4. **Parent-class chain** — walk up `parent_class` via the
//!    registry for (2) and (3) until a match is found or the chain
//!    ends.
//!
//! Once a target class is known, methods are looked up on that
//! class's [`ApexClassSymbols`] and paired with their indexed
//! `Function` nodes via FQN canonical signature (reused from the
//! constructor-resolver pairing logic). Overload disambiguation
//! reuses the same signature-matcher ladder as the constructor
//! arm — arity filter → exact match → widening → arity-only
//! fanout.
//!
//! # Explicit non-goal
//!
//! No inference for assignments. `UTIL_Permissions p = makePermissions();`
//! uses the declared LHS type (`UTIL_Permissions`), not the return
//! type of `makePermissions()`. The extractor records the declared
//! type of the local/field; this resolver trusts it.
//!
//! # Dotted receivers
//!
//! Receivers that are themselves dotted expressions (`Outer.Inner.m()`,
//! `someField.subField.m()`) are deferred to TR-A.6's inner-class
//! containment walker. This module declines to resolve them — the
//! resolver falls back to the existing name-only lookup in that
//! case, preserving today's behaviour until PR 5 wires the walker.
//!
//! # Static dispatch by type name (R40)
//!
//! Apex supports calling static methods via `ClassName.staticMethod(...)`
//! from any other class. The receiver text is a bare or dotted type
//! name, not a value expression. Before the BareIdent / DottedDefer
//! paths give up, the resolver now consults the
//! [`ApexClassRegistry`]: if the receiver matches a registered type
//! (user class, preloaded SObject, or managed-package stub), the
//! target class is that type directly. This binds the static-dispatch
//! call on the same ladder as instance dispatch so overload ranking
//! and parent-chain inheritance apply identically.

use std::collections::HashMap;

use crate::application::ports::{CallSite, LocalVarScope};
use crate::domain::apex::class_symbols::{
    ApexClassSymbols, ApexMethod, ApexParameter, ApexTypeRef, CollectionKind,
};
use crate::domain::{Confidence, Node, Range};

use super::arg_type_narrower::narrow_arg_types;
use super::class_registry::ApexClassRegistry;
use super::inner_class_resolver::canonicalise_api_name;
use super::signature_matcher::{rank_candidates, CtorLike, SigMatchResult};

/// Maximum number of `parent_class` links to follow when searching a
/// class chain for a field or method. Guards against cyclical
/// `parent_class` declarations in malformed source.
const PARENT_CHAIN_MAX_DEPTH: usize = 16;

/// Outcome returned by [`resolve_field_type_call`]. `None` means the
/// resolver declines (no receiver text, no enclosing type, receiver
/// is dotted and deferred to TR-A.6, receiver's type didn't resolve,
/// or no method of the requested name exists on the target).
///
/// `Some` always carries at least one callee — call sites that
/// resolve the target class but find no matching method return
/// `None`, not `Some(empty)`.
pub(super) struct FieldDispatchResolution<'a> {
    pub callees: Vec<&'a Node>,
    pub confidence: Confidence,
}

pub(super) type FieldDispatchOutcome<'a> = Option<FieldDispatchResolution<'a>>;

/// Resolve a method call whose receiver text is known. See module
/// docs for the lookup ladder.
///
/// `method_name` is the stripped method name (no `obj.` prefix, no
/// `method_call:` prefix — the caller has already normalised).
///
/// `enclosing_type_api` is the api-name of the type that owns the
/// call-site range, from
/// [`super::heuristic_resolver::SymbolIndex::find_enclosing_type`]
/// plus an api-name strip. Required: without it we have no base for
/// `this`, no enclosing-class fields to search, and no parent chain.
///
/// Returns `None` when the resolver declines — the caller should
/// then fall through to its existing name-based dispatch.
pub(super) fn resolve_field_type_call<'a>(
    call_site: &CallSite,
    caller: &Node,
    method_name: &str,
    enclosing_type_api: &str,
    registry: &'a ApexClassRegistry,
    functions_by_name_lower: &HashMap<String, Vec<&'a Node>>,
    local_var_scopes: &[LocalVarScope],
) -> FieldDispatchOutcome<'a> {
    // TR-A.4 bare-self-call arm. In Apex every method invocation
    // without an explicit receiver runs against `this`, so
    // `foo(args)` inside a method body is semantically identical
    // to `this.foo(args)`. Before TR-A.4 the resolver declined on
    // `receiver_text = None` and fell through to the name-only
    // fanout path, which fails the fanout cap whenever the target
    // method name is common across the codebase (`log`, `save`,
    // `execute`). Treating the bare form as `SelfRef { field: None }`
    // funnels it through the same enclosing-class lookup the
    // explicit-`this` form already used, unifying the two shapes
    // on a single code path. This closes R38's
    // `Contacts::loadAccountByIdMap()` deferred FQN.
    let normalised_receiver = match call_site.receiver_text.as_deref() {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                NormalisedReceiver::SelfRef { field: None }
            } else {
                normalise_receiver(trimmed)
            }
        }
        None => NormalisedReceiver::SelfRef { field: None },
    };

    // Dotted receivers are TR-A.6 territory. Single `this.<ident>`
    // is handled here because the target-type resolution is trivially
    // local to the enclosing class — no containment-walk needed.

    let target_type = resolve_receiver_type(
        &normalised_receiver,
        &call_site.location,
        enclosing_type_api,
        registry,
        local_var_scopes,
    )?;

    let target_api_raw = target_type.to_api_name();
    let target_api = strip_generic_suffix(&target_api_raw);

    // TR-A.6: normalise short inner-class references to their dotted
    // registry key. A receiver typed as `Inner` (declared in the
    // enclosing class's inner-class list) is stored in the registry
    // under `Outer.Inner`; a plain short-name lookup would miss and
    // fall through to the name-only fanout path, downgrading to Low
    // confidence. The canonicaliser consults the enclosing class's
    // `inner_classes` first, then falls back to a direct registry
    // lookup for top-level classes. When the canonicaliser declines
    // (neither sibling-inner nor top-level), we keep the raw short
    // form so the parent-chain walk and the name-only fallback each
    // get their chance — the caller already saw the field as typed,
    // so refusing to walk the parent chain would be strictly worse.
    let canonical_target = canonicalise_api_name(target_api, enclosing_type_api, registry)
        .unwrap_or_else(|| target_api.to_string());

    // Walk the target type + its parent chain looking for the method.
    // We return the first class in the chain that declares the method;
    // Apex semantics say an override on the child wins, so stopping at
    // the first hit is correct.
    let mut visited: Vec<String> = Vec::with_capacity(4);
    let mut current_api: String = canonical_target;

    // TR-A.4: narrow identifier arguments to their declared types
    // once, up-front, before the parent-chain walk. Callers that
    // land here all see the same caller scope, so doing this inside
    // the loop would be wasted work. The narrower is a no-op on args
    // already typed by the inferrer (literals / `new` expressions)
    // so the extra pass is cheap on call sites that don't need it.
    let narrowed_args = narrow_arg_types(
        &call_site.arg_types,
        &call_site.location,
        enclosing_type_api,
        registry,
        local_var_scopes,
    );

    for _ in 0..PARENT_CHAIN_MAX_DEPTH {
        if visited.iter().any(|v| v.eq_ignore_ascii_case(&current_api)) {
            break;
        }
        visited.push(current_api.clone());

        let Some(syms) = registry.symbols_for(&current_api) else {
            break;
        };

        let candidates: Vec<&ApexMethod> = syms.methods_named(method_name).collect();
        if !candidates.is_empty() {
            return method_outcome(
                &candidates,
                &current_api,
                caller,
                functions_by_name_lower,
                &narrowed_args,
            );
        }

        let Some(parent) = syms.parent_class.clone() else {
            break;
        };
        let parent_short = strip_generic_suffix(&parent);
        // TR-A.6: a `parent_class = "Sibling"` declared inside an
        // inner class can mean either a top-level class or a
        // sibling-inner of the same outer. Canonicalise against the
        // current class's outer (derived from the dotted key) so the
        // parent-chain walk reaches sibling-inner parents too.
        let parent_enclosing = current_api
            .rsplit_once('.')
            .map(|(outer, _)| outer.to_string())
            .unwrap_or_else(|| enclosing_type_api.to_string());
        current_api = canonicalise_api_name(parent_short, &parent_enclosing, registry)
            .unwrap_or_else(|| parent_short.to_string());
    }

    None
}

// ---------------------------------------------------------------------------
// Receiver normalisation
// ---------------------------------------------------------------------------

/// Return a simpler `NormalisedReceiver` representation of a
/// receiver text. Distinguishes the shapes the resolver can handle:
///
/// * `SelfRef` — `this` (bare) or `this.<field>`. The field name, if
///   any, is carried through.
/// * `BareIdent(name)` — plain identifier, may resolve to a local,
///   an enclosing-class field, or (R40) a type name for static
///   dispatch.
/// * `DottedDefer(text)` — dotted receiver that isn't `this.<field>`.
///   The full original text is carried so the static-dispatch arm
///   (R40) can try to resolve it as a dotted type name
///   (`Outer.Inner.staticMethod()`); if that fails the resolver
///   still declines.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NormalisedReceiver<'a> {
    SelfRef { field: Option<&'a str> },
    BareIdent(&'a str),
    DottedDefer(&'a str),
}

fn normalise_receiver(text: &str) -> NormalisedReceiver<'_> {
    // `this` / `this.<field>`. Case-insensitive on the keyword.
    if text.eq_ignore_ascii_case("this") {
        return NormalisedReceiver::SelfRef { field: None };
    }
    if let Some(field) = text
        .strip_prefix("this.")
        .or_else(|| text.strip_prefix("This."))
    {
        // Accept a single-segment field lookup (`this.foo`). Deeper
        // chains (`this.foo.bar`) are dotted → defer to TR-A.6.
        let field = field.trim();
        if field.is_empty() || field.contains('.') {
            return NormalisedReceiver::DottedDefer(text);
        }
        return NormalisedReceiver::SelfRef { field: Some(field) };
    }
    // Anything containing a `.` that isn't `this.<field>` is dotted.
    // `someField.subField`, `Outer.Inner`, `a.b.c` — all deferred
    // with the original text so the R40 static-dispatch arm can
    // still probe the registry for a dotted type-name match.
    if text.contains('.') {
        return NormalisedReceiver::DottedDefer(text);
    }
    NormalisedReceiver::BareIdent(text)
}

// ---------------------------------------------------------------------------
// Target-type resolution
// ---------------------------------------------------------------------------

fn resolve_receiver_type(
    receiver: &NormalisedReceiver<'_>,
    call_location: &Range,
    enclosing_type_api: &str,
    registry: &ApexClassRegistry,
    local_var_scopes: &[LocalVarScope],
) -> Option<ApexTypeRef> {
    match receiver {
        NormalisedReceiver::SelfRef { field: None } => {
            // `this.m(...)` → target is the enclosing class.
            Some(ApexTypeRef::UserDefined {
                api_name: enclosing_type_api.to_string(),
            })
        }
        NormalisedReceiver::SelfRef { field: Some(name) } => {
            // `this.<field>.m(...)` → target is the declared type of
            // `<field>` on the enclosing class (or a parent).
            resolve_field_in_class_chain(enclosing_type_api, name, registry)
        }
        NormalisedReceiver::BareIdent(name) => {
            // 1. Local scope takes precedence over fields (Apex
            //    shadowing rule: a local variable hides a same-named
            //    field for the duration of its declaration).
            if let Some(ty) = lookup_in_local_scopes(local_var_scopes, call_location, name) {
                return Some(ty);
            }
            // 2. Enclosing-class fields + parent chain.
            if let Some(ty) = resolve_field_in_class_chain(enclosing_type_api, name, registry) {
                return Some(ty);
            }
            // 3. R40 — static dispatch by type name
            //    (`ClassName.staticMethod(...)`). A receiver text
            //    that matches a registered class is treated as a
            //    type reference, not a value. Apex's shadowing
            //    rules guarantee locals / fields are searched first
            //    (steps 1 & 2), so a false match against a
            //    same-named class is impossible.
            resolve_type_name_receiver(name, enclosing_type_api, registry)
        }
        // DottedDefer is a dotted receiver. Two shapes we can still
        // salvage as static dispatch:
        //   * `Outer.Inner` where the dotted form is registered as
        //     an inner class (TR-A.6 registry key).
        //   * A top-level class name happens to appear literally as
        //     `Namespace.Class` in a managed-package handoff
        //     (vanishingly rare; still covered via registry lookup).
        // Property-access shapes (`Enum.VALUE`, `ClassName.CONSTANT`)
        // are rejected because we require the FULL dotted string to
        // resolve as a type — enum values and static-field constants
        // will never satisfy that and fall through to the name-only
        // fallback.
        NormalisedReceiver::DottedDefer(text) => {
            resolve_type_name_receiver(text, enclosing_type_api, registry)
        }
    }
}

/// R40 — resolve a receiver that names a type (for static dispatch).
/// Consults the registry with the raw text, the canonicalised form
/// (inner-class sibling rewrite), and the short-name tail in that
/// order. Returns the matching type as a `UserDefined` so the
/// downstream method-lookup path treats it like any other target
/// class. `None` means the receiver is not a known type — caller
/// keeps its existing decline behaviour.
fn resolve_type_name_receiver(
    text: &str,
    enclosing_type_api: &str,
    registry: &ApexClassRegistry,
) -> Option<ApexTypeRef> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Primary: exact registry match (handles both short top-level
    // names like `ADV_PackageInfo_SVC` and dotted inner-class keys
    // like `GE_Template.Element`).
    if registry.symbols_for(trimmed).is_some() {
        return Some(ApexTypeRef::UserDefined {
            api_name: trimmed.to_string(),
        });
    }

    // Sibling-inner rewrite: when called from inside `Outer`, a
    // short receiver `Inner` might name a sibling inner class
    // registered as `Outer.Inner`. Reuses the ctor-resolver's
    // canonicalisation.
    if let Some(canonical) = canonicalise_api_name(trimmed, enclosing_type_api, registry) {
        if registry.symbols_for(&canonical).is_some() {
            return Some(ApexTypeRef::UserDefined {
                api_name: canonical,
            });
        }
    }

    None
}

/// Pick the innermost local scope whose body contains `call_location`,
/// then find the most recent same-named local declared **before** the
/// call site. Returns the local's declared type.
///
/// The "declared before" check is enforced for two reasons: (a) Apex's
/// forward-declaration rule for locals prevents a use-before-declare
/// anyway, and (b) the extractor records each declarator in source
/// order, so the ranking collapses to a simple filter.
fn lookup_in_local_scopes(
    scopes: &[LocalVarScope],
    call_location: &Range,
    name: &str,
) -> Option<ApexTypeRef> {
    // Innermost = smallest body range that contains the call site.
    let mut containing: Vec<&LocalVarScope> = scopes
        .iter()
        .filter(|s| range_contains(&s.body, call_location))
        .collect();
    containing.sort_by_key(|s| range_span(&s.body));

    // Innermost enclosing scope wins. The extractor currently emits
    // one scope per method / constructor body (parameters + all
    // locals declared anywhere inside the body); nested block scopes
    // are not yet modelled. When they are, switch to a walk-outward
    // lookup — the `containing` vec is already sorted innermost-first
    // for exactly that upgrade.
    let scope = containing.into_iter().next()?;
    for local in scope.locals.iter().rev() {
        if !local.name.eq_ignore_ascii_case(name) {
            continue;
        }
        if range_precedes_or_equals(&local.declared_at, call_location) {
            return Some(local.ty.clone());
        }
    }
    None
}

/// Walk `class_api`'s `ApexClassSymbols` for a field named `field`;
/// if missing, recurse through `parent_class` up to
/// [`PARENT_CHAIN_MAX_DEPTH`]. Returns the declared type of the first
/// match.
fn resolve_field_in_class_chain(
    class_api: &str,
    field: &str,
    registry: &ApexClassRegistry,
) -> Option<ApexTypeRef> {
    let mut visited: Vec<String> = Vec::with_capacity(4);
    let mut current: String = class_api.to_string();
    for _ in 0..PARENT_CHAIN_MAX_DEPTH {
        if visited.iter().any(|v| v.eq_ignore_ascii_case(&current)) {
            break;
        }
        visited.push(current.clone());

        let Some(syms) = registry.symbols_for(&current) else {
            break;
        };
        if let Some(f) = syms.find_field(field) {
            return Some(f.ty.clone());
        }
        let Some(parent) = syms.parent_class.clone() else {
            break;
        };
        current = strip_generic_suffix(&parent).to_string();
    }
    None
}

// ---------------------------------------------------------------------------
// Method-node pairing + ranking
// ---------------------------------------------------------------------------

/// Pair the declared-method candidates with their graph `Function`
/// nodes, rank under the signature ladder, and turn the result into a
/// [`FieldDispatchResolution`]. Mirrors the constructor-resolver's
/// `into_resolution` flow so Low-fanout / Medium-unique / drop
/// semantics are identical across arms.
fn method_outcome<'a>(
    methods: &[&'a ApexMethod],
    target_api: &str,
    caller: &Node,
    functions_by_name_lower: &HashMap<String, Vec<&'a Node>>,
    narrowed_args: &[ApexTypeRef],
) -> FieldDispatchOutcome<'a> {
    let method_nodes = collect_method_nodes(methods, target_api, functions_by_name_lower);
    let pairings = pair_methods_with_nodes(methods, &method_nodes);

    let ranked = rank_candidates(&pairings, narrowed_args);
    match ranked {
        SigMatchResult::None => None,
        SigMatchResult::Unique(winner) => {
            let node = winner.node?;
            if node.id == caller.id {
                return None;
            }
            Some(FieldDispatchResolution {
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
            Some(FieldDispatchResolution {
                callees,
                confidence: Confidence::Low,
            })
        }
        // TR-A.4 implicit tier: unique survivor wins but at Low
        // confidence (the match only survived via Object widening
        // / autoboxing, never a typed bind).
        SigMatchResult::UniqueLow(winner) => {
            let node = winner.node?;
            if node.id == caller.id {
                return None;
            }
            Some(FieldDispatchResolution {
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
            Some(FieldDispatchResolution {
                callees,
                confidence: Confidence::Low,
            })
        }
    }
}

/// Wrapper carrying the `ApexMethod` and its graph `Function` node (when paired).
struct MethodNodeRef<'a> {
    method: &'a ApexMethod,
    node: Option<&'a Node>,
}

impl<'a> CtorLike for MethodNodeRef<'a> {
    fn parameters(&self) -> &[ApexParameter] {
        &self.method.parameters
    }
}

/// Collect graph `Function` nodes whose short name matches
/// `method.name` and whose enclosing class matches `target_api`.
fn collect_method_nodes<'a>(
    methods: &[&'a ApexMethod],
    target_api: &str,
    functions_by_name_lower: &HashMap<String, Vec<&'a Node>>,
) -> Vec<&'a Node> {
    let mut out: Vec<&Node> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for method in methods {
        let key = method.name.to_ascii_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());
        let Some(candidates) = functions_by_name_lower.get(&key) else {
            continue;
        };
        for cand in candidates {
            if enclosing_class_api_matches(&cand.fqn, target_api) {
                out.push(*cand);
            }
        }
    }
    out
}

/// `true` when the enclosing class of a function FQN matches `target_api`
/// under Apex's case-insensitive identifier rules.
fn enclosing_class_api_matches(fqn: &str, target_api: &str) -> bool {
    let Some(class_fqn) = enclosing_class_fqn(fqn) else {
        return false;
    };
    let api = class_fqn
        .rsplit_once("::")
        .map(|(_, tail)| tail)
        .unwrap_or(class_fqn.as_str());
    api.eq_ignore_ascii_case(target_api)
}

/// Pair each `ApexMethod` with the Function node whose parameter
/// signature matches under the FQN canonical rendering. Mirrors the
/// ctor arm's `pair_ctors_with_nodes` — the only shape change is the
/// short-name prefix in the FQN (the method's name, not the class's).
fn pair_methods_with_nodes<'a>(
    methods: &[&'a ApexMethod],
    candidate_nodes: &[&'a Node],
) -> Vec<MethodNodeRef<'a>> {
    // Partition candidate nodes by (method_name_lower, param_sig_lower)
    // so overloads of `compare` under the same enclosing class don't
    // collide with `save` nodes also on that class.
    let mut by_key: HashMap<(String, String), &Node> = HashMap::new();
    for node in candidate_nodes {
        let Some((short, sig)) = fqn_method_and_sig(&node.fqn) else {
            continue;
        };
        let key = (short.to_ascii_lowercase(), sig.to_ascii_lowercase());
        by_key.entry(key).or_insert(*node);
    }

    methods
        .iter()
        .map(|m| {
            // Try each Cartesian sig alternative (see
            // `param_sig_alternatives`); first HashMap hit wins.
            // Handles the `List<T>` / `T[]` source-form divergence
            // documented on the ctor-resolver twin helper.
            let node = param_sig_alternatives(&m.parameters)
                .iter()
                .find_map(|sig| {
                    let key = (m.name.to_ascii_lowercase(), sig.to_ascii_lowercase());
                    by_key.get(&key).copied()
                });
            MethodNodeRef { method: m, node }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Shared helpers (duplicated from sibling resolvers to avoid
// widening their private surface; keep in sync with the originals
// in `constructor_resolver.rs` / `heuristic_resolver.rs`).
// ---------------------------------------------------------------------------

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

/// Extract `(short_method_name, param_sig)` from a method FQN.
/// Shape: `...::<class>::<method>(<sig>)`. Returns `None` if the FQN
/// carries no `(...)` signature.
fn fqn_method_and_sig(fqn: &str) -> Option<(&str, &str)> {
    let open = fqn.rfind('(')?;
    let close = fqn[open..].find(')')?;
    let sig = &fqn[open + 1..open + close];
    let before_open = &fqn[..open];
    let short = before_open.rsplit(['.', ':']).next().unwrap_or(before_open);
    Some((short, sig))
}

fn canonical_apex_param_sig(ty: &ApexTypeRef) -> String {
    match ty {
        ApexTypeRef::Primitive { name } => name.clone(),
        ApexTypeRef::Sobject { api_name } | ApexTypeRef::UserDefined { api_name } => {
            api_name.clone()
        }
        ApexTypeRef::Collection {
            kind: CollectionKind::List,
            ..
        } => "List".to_string(),
        ApexTypeRef::Collection {
            kind: CollectionKind::Set,
            ..
        } => "Set".to_string(),
        ApexTypeRef::Map { .. } => "Map".to_string(),
        ApexTypeRef::Generic { base, .. } => base.clone(),
        ApexTypeRef::Unresolved { raw } => raw.split_whitespace().collect(),
    }
}

/// Twin of `constructor_resolver::canonical_apex_param_sig_alternatives`.
/// Source can write the same list type as `List<T>` or `T[]`; both
/// normalise to `Collection::List` on the ApexTypeRef side but are
/// preserved verbatim in the Function FQN by `apex_fqn::canonical_type`.
/// Returning both spellings lets the method-node pairing match either
/// FQN shape.
fn canonical_apex_param_sig_alternatives(ty: &ApexTypeRef) -> Vec<String> {
    match ty {
        ApexTypeRef::Collection {
            kind: CollectionKind::List,
            element,
        } => {
            let element_sig = canonical_apex_param_sig(element);
            vec!["List".to_string(), format!("{element_sig}[]")]
        }
        other => vec![canonical_apex_param_sig(other)],
    }
}

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

/// Strip `<...>` generic argument suffix off an api-name rendering.
/// `List<Account>` → `List`, `UTIL_Permissions` → `UTIL_Permissions`.
/// Used so a field typed `List<Account>` resolves as target `List`
/// (matching registry entry for the collection type) rather than
/// failing lookup on the verbatim generic rendering.
fn strip_generic_suffix(api: &str) -> &str {
    match api.find('<') {
        Some(idx) => api[..idx].trim_end(),
        None => api.trim(),
    }
}

fn range_span(r: &Range) -> (u32, u32) {
    (r.end_line.saturating_sub(r.start_line), r.end_char)
}

fn range_contains(outer: &Range, inner: &Range) -> bool {
    if outer.file != inner.file {
        return false;
    }
    let o_start = (outer.start_line, outer.start_char);
    let o_end = (outer.end_line, outer.end_char);
    let i_start = (inner.start_line, inner.start_char);
    let i_end = (inner.end_line, inner.end_char);
    o_start <= i_start && i_end <= o_end
}

fn range_precedes_or_equals(a: &Range, b: &Range) -> bool {
    if a.file != b.file {
        return false;
    }
    (a.start_line, a.start_char) <= (b.start_line, b.start_char)
}

// Silence unused-field warning — `ApexClassSymbols` is imported for
// downstream crates that reference the type; actual use is through
// `registry.symbols_for(...)` which returns `Option<&ApexClassSymbols>`.
#[allow(dead_code)]
fn _force_use_apex_class_symbols(_: &ApexClassSymbols) {}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::{LocalVarDecl, LocalVarScope};
    use crate::domain::apex::class_symbols::{
        Access, ApexClassSymbols, ApexField, ApexMethod, ApexParameter, ApexTypeRef,
    };
    use crate::domain::{Node, Range};
    use std::path::PathBuf;

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

    fn method(name: &str, params: Vec<(&str, ApexTypeRef)>) -> ApexMethod {
        ApexMethod {
            name: name.to_string(),
            parameters: params
                .into_iter()
                .map(|(n, t)| ApexParameter {
                    name: n.to_string(),
                    ty: t,
                })
                .collect(),
            return_type: None,
            access: Access::Public,
            is_static: false,
            is_virtual: false,
            is_abstract: false,
        }
    }

    fn field(name: &str, ty: ApexTypeRef) -> ApexField {
        ApexField {
            name: name.to_string(),
            ty,
            access: Access::Private,
            is_static: false,
            is_final: false,
        }
    }

    fn fn_node(fqn: &str, file: &str) -> Node {
        Node::function(
            fqn.to_string(),
            Range::with_file(1, 0, 10, 0, file.to_string()),
        )
    }

    fn build_registry_with_util_permissions() -> ApexClassRegistry {
        let mut reg = ApexClassRegistry::new();
        reg.insert_user_declared(
            "UTIL_Permissions",
            super::super::class_registry::ApexTypeKind::Class,
            PathBuf::from("/x/UTIL_Permissions.cls"),
            None,
        );
        let syms = ApexClassSymbols {
            methods: vec![method("canUpdate", vec![("t", sobj("SObjectType"))])],
            ..Default::default()
        };
        reg.attach_symbols("UTIL_Permissions", syms);
        reg
    }

    #[test]
    fn normalises_this_bare_and_dotted_field() {
        assert_eq!(
            normalise_receiver("this"),
            NormalisedReceiver::SelfRef { field: None }
        );
        assert_eq!(
            normalise_receiver("this.foo"),
            NormalisedReceiver::SelfRef { field: Some("foo") }
        );
        assert_eq!(
            normalise_receiver("permissionsService"),
            NormalisedReceiver::BareIdent("permissionsService")
        );
        // Dotted receivers (other than this.<single_ident>) carry
        // the original text through so the R40 static-dispatch arm
        // can still probe the registry for a dotted type-name match.
        assert_eq!(
            normalise_receiver("a.b"),
            NormalisedReceiver::DottedDefer("a.b")
        );
        assert_eq!(
            normalise_receiver("this.a.b"),
            NormalisedReceiver::DottedDefer("this.a.b")
        );
    }

    #[test]
    fn strips_generic_suffix() {
        assert_eq!(strip_generic_suffix("UTIL_Permissions"), "UTIL_Permissions");
        assert_eq!(strip_generic_suffix("List<Account>"), "List");
        assert_eq!(strip_generic_suffix("Map<Id, Account>"), "Map");
    }

    #[test]
    fn fqn_method_and_sig_splits_correctly() {
        assert_eq!(
            fqn_method_and_sig("path::Foo::Foo::doIt(Integer)"),
            Some(("doIt", "Integer"))
        );
        assert_eq!(
            fqn_method_and_sig("path::Outer::Outer.Inner::run()"),
            Some(("run", ""))
        );
        assert_eq!(fqn_method_and_sig("path::Foo::Foo"), None);
    }

    #[test]
    fn resolves_bare_field_call_via_enclosing_class() {
        // Enclosing class `Client` has field `permissionsService: UTIL_Permissions`;
        // call is `permissionsService.canUpdate(...)` from within Client.
        let mut reg = build_registry_with_util_permissions();
        reg.insert_user_declared(
            "Client",
            super::super::class_registry::ApexTypeKind::Class,
            PathBuf::from("/x/Client.cls"),
            None,
        );
        let client_syms = ApexClassSymbols {
            fields: vec![field("permissionsService", ud("UTIL_Permissions"))],
            ..Default::default()
        };
        reg.attach_symbols("Client", client_syms);

        let canupdate_node = fn_node(
            "path::UTIL_Permissions::UTIL_Permissions::canUpdate(SObjectType)",
            "UTIL_Permissions.cls",
        );
        let caller = fn_node("path::Client::Client::run()", "Client.cls");
        let mut functions_by_name_lower: HashMap<String, Vec<&Node>> = HashMap::new();
        functions_by_name_lower
            .entry("canupdate".to_string())
            .or_default()
            .push(&canupdate_node);

        let call = CallSite {
            location: Range::with_file(5, 0, 5, 40, "Client.cls".to_string()),
            function_name: "method_call:permissionsService.canUpdate".to_string(),
            receiver_range: Some(Range::with_file(5, 0, 5, 18, "Client.cls".to_string())),
            receiver_text: Some("permissionsService".to_string()),
            arg_types: vec![sobj("SObjectType")],
        };

        let outcome = resolve_field_type_call(
            &call,
            &caller,
            "canUpdate",
            "Client",
            &reg,
            &functions_by_name_lower,
            &[],
        )
        .expect("should resolve");
        assert_eq!(outcome.callees.len(), 1);
        assert_eq!(outcome.callees[0].id, canupdate_node.id);
        assert_eq!(outcome.confidence, Confidence::Medium);
    }

    #[test]
    fn resolves_local_var_shadows_same_named_field() {
        // Field `svc: A`, local `svc: B` — local wins per Apex
        // shadowing. Call `svc.doIt()` resolves to B::doIt, not A::doIt.
        let mut reg = ApexClassRegistry::new();
        reg.insert_user_declared(
            "Client",
            super::super::class_registry::ApexTypeKind::Class,
            PathBuf::from("/x/Client.cls"),
            None,
        );
        reg.insert_user_declared(
            "A",
            super::super::class_registry::ApexTypeKind::Class,
            PathBuf::from("/x/A.cls"),
            None,
        );
        reg.insert_user_declared(
            "B",
            super::super::class_registry::ApexTypeKind::Class,
            PathBuf::from("/x/B.cls"),
            None,
        );
        reg.attach_symbols(
            "Client",
            ApexClassSymbols {
                fields: vec![field("svc", ud("A"))],
                ..Default::default()
            },
        );
        reg.attach_symbols(
            "A",
            ApexClassSymbols {
                methods: vec![method("doIt", vec![])],
                ..Default::default()
            },
        );
        reg.attach_symbols(
            "B",
            ApexClassSymbols {
                methods: vec![method("doIt", vec![])],
                ..Default::default()
            },
        );

        let a_node = fn_node("path::A::A::doIt()", "A.cls");
        let b_node = fn_node("path::B::B::doIt()", "B.cls");
        let caller = fn_node("path::Client::Client::run()", "Client.cls");
        let mut fns: HashMap<String, Vec<&Node>> = HashMap::new();
        fns.entry("doit".to_string())
            .or_default()
            .extend([&a_node, &b_node]);

        let body_range = Range::with_file(3, 0, 10, 0, "Client.cls".to_string());
        let scope = LocalVarScope {
            body: body_range.clone(),
            locals: vec![LocalVarDecl {
                name: "svc".to_string(),
                ty: ud("B"),
                declared_at: Range::with_file(4, 0, 4, 10, "Client.cls".to_string()),
            }],
        };

        let call = CallSite {
            location: Range::with_file(5, 0, 5, 10, "Client.cls".to_string()),
            function_name: "method_call:svc.doIt".to_string(),
            receiver_range: None,
            receiver_text: Some("svc".to_string()),
            arg_types: vec![],
        };

        let outcome =
            resolve_field_type_call(&call, &caller, "doIt", "Client", &reg, &fns, &[scope])
                .expect("should resolve");
        assert_eq!(outcome.callees.len(), 1);
        assert_eq!(
            outcome.callees[0].id, b_node.id,
            "local should shadow field"
        );
    }

    #[test]
    fn resolves_this_keyword_to_enclosing_class_method() {
        // `this.run()` from inside Client::caller() resolves to Client::run().
        let mut reg = ApexClassRegistry::new();
        reg.insert_user_declared(
            "Client",
            super::super::class_registry::ApexTypeKind::Class,
            PathBuf::from("/x/Client.cls"),
            None,
        );
        reg.attach_symbols(
            "Client",
            ApexClassSymbols {
                methods: vec![method("run", vec![])],
                ..Default::default()
            },
        );
        let run_node = fn_node("path::Client::Client::run()", "Client.cls");
        let caller = fn_node("path::Client::Client::caller()", "Client.cls");
        let mut fns: HashMap<String, Vec<&Node>> = HashMap::new();
        fns.entry("run".to_string()).or_default().push(&run_node);

        let call = CallSite {
            location: Range::with_file(5, 0, 5, 10, "Client.cls".to_string()),
            function_name: "method_call:this.run".to_string(),
            receiver_range: None,
            receiver_text: Some("this".to_string()),
            arg_types: vec![],
        };

        let outcome = resolve_field_type_call(&call, &caller, "run", "Client", &reg, &fns, &[])
            .expect("should resolve");
        assert_eq!(outcome.callees.len(), 1);
        assert_eq!(outcome.callees[0].id, run_node.id);
        assert_eq!(outcome.confidence, Confidence::Medium);
    }

    #[test]
    fn walks_parent_chain_for_field_and_method_lookup() {
        // Client extends Base; field `svc: UTIL_Permissions` declared on Base;
        // `svc.canUpdate(...)` from Client::run() must resolve through Base.
        let mut reg = build_registry_with_util_permissions();
        reg.insert_user_declared(
            "Base",
            super::super::class_registry::ApexTypeKind::Class,
            PathBuf::from("/x/Base.cls"),
            None,
        );
        reg.insert_user_declared(
            "Client",
            super::super::class_registry::ApexTypeKind::Class,
            PathBuf::from("/x/Client.cls"),
            None,
        );
        reg.attach_symbols(
            "Base",
            ApexClassSymbols {
                fields: vec![field("svc", ud("UTIL_Permissions"))],
                ..Default::default()
            },
        );
        reg.attach_symbols(
            "Client",
            ApexClassSymbols {
                parent_class: Some("Base".to_string()),
                ..Default::default()
            },
        );

        let canupdate_node = fn_node(
            "path::UTIL_Permissions::UTIL_Permissions::canUpdate(SObjectType)",
            "UTIL_Permissions.cls",
        );
        let caller = fn_node("path::Client::Client::run()", "Client.cls");
        let mut fns: HashMap<String, Vec<&Node>> = HashMap::new();
        fns.entry("canupdate".to_string())
            .or_default()
            .push(&canupdate_node);

        let call = CallSite {
            location: Range::with_file(5, 0, 5, 40, "Client.cls".to_string()),
            function_name: "method_call:svc.canUpdate".to_string(),
            receiver_range: None,
            receiver_text: Some("svc".to_string()),
            arg_types: vec![sobj("SObjectType")],
        };

        let outcome =
            resolve_field_type_call(&call, &caller, "canUpdate", "Client", &reg, &fns, &[])
                .expect("parent-chain field lookup should resolve");
        assert_eq!(outcome.callees[0].id, canupdate_node.id);
    }

    #[test]
    fn declines_on_dotted_receiver() {
        // `a.b.c.m()` — receiver is `a.b.c`. After the R40 (PR 8) static-
        // dispatch arm, dotted receivers no longer auto-defer; they probe
        // the registry as a possible TypeName. `a.b.c` is not registered
        // here, so resolution returns None (no wrong edge emitted).
        let reg = build_registry_with_util_permissions();
        let caller = fn_node("path::Client::Client::run()", "Client.cls");
        let call = CallSite {
            location: Range::with_file(5, 0, 5, 10, "Client.cls".to_string()),
            function_name: "method_call:a.b.c.m".to_string(),
            receiver_range: None,
            receiver_text: Some("a.b.c".to_string()),
            arg_types: vec![],
        };
        let outcome =
            resolve_field_type_call(&call, &caller, "m", "Client", &reg, &HashMap::new(), &[]);
        assert!(outcome.is_none());
    }

    #[test]
    fn declines_when_receiver_type_has_no_symbols() {
        // Receiver has a declared type but no symbols are attached yet
        // (system type / managed-package stub). Resolver declines so the
        // fallback path takes over; no wrong edge emitted.
        let reg = ApexClassRegistry::with_standard_preload();
        let caller = fn_node("path::Client::Client::run()", "Client.cls");
        let scope = LocalVarScope {
            body: Range::with_file(3, 0, 10, 0, "Client.cls".to_string()),
            locals: vec![LocalVarDecl {
                name: "req".to_string(),
                ty: ud("Database"),
                declared_at: Range::with_file(4, 0, 4, 10, "Client.cls".to_string()),
            }],
        };
        let call = CallSite {
            location: Range::with_file(5, 0, 5, 10, "Client.cls".to_string()),
            function_name: "method_call:req.query".to_string(),
            receiver_range: None,
            receiver_text: Some("req".to_string()),
            arg_types: vec![],
        };
        let outcome = resolve_field_type_call(
            &call,
            &caller,
            "query",
            "Client",
            &reg,
            &HashMap::new(),
            &[scope],
        );
        // `Database` is preloaded as a system type with no symbols —
        // registry.symbols_for returns None → resolver declines.
        assert!(outcome.is_none());
    }

    #[test]
    fn overload_dispatch_picks_exact_match() {
        // `svc.compare("x","y")` with two overloads (Object,Object) and
        // (String,String) → picks String overload.
        let mut reg = ApexClassRegistry::new();
        reg.insert_user_declared(
            "Svc",
            super::super::class_registry::ApexTypeKind::Class,
            PathBuf::from("/x/Svc.cls"),
            None,
        );
        reg.insert_user_declared(
            "Client",
            super::super::class_registry::ApexTypeKind::Class,
            PathBuf::from("/x/Client.cls"),
            None,
        );
        reg.attach_symbols(
            "Svc",
            ApexClassSymbols {
                methods: vec![
                    method(
                        "compare",
                        vec![("a", prim("Object")), ("b", prim("Object"))],
                    ),
                    method(
                        "compare",
                        vec![("a", prim("String")), ("b", prim("String"))],
                    ),
                ],
                ..Default::default()
            },
        );
        reg.attach_symbols(
            "Client",
            ApexClassSymbols {
                fields: vec![field("svc", ud("Svc"))],
                ..Default::default()
            },
        );

        let obj_overload = fn_node("path::Svc::Svc::compare(Object,Object)", "Svc.cls");
        let str_overload = fn_node("path::Svc::Svc::compare(String,String)", "Svc.cls");
        let caller = fn_node("path::Client::Client::run()", "Client.cls");
        let mut fns: HashMap<String, Vec<&Node>> = HashMap::new();
        fns.entry("compare".to_string())
            .or_default()
            .extend([&obj_overload, &str_overload]);

        let call = CallSite {
            location: Range::with_file(5, 0, 5, 30, "Client.cls".to_string()),
            function_name: "method_call:svc.compare".to_string(),
            receiver_range: None,
            receiver_text: Some("svc".to_string()),
            arg_types: vec![prim("String"), prim("String")],
        };

        let outcome = resolve_field_type_call(&call, &caller, "compare", "Client", &reg, &fns, &[])
            .expect("should resolve");
        assert_eq!(outcome.callees.len(), 1);
        assert_eq!(outcome.callees[0].id, str_overload.id);
        assert_eq!(outcome.confidence, Confidence::Medium);
    }
}
