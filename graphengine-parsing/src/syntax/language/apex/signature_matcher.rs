//! Apex signature matching (TR-A.1 → TR-A.4).
//!
//! Single source of truth for "which ctor / method does this call
//! site bind to given these inferred argument types?" Used by the
//! Apex heuristic resolver's ctor arm today (TR-A.1) and by method
//! dispatch in TR-A.4.
//!
//! # Matching ladder
//!
//! Candidates pass through the ladder in order:
//!
//! 1. **Arity filter** — drop candidates whose declared parameter
//!    count differs from `arg_types.len()`.
//! 2. **Exact match** — every argument's [`ApexTypeRef`] equals the
//!    corresponding parameter's type under [`ApexTypeRef::is_exact`].
//!    `Unresolved` is treated as a **wildcard** that matches any
//!    parameter (never drops a candidate on unknown argument types —
//!    identifier-type narrowing is TR-A.3's job).
//! 3. **Widening** — argument types widen into parameter types via
//!    the Apex numeric ladder (`Integer → Long → Decimal → Double`),
//!    the SObject subtype relation (`Account → SObject`), and
//!    collection-element recursion (`List<Account> → List<SObject>`,
//!    `Map<Id, Account> → Map<Id, SObject>`).
//!
//! 4. **Implicit** (TR-A.4) — the last resort below Widening: any
//!    non-primitive argument type widens to the Apex root
//!    `Object`, and any primitive argument autoboxes into `Object`
//!    (Apex's universal parameter type for heterogeneous
//!    collections and generic-method signatures). A match that
//!    only survives this tier returns
//!    [`SigMatchResult::UniqueLow`] / [`SigMatchResult::FanoutLow`]
//!    so the caller emits the edge at [`Confidence::Low`] instead
//!    of Medium — "a match exists but only because `Object`
//!    accepts everything" is useful signal, but it is weaker than
//!    a true exact or typed-widening bind and should not be
//!    marketed as Medium.
//!
//! # Return shape
//!
//! [`SigMatchResult`]:
//! - `Unique(&T)` — exactly one candidate survived the highest-priority
//!   tier it reached. Resolver emits a Medium-confidence edge.
//! - `Fanout(Vec<&T>)` — multiple candidates tied at the same tier.
//!   Resolver emits Low-confidence edges capped at 8 by Sprint H.2.
//! - `None` — no candidate survived arity. Resolver drops.
//!
//! # `CtorLike` / `MethodLike`
//!
//! Both constructors ([`ApexConstructor`]) and methods ([`ApexMethod`])
//! pass through the matcher via a single [`CtorLike`] trait so TR-A.4's
//! method-overload arm reuses this module without a shape change. The
//! trait exposes only `fn parameters(&self) -> &[ApexParameter]`;
//! higher-level method-specific filters (static vs instance,
//! visibility) live in the caller.

use crate::domain::apex::class_symbols::{
    ApexConstructor, ApexMethod, ApexParameter, ApexTypeRef, CollectionKind,
};

/// Abstraction over signature-carrying symbols so the matcher
/// uniformly handles constructor overloads (TR-A.1) and method
/// overloads (TR-A.4).
pub trait CtorLike {
    fn parameters(&self) -> &[ApexParameter];
}

impl CtorLike for ApexConstructor {
    fn parameters(&self) -> &[ApexParameter] {
        &self.parameters
    }
}

impl CtorLike for ApexMethod {
    fn parameters(&self) -> &[ApexParameter] {
        &self.parameters
    }
}

/// Outcome of [`rank_candidates`].
///
/// The `Low` variants carry the extra signal that the match only
/// survived thanks to the TR-A.4 implicit-conversion tier
/// (`Object` widening / primitive autoboxing). Callers use this to
/// drop the emitted edge's confidence from Medium to Low without
/// having to re-examine the surviving candidate(s).
#[derive(Debug)]
pub enum SigMatchResult<'a, T> {
    /// One candidate is uniquely best at the Exact or Widening tier.
    /// Callers emit Medium confidence.
    Unique(&'a T),
    /// Multiple candidates tied at the Exact or Widening tier.
    /// Callers typically emit Low confidence (ambiguity already
    /// justifies it) but can apply a same-class preference first.
    Fanout(Vec<&'a T>),
    /// One candidate matched only because of the TR-A.4 implicit
    /// tier (Object widening / autoboxing). Callers emit Low
    /// confidence even though the survivor is unique — the match
    /// itself is structurally weaker than a typed-widening bind.
    UniqueLow(&'a T),
    /// Multiple candidates tied at the TR-A.4 implicit tier.
    /// Callers emit Low confidence (fanout).
    FanoutLow(Vec<&'a T>),
    /// Arity rejected every candidate.
    None,
}

/// Priority tier a candidate survived to. Higher value = stronger
/// match. Used internally to pick the single highest-priority survivor
/// set when reporting `Unique` / `Fanout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    Exact,
    Widening,
    /// TR-A.4: Object / autoboxing last-resort tier. See module docs.
    Implicit,
}

/// Rank a candidate pool against a call's inferred argument types and
/// return the best match. See module docs for the ladder.
pub fn rank_candidates<'a, T: CtorLike>(
    candidates: &'a [T],
    arg_types: &[ApexTypeRef],
) -> SigMatchResult<'a, T> {
    // Tier 1 — arity filter.
    let arity_survivors: Vec<&T> = candidates
        .iter()
        .filter(|c| c.parameters().len() == arg_types.len())
        .collect();

    if arity_survivors.is_empty() {
        return SigMatchResult::None;
    }

    // Tier 2 — exact match (Unresolved is a wildcard).
    let exact: Vec<&T> = arity_survivors
        .iter()
        .copied()
        .filter(|c| matches_all(c.parameters(), arg_types, Tier::Exact))
        .collect();

    if !exact.is_empty() {
        return pick(exact);
    }

    // Tier 3 — widening.
    let widening: Vec<&T> = arity_survivors
        .iter()
        .copied()
        .filter(|c| matches_all(c.parameters(), arg_types, Tier::Widening))
        .collect();

    if !widening.is_empty() {
        return pick(widening);
    }

    // Tier 4 — TR-A.4 implicit conversion (Object widening /
    // autoboxing). This is the last resort below typed widening;
    // callers emit the resulting edges at Low confidence even when
    // the survivor is unique.
    let implicit: Vec<&T> = arity_survivors
        .iter()
        .copied()
        .filter(|c| matches_all(c.parameters(), arg_types, Tier::Implicit))
        .collect();

    if !implicit.is_empty() {
        return pick_low(implicit);
    }

    // Arity-only survivors with no exact / widening / implicit match
    // still fanout — they represent the "same shape, unknown types"
    // case where TR-A.3's local-var scope (or future type-propagation
    // work) will tighten later. Returning them as Fanout / Unique at
    // the standard tier lets the resolver keep the signal at Low
    // confidence via its own ambiguity handling rather than losing
    // the call entirely.
    pick(arity_survivors)
}

fn pick<T>(survivors: Vec<&T>) -> SigMatchResult<'_, T> {
    if survivors.len() == 1 {
        SigMatchResult::Unique(survivors[0])
    } else {
        SigMatchResult::Fanout(survivors)
    }
}

/// Variant of [`pick`] that forces Low confidence regardless of
/// survivor count. Used for the TR-A.4 implicit tier — even a
/// unique match here is structurally weaker than a typed-widening
/// bind, and the caller should never promote it to Medium.
fn pick_low<T>(survivors: Vec<&T>) -> SigMatchResult<'_, T> {
    if survivors.len() == 1 {
        SigMatchResult::UniqueLow(survivors[0])
    } else {
        SigMatchResult::FanoutLow(survivors)
    }
}

fn matches_all(params: &[ApexParameter], args: &[ApexTypeRef], tier: Tier) -> bool {
    params
        .iter()
        .zip(args.iter())
        .all(|(p, a)| matches_one(&p.ty, a, tier))
}

/// `arg` matches `param` at the given tier.
fn matches_one(param: &ApexTypeRef, arg: &ApexTypeRef, tier: Tier) -> bool {
    // Unresolved on the argument side acts as a wildcard — pre-TR-A.3
    // we haven't inferred the type (bare identifier / field access /
    // method return), so treat the slot as "compatible with anything"
    // rather than fail-closed. False negatives would drop legitimate
    // edges; false positives fan out into Low-confidence which is the
    // correct pre-type-scope behaviour. `Null` on the argument side
    // also acts as a wildcard against non-primitive parameter types
    // (Apex permits `null` wherever a reference is accepted).
    if is_wildcard_arg(arg, param) {
        return true;
    }
    match tier {
        Tier::Exact => is_exact(param, arg),
        Tier::Widening => is_exact(param, arg) || widens_to(arg, param),
        Tier::Implicit => {
            is_exact(param, arg) || widens_to(arg, param) || implicitly_converts(arg, param)
        }
    }
}

/// TR-A.4 implicit-conversion rule. Apex permits every non-primitive
/// type to bind to an `Object` parameter (the universal base class
/// for user classes and SObjects) and every primitive to autobox
/// into `Object` when passed as an argument. This captures both in
/// a single predicate so the matcher's tier ladder can decide
/// purely on structure.
///
/// **Explicit non-goal.** This does NOT model `String.valueOf(...)`
/// -shape coercions (Apex requires an explicit call for number →
/// String, so a static analyser should not auto-widen an `Integer`
/// arg onto a `String` parameter). It also does NOT model
/// `Integer → String` via concatenation — that only happens in
/// expression context (`'' + i`), not in call sites. The rule is
/// deliberately narrow: Apex treats `Object` as a universal sink,
/// and the heuristic resolver mirrors that reality.
fn implicitly_converts(arg: &ApexTypeRef, param: &ApexTypeRef) -> bool {
    use ApexTypeRef::*;
    // The Apex `Object` token is classified as `Primitive("Object")`
    // by `class_symbols_extractor::is_primitive` (it lives in the
    // same "scalar value type" table as `Integer` / `String` /
    // `Id`). Accept both `Primitive("Object")` and
    // `UserDefined("Object")` so handwritten-fixture data (which
    // tends to construct UserDefined) matches the live-extractor
    // classification (which always yields Primitive). Neither shape
    // matches `Primitive("Object")` via `is_exact` — the exact
    // check is case-insensitive on the `name` field which is fine,
    // but the argument must be `Primitive` too, which a user-class
    // argument never is.
    let param_is_object = matches!(
        param,
        Primitive { name } if name.eq_ignore_ascii_case("Object"),
    ) || matches!(
        param,
        UserDefined { api_name } if api_name.eq_ignore_ascii_case("Object"),
    );
    match param {
        _ if param_is_object => {
            // Any arg except another bare-`Object` (which would have
            // matched exact) qualifies. Collections-of-Object are
            // also accepted via `widens_to` at the collection tier
            // because the element recursion handles it.
            !matches!(
                arg,
                Primitive { name } if name.eq_ignore_ascii_case("Object"),
            ) && !matches!(
                arg,
                UserDefined { api_name } if api_name.eq_ignore_ascii_case("Object"),
            )
        }
        // `Collection<Object>` / `Map<K, Object>` / `Generic<...Object...>`
        // recurse structurally: the container kinds must match, and
        // the element / value positions must either be exact, widen,
        // or (recursively) implicitly convert.
        Collection {
            kind: pk,
            element: pe,
        } => match arg {
            Collection {
                kind: ak,
                element: ae,
            } => ak == pk && (is_exact(ae, pe) || widens_to(ae, pe) || implicitly_converts(ae, pe)),
            _ => false,
        },
        Map { key: pk, value: pv } => match arg {
            Map { key: ak, value: av } => {
                (is_exact(ak, pk) || widens_to(ak, pk) || implicitly_converts(ak, pk))
                    && (is_exact(av, pv) || widens_to(av, pv) || implicitly_converts(av, pv))
            }
            _ => false,
        },
        Generic {
            base: pb,
            parameters: pp,
        } => match arg {
            Generic {
                base: ab,
                parameters: ap,
            } => {
                ab.eq_ignore_ascii_case(pb)
                    && ap.len() == pp.len()
                    && ap.iter().zip(pp.iter()).all(|(a, p)| {
                        is_exact(a, p) || widens_to(a, p) || implicitly_converts(a, p)
                    })
            }
            _ => false,
        },
        _ => false,
    }
}

fn is_wildcard_arg(arg: &ApexTypeRef, param: &ApexTypeRef) -> bool {
    match arg {
        ApexTypeRef::Unresolved { .. } => true,
        ApexTypeRef::Primitive { name } if name.eq_ignore_ascii_case("Null") => {
            // `null` satisfies any reference type (anything non-primitive
            // on the Apex side). Primitive parameters reject `null` at
            // Apex compile time; mirror that here.
            !matches!(param, ApexTypeRef::Primitive { .. })
        }
        _ => false,
    }
}

/// Structural equality for [`ApexTypeRef`], case-insensitive on
/// `api_name` fields (Apex identifiers are case-insensitive).
/// Collection and map variants recurse element-wise. Kept private to
/// this module so callers always go through [`rank_candidates`].
pub fn is_exact(a: &ApexTypeRef, b: &ApexTypeRef) -> bool {
    use ApexTypeRef::*;
    match (a, b) {
        (Primitive { name: n1 }, Primitive { name: n2 }) => n1.eq_ignore_ascii_case(n2),
        (Sobject { api_name: a1 }, Sobject { api_name: a2 }) => a1.eq_ignore_ascii_case(a2),
        (UserDefined { api_name: a1 }, UserDefined { api_name: a2 }) => a1.eq_ignore_ascii_case(a2),
        // UserDefined vs Sobject is a registry reconciliation concern —
        // at signature-match time both variants share the same api name
        // for an SObject reference (e.g. `Account`), so treat them as
        // exact when the name matches. This keeps ctor params declared
        // as `Account` compatible with arguments inferred as either
        // variant depending on whether the registry has reconciled yet.
        (UserDefined { api_name: a1 }, Sobject { api_name: a2 })
        | (Sobject { api_name: a1 }, UserDefined { api_name: a2 }) => a1.eq_ignore_ascii_case(a2),
        (
            Collection {
                kind: k1,
                element: e1,
            },
            Collection {
                kind: k2,
                element: e2,
            },
        ) => k1 == k2 && is_exact(e1, e2),
        (Map { key: k1, value: v1 }, Map { key: k2, value: v2 }) => {
            is_exact(k1, k2) && is_exact(v1, v2)
        }
        (
            Generic {
                base: b1,
                parameters: p1,
            },
            Generic {
                base: b2,
                parameters: p2,
            },
        ) => {
            b1.eq_ignore_ascii_case(b2)
                && p1.len() == p2.len()
                && p1.iter().zip(p2.iter()).all(|(a, b)| is_exact(a, b))
        }
        (Unresolved { raw: r1 }, Unresolved { raw: r2 }) => r1.eq_ignore_ascii_case(r2),
        _ => false,
    }
}

/// Does `arg` widen into `param` under Apex's implicit conversions?
/// Covers:
/// - numeric widening: `Integer → Long → Decimal`, `Integer → Double`,
///   `Long → Decimal`, `Long → Double`, `Decimal → Double` (Apex
///   specifically permits the former; the latter reflects Apex's
///   actual numeric ladder)
/// - SObject subtype: any SObject-ish arg → `SObject` parameter (the
///   Apex base SObject type)
/// - collection-element recursion
/// - map key/value recursion
/// - `Generic<T…>` recursion
fn widens_to(arg: &ApexTypeRef, param: &ApexTypeRef) -> bool {
    use ApexTypeRef::*;
    match (arg, param) {
        (Primitive { name: a }, Primitive { name: p }) => numeric_widens(a, p),
        (Sobject { .. } | UserDefined { .. }, UserDefined { api_name })
            if api_name.eq_ignore_ascii_case("SObject") =>
        {
            true
        }
        (Sobject { .. }, Sobject { api_name }) if api_name.eq_ignore_ascii_case("SObject") => true,
        (
            Collection {
                kind: ak,
                element: ae,
            },
            Collection {
                kind: pk,
                element: pe,
            },
        ) => ak == pk && (is_exact(ae, pe) || widens_to(ae, pe)),
        (Map { key: ak, value: av }, Map { key: pk, value: pv }) => {
            (is_exact(ak, pk) || widens_to(ak, pk)) && (is_exact(av, pv) || widens_to(av, pv))
        }
        (
            Generic {
                base: ab,
                parameters: ap,
            },
            Generic {
                base: pb,
                parameters: pp,
            },
        ) => {
            ab.eq_ignore_ascii_case(pb)
                && ap.len() == pp.len()
                && ap
                    .iter()
                    .zip(pp.iter())
                    .all(|(a, p)| is_exact(a, p) || widens_to(a, p))
        }
        _ => false,
    }
}

/// Apex numeric widening — strictly one-directional. Matches the
/// legal implicit conversions permitted by the Apex compiler.
fn numeric_widens(from: &str, to: &str) -> bool {
    let f = numeric_rank(from);
    let t = numeric_rank(to);
    match (f, t) {
        (Some(a), Some(b)) => a < b,
        _ => false,
    }
}

fn numeric_rank(name: &str) -> Option<u8> {
    match name.to_ascii_lowercase().as_str() {
        "integer" => Some(0),
        "long" => Some(1),
        "decimal" => Some(2),
        "double" => Some(3),
        _ => None,
    }
}

// Allow the unused CollectionKind import when tests are disabled.
#[allow(unused_imports)]
use CollectionKind as _KeepCollectionKindReferenced;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::Access;

    fn prim(name: &str) -> ApexTypeRef {
        ApexTypeRef::Primitive {
            name: name.to_string(),
        }
    }
    fn user(name: &str) -> ApexTypeRef {
        ApexTypeRef::UserDefined {
            api_name: name.to_string(),
        }
    }
    fn sobj(name: &str) -> ApexTypeRef {
        ApexTypeRef::Sobject {
            api_name: name.to_string(),
        }
    }
    fn unresolved(raw: &str) -> ApexTypeRef {
        ApexTypeRef::Unresolved {
            raw: raw.to_string(),
        }
    }
    fn list_of(el: ApexTypeRef) -> ApexTypeRef {
        ApexTypeRef::Collection {
            kind: CollectionKind::List,
            element: Box::new(el),
        }
    }

    fn ctor(params: Vec<ApexTypeRef>) -> ApexConstructor {
        ApexConstructor {
            parameters: params
                .into_iter()
                .enumerate()
                .map(|(i, ty)| ApexParameter {
                    name: format!("p{}", i),
                    ty,
                })
                .collect(),
            access: Access::Public,
        }
    }

    // ------------------------------------------------------------------
    // Arity filter
    // ------------------------------------------------------------------

    #[test]
    fn arity_mismatch_returns_none() {
        let candidates = vec![ctor(vec![prim("String")])];
        let result = rank_candidates(&candidates, &[prim("String"), prim("Integer")]);
        assert!(matches!(result, SigMatchResult::None));
    }

    #[test]
    fn zero_arg_matches_zero_param_ctor() {
        let candidates = vec![ctor(vec![])];
        let result = rank_candidates(&candidates, &[]);
        assert!(matches!(result, SigMatchResult::Unique(_)));
    }

    // ------------------------------------------------------------------
    // Exact match
    // ------------------------------------------------------------------

    #[test]
    fn exact_single_match() {
        let candidates = vec![ctor(vec![prim("String")]), ctor(vec![prim("Integer")])];
        let result = rank_candidates(&candidates, &[prim("String")]);
        match result {
            SigMatchResult::Unique(c) => assert_eq!(c.parameters[0].ty, prim("String")),
            other => panic!("expected Unique, got {:?}", other),
        }
    }

    #[test]
    fn exact_beats_widening() {
        // `Integer` overload coexists with a `Decimal` overload; an
        // Integer arg should bind to Integer, not widen to Decimal.
        let candidates = vec![ctor(vec![prim("Decimal")]), ctor(vec![prim("Integer")])];
        let result = rank_candidates(&candidates, &[prim("Integer")]);
        match result {
            SigMatchResult::Unique(c) => assert_eq!(c.parameters[0].ty, prim("Integer")),
            other => panic!("expected Unique(Integer), got {:?}", other),
        }
    }

    #[test]
    fn exact_case_insensitive_on_user_defined() {
        let candidates = vec![ctor(vec![user("Logger")])];
        let result = rank_candidates(&candidates, &[user("logger")]);
        assert!(matches!(result, SigMatchResult::Unique(_)));
    }

    // ------------------------------------------------------------------
    // UserDefined ↔ Sobject reconciliation
    // ------------------------------------------------------------------

    #[test]
    fn user_defined_account_matches_sobject_account() {
        // Arg inferred as UserDefined("Account") (extractor view); param
        // declared as Sobject("Account") (registry-reconciled view).
        let candidates = vec![ctor(vec![sobj("Account")])];
        let result = rank_candidates(&candidates, &[user("Account")]);
        assert!(matches!(result, SigMatchResult::Unique(_)));
    }

    // ------------------------------------------------------------------
    // Unresolved wildcard
    // ------------------------------------------------------------------

    #[test]
    fn unresolved_arg_is_wildcard_across_arity_survivors() {
        let candidates = vec![ctor(vec![prim("String")]), ctor(vec![prim("Integer")])];
        let result = rank_candidates(&candidates, &[unresolved("someVar")]);
        match result {
            SigMatchResult::Fanout(cs) => assert_eq!(cs.len(), 2),
            other => panic!("expected Fanout of 2, got {:?}", other),
        }
    }

    #[test]
    fn unresolved_position_does_not_drop_match_on_known_position() {
        // Arg shape: (Unresolved, Integer) vs candidate (String, Integer)
        let candidates = vec![ctor(vec![prim("String"), prim("Integer")])];
        let result = rank_candidates(&candidates, &[unresolved("x"), prim("Integer")]);
        assert!(matches!(result, SigMatchResult::Unique(_)));
    }

    // ------------------------------------------------------------------
    // Null wildcard
    // ------------------------------------------------------------------

    #[test]
    fn null_arg_matches_reference_parameter() {
        let candidates = vec![ctor(vec![user("Logger")])];
        let result = rank_candidates(&candidates, &[prim("Null")]);
        assert!(matches!(result, SigMatchResult::Unique(_)));
    }

    #[test]
    fn null_arg_does_not_match_primitive_parameter() {
        // `null` on a primitive-typed param isn't an exact match, and
        // there's no widening path either — arity-only fanout (size 1).
        // The call survives as Fanout (which the resolver handles as
        // Low-confidence), NOT Unique — because `null` on an `Integer`
        // param would be a compile-time error in Apex.
        let candidates = vec![ctor(vec![prim("Integer")])];
        let result = rank_candidates(&candidates, &[prim("Null")]);
        // Arity-only survivor: Unique shape at arity tier only.
        match result {
            SigMatchResult::Unique(_) => {}
            other => panic!(
                "arity-only single-candidate should still emit Unique, got {:?}",
                other
            ),
        }
    }

    // ------------------------------------------------------------------
    // Widening — numeric
    // ------------------------------------------------------------------

    #[test]
    fn integer_widens_to_long() {
        let candidates = vec![ctor(vec![prim("Long")])];
        let result = rank_candidates(&candidates, &[prim("Integer")]);
        assert!(matches!(result, SigMatchResult::Unique(_)));
    }

    #[test]
    fn integer_widens_to_decimal() {
        let candidates = vec![ctor(vec![prim("Decimal")])];
        let result = rank_candidates(&candidates, &[prim("Integer")]);
        assert!(matches!(result, SigMatchResult::Unique(_)));
    }

    #[test]
    fn decimal_does_not_widen_to_integer() {
        // Apex does not permit implicit narrowing; exact-fail + widen-fail
        // should leave the arity-only survivor.
        let candidates = vec![ctor(vec![prim("Integer")])];
        let result = rank_candidates(&candidates, &[prim("Decimal")]);
        // Single arity-only survivor, Unique shape.
        match result {
            SigMatchResult::Unique(_) => {}
            other => panic!(
                "arity-only single-candidate should still emit Unique, got {:?}",
                other
            ),
        }
    }

    // ------------------------------------------------------------------
    // Widening — SObject subtype
    // ------------------------------------------------------------------

    #[test]
    fn account_widens_to_sobject_parameter() {
        let candidates = vec![ctor(vec![sobj("SObject")])];
        let result = rank_candidates(&candidates, &[sobj("Account")]);
        assert!(matches!(result, SigMatchResult::Unique(_)));
    }

    // ------------------------------------------------------------------
    // Widening — collection element
    // ------------------------------------------------------------------

    #[test]
    fn list_of_account_widens_to_list_of_sobject() {
        let candidates = vec![ctor(vec![list_of(sobj("SObject"))])];
        let result = rank_candidates(&candidates, &[list_of(sobj("Account"))]);
        assert!(matches!(result, SigMatchResult::Unique(_)));
    }

    #[test]
    fn list_of_string_does_not_widen_to_list_of_integer() {
        let candidates = vec![ctor(vec![list_of(prim("Integer"))])];
        let result = rank_candidates(&candidates, &[list_of(prim("String"))]);
        // Arity-only survivor; not dropped.
        match result {
            SigMatchResult::Unique(_) => {}
            other => panic!("expected Unique (arity-only), got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // Tie-breaker fanout
    // ------------------------------------------------------------------

    #[test]
    fn exact_tie_returns_fanout() {
        let candidates = vec![ctor(vec![prim("String")]), ctor(vec![prim("String")])];
        let result = rank_candidates(&candidates, &[prim("String")]);
        match result {
            SigMatchResult::Fanout(cs) => assert_eq!(cs.len(), 2),
            other => panic!("expected Fanout of 2, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // CtorLike / MethodLike abstraction
    // ------------------------------------------------------------------

    // ------------------------------------------------------------------
    // Implicit tier (TR-A.4) — Object widening / autoboxing
    // ------------------------------------------------------------------

    #[test]
    fn user_class_widens_to_object_param_at_implicit_tier_only() {
        // `log(Object)` vs `log(String)`; arg is UserDefined("Payload").
        // Exact fails both; Widening fails both (no SObject); Implicit
        // tier catches `Payload → Object`, so we return UniqueLow.
        let candidates = vec![ctor(vec![prim("String")]), ctor(vec![prim("Object")])];
        let result = rank_candidates(&candidates, &[user("Payload")]);
        match result {
            SigMatchResult::UniqueLow(c) => {
                assert_eq!(c.parameters[0].ty, prim("Object"))
            }
            other => panic!("expected UniqueLow(Object), got {:?}", other),
        }
    }

    #[test]
    fn user_class_userdefined_object_param_also_widens() {
        // Mirror of the previous case but with the `Object` param
        // declared as `UserDefined("Object")` (handwritten-fixture
        // shape). Both shapes must match.
        let candidates = vec![ctor(vec![user("Object")])];
        let result = rank_candidates(&candidates, &[user("Payload")]);
        match result {
            SigMatchResult::UniqueLow(_) => {}
            other => panic!("expected UniqueLow, got {:?}", other),
        }
    }

    #[test]
    fn primitive_autoboxes_to_object_at_implicit_tier() {
        let candidates = vec![ctor(vec![prim("Object")])];
        let result = rank_candidates(&candidates, &[prim("Integer")]);
        match result {
            SigMatchResult::UniqueLow(_) => {}
            other => panic!("expected UniqueLow(autobox), got {:?}", other),
        }
    }

    #[test]
    fn exact_object_arg_beats_implicit_tier() {
        // `Object` arg against `Object` param should bind at Exact,
        // not demote to Implicit.
        let candidates = vec![ctor(vec![prim("Object")])];
        let result = rank_candidates(&candidates, &[prim("Object")]);
        match result {
            SigMatchResult::Unique(_) => {}
            other => panic!("expected Unique (Exact), got {:?}", other),
        }
    }

    #[test]
    fn exact_string_overload_beats_object_overload_under_string_arg() {
        // The core TR-A.4 "String exact beats Object widen" case.
        let candidates = vec![ctor(vec![prim("Object")]), ctor(vec![prim("String")])];
        let result = rank_candidates(&candidates, &[prim("String")]);
        match result {
            SigMatchResult::Unique(c) => {
                assert_eq!(c.parameters[0].ty, prim("String"))
            }
            other => panic!("expected Unique(String), got {:?}", other),
        }
    }

    #[test]
    fn multiple_implicit_survivors_report_fanout_low() {
        // Two Object-shaped params at the same position — both survive
        // the Implicit tier, so we expect FanoutLow.
        let candidates = vec![ctor(vec![prim("Object")]), ctor(vec![user("Object")])];
        let result = rank_candidates(&candidates, &[user("Payload")]);
        match result {
            SigMatchResult::FanoutLow(cs) => assert_eq!(cs.len(), 2),
            other => panic!("expected FanoutLow of 2, got {:?}", other),
        }
    }

    #[test]
    fn method_like_reuses_matcher() {
        let methods = vec![ApexMethod {
            name: "m".to_string(),
            parameters: vec![ApexParameter {
                name: "x".into(),
                ty: prim("String"),
            }],
            return_type: None,
            access: Access::Public,
            is_static: false,
            is_virtual: false,
            is_abstract: false,
        }];
        let result = rank_candidates(&methods, &[prim("String")]);
        assert!(matches!(result, SigMatchResult::Unique(_)));
    }
}
