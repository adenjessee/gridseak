//! Argument-type narrowing (TR-A.4).
//!
//! Pure post-processing over the [`ApexTypeRef`]s that the
//! [`arg_type_inferrer`](super::arg_type_inferrer) emits at call-site
//! extraction time. Replaces `Unresolved { raw: <ident> }` slots with
//! a concrete type pulled from the caller's scope, in the same
//! lookup order the field-type-aware receiver resolver uses:
//!
//!   1. **Local scope** — a local variable or formal parameter of
//!      the enclosing method/constructor body, declared at or
//!      before the call-site. Most-recent declaration wins.
//!   2. **Enclosing class fields (+ parent chain)** — a field on the
//!      enclosing class, or any ancestor reachable via the
//!      `parent_class` link, up to
//!      [`PARENT_CHAIN_MAX_DEPTH`] hops.
//!
//! # Why this exists
//!
//! The arg-type inferrer runs against bare syntax and cannot see
//! local / field / parent-chain declarations — identifier-typed
//! arguments always fall through to `Unresolved { raw }`. The
//! signature matcher treats `Unresolved` as a wildcard, so overload
//! dispatch on calls like `cmp.compare(s1, s2)` fanouts across all
//! arity-matching overloads instead of picking the correct one.
//!
//! TR-A.4's overload-dispatch guarantees depend on identifier
//! arguments narrowing to their declared types before the matcher
//! runs. This module is the one deterministic place that
//! narrowing happens, so the constructor arm (TR-A.1) and the
//! field-type-aware method arm (TR-A.3/TR-A.4) share a single
//! source of truth.
//!
//! # Non-goals
//!
//! - **Method-return-type propagation** — `log(getFoo())` still
//!   returns `Unresolved`; that's Phase B scope.
//! - **Dotted identifiers** — `log(a.b)` stays `Unresolved`;
//!   TR-A.6's containment walker owns receiver-chain traversal and
//!   will later extend this module to cover `this.<field>`-rooted
//!   chains. Today's narrower accepts only single-segment
//!   identifiers because `arg_type_inferrer` collapses whitespace
//!   in `Unresolved.raw` (so a dotted arg still carries the `.`),
//!   keeping the filter trivial and safe.
//! - **Ambiguous shadowing** — Apex's for-loop variable rule is
//!   honoured via the local-scope innermost-range pick; the
//!   declared-at-or-before filter preserves source order.
//!
//! The narrower is a no-op on every arg that is already resolved
//! (literals, `new` expressions, SObject-type tokens). The narrower
//! is also a no-op on `Unresolved.raw` that doesn't look like an
//! identifier (dotted text, empty, or contains whitespace → leave
//! as wildcard). Anything we cannot prove a type for stays
//! `Unresolved`, which keeps the signature matcher's wildcard
//! semantics intact (no false-negative candidate drops).

use crate::application::ports::LocalVarScope;
use crate::domain::apex::class_symbols::ApexTypeRef;
use crate::domain::Range;

use super::class_registry::ApexClassRegistry;

/// Same cap used by
/// [`field_type_resolver`](super::field_type_resolver) — keep them
/// in sync. Guards against cyclical `parent_class` declarations in
/// malformed source.
const PARENT_CHAIN_MAX_DEPTH: usize = 16;

/// Narrow every `Unresolved` identifier argument in `arg_types`
/// against the caller's local scope + enclosing-class fields +
/// parent chain. Returns a new vector; the input is not mutated so
/// callers can keep the original for logging / telemetry.
pub(super) fn narrow_arg_types(
    arg_types: &[ApexTypeRef],
    call_location: &Range,
    enclosing_type_api: &str,
    registry: &ApexClassRegistry,
    local_var_scopes: &[LocalVarScope],
) -> Vec<ApexTypeRef> {
    arg_types
        .iter()
        .map(|a| {
            narrow_one(
                a,
                call_location,
                enclosing_type_api,
                registry,
                local_var_scopes,
            )
        })
        .collect()
}

fn narrow_one(
    arg: &ApexTypeRef,
    call_location: &Range,
    enclosing_type_api: &str,
    registry: &ApexClassRegistry,
    local_var_scopes: &[LocalVarScope],
) -> ApexTypeRef {
    let ApexTypeRef::Unresolved { raw } = arg else {
        return arg.clone();
    };
    let Some(ident) = as_bare_identifier(raw) else {
        return arg.clone();
    };

    if let Some(ty) = lookup_in_local_scopes(local_var_scopes, call_location, ident) {
        return ty;
    }
    if let Some(ty) = resolve_field_in_class_chain(enclosing_type_api, ident, registry) {
        return ty;
    }
    arg.clone()
}

/// `true` when `raw` is a single Apex identifier (no dots, no
/// whitespace, leading char is a letter or underscore). This is the
/// minimum predicate that keeps dotted/complex expressions safely on
/// the wildcard path — they only land here when upstream inference
/// didn't understand them, so assuming "I can look this up as a
/// simple field/local name" would be wrong.
fn as_bare_identifier(raw: &str) -> Option<&str> {
    let s = raw.trim();
    if s.is_empty() || s.contains('.') {
        return None;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return None,
    }
    if chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some(s)
    } else {
        None
    }
}

/// Mirrors `field_type_resolver::lookup_in_local_scopes`. Kept as a
/// private copy rather than a shared helper because the sibling
/// resolver needs it on a separate code path (receiver resolution)
/// and the two callsites carry subtly different contracts — the
/// receiver side returns `Option<ApexTypeRef>` for a single
/// receiver; this side returns a narrowed `Vec<ApexTypeRef>` for a
/// whole arg list. Pulling both into a shared helper would leak the
/// "declared-at-or-before" invariant into the shared module, which
/// is the sort of hard-to-test coupling we've been burned by before.
fn lookup_in_local_scopes(
    scopes: &[LocalVarScope],
    call_location: &Range,
    name: &str,
) -> Option<ApexTypeRef> {
    let mut containing: Vec<&LocalVarScope> = scopes
        .iter()
        .filter(|s| range_contains(&s.body, call_location))
        .collect();
    containing.sort_by_key(|s| range_span(&s.body));

    // Innermost enclosing scope wins. The extractor currently emits
    // one scope per method / constructor body (parameters + all
    // locals declared anywhere inside the body); nested block scopes
    // are not yet modelled. When they are, this must switch to a
    // walk-outward lookup — the `containing` vec is already sorted
    // innermost-first for exactly that upgrade.
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

/// Mirror of `field_type_resolver::resolve_field_in_class_chain`. Kept
/// as a private copy for the same reason the local-scope helper is
/// — see that helper's doc comment.
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
        current = parent;
    }
    None
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

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::{LocalVarDecl, LocalVarScope};
    use crate::domain::apex::class_symbols::{Access, ApexClassSymbols, ApexField};
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

    fn unresolved(raw: &str) -> ApexTypeRef {
        ApexTypeRef::Unresolved {
            raw: raw.to_string(),
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

    #[test]
    fn is_bare_identifier_accepts_simple_names() {
        assert_eq!(as_bare_identifier("s1"), Some("s1"));
        assert_eq!(as_bare_identifier("_foo"), Some("_foo"));
        assert_eq!(as_bare_identifier("myVar_2"), Some("myVar_2"));
    }

    #[test]
    fn is_bare_identifier_rejects_complex_or_empty() {
        assert!(as_bare_identifier("").is_none());
        assert!(as_bare_identifier("a.b").is_none());
        assert!(as_bare_identifier("foo()").is_none());
        assert!(as_bare_identifier("123").is_none());
        assert!(as_bare_identifier("this").is_some()); // keyword but still a valid ident shape — caller decides
    }

    #[test]
    fn narrow_keeps_already_resolved_args_untouched() {
        let reg = ApexClassRegistry::new();
        let loc = Range::with_file(5, 0, 5, 10, "A.cls".to_string());
        let narrowed = narrow_arg_types(
            &[prim("String"), ud("Foo"), unresolved("1 + 2")],
            &loc,
            "Client",
            &reg,
            &[],
        );
        assert_eq!(narrowed[0], prim("String"));
        assert_eq!(narrowed[1], ud("Foo"));
        // `1 + 2` is not a bare identifier → left as wildcard.
        assert_eq!(narrowed[2], unresolved("1 + 2"));
    }

    #[test]
    fn narrow_picks_local_before_field() {
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
                fields: vec![field("svc", ud("FieldSvc"))],
                ..Default::default()
            },
        );
        let body = Range::with_file(3, 0, 10, 0, "Client.cls".to_string());
        let local_decl = Range::with_file(4, 0, 4, 10, "Client.cls".to_string());
        let call = Range::with_file(5, 0, 5, 20, "Client.cls".to_string());
        let scope = LocalVarScope {
            body,
            locals: vec![LocalVarDecl {
                name: "svc".to_string(),
                ty: ud("LocalSvc"),
                declared_at: local_decl,
            }],
        };
        let narrowed = narrow_arg_types(&[unresolved("svc")], &call, "Client", &reg, &[scope]);
        assert_eq!(narrowed[0], ud("LocalSvc"), "local shadows field");
    }

    #[test]
    fn narrow_falls_back_to_field_when_no_local_matches() {
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
                fields: vec![field("svc", ud("FieldSvc"))],
                ..Default::default()
            },
        );
        let call = Range::with_file(5, 0, 5, 20, "Client.cls".to_string());
        let narrowed = narrow_arg_types(&[unresolved("svc")], &call, "Client", &reg, &[]);
        assert_eq!(narrowed[0], ud("FieldSvc"));
    }

    #[test]
    fn narrow_walks_parent_chain_for_field_lookup() {
        let mut reg = ApexClassRegistry::new();
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
                fields: vec![field("svc", ud("BaseSvc"))],
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
        let call = Range::with_file(5, 0, 5, 20, "Client.cls".to_string());
        let narrowed = narrow_arg_types(&[unresolved("svc")], &call, "Client", &reg, &[]);
        assert_eq!(narrowed[0], ud("BaseSvc"));
    }

    #[test]
    fn narrow_leaves_unresolved_when_no_scope_match() {
        let reg = ApexClassRegistry::new();
        let call = Range::with_file(5, 0, 5, 10, "Client.cls".to_string());
        let narrowed = narrow_arg_types(&[unresolved("mystery")], &call, "Client", &reg, &[]);
        assert_eq!(narrowed[0], unresolved("mystery"));
    }
}
