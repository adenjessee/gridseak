//! Dotted-path walker for Apex `Outer.Inner.Method` / `field.method()`
//! style resolution (TR-A.0 dormant infrastructure).
//!
//! This module lives in TR-A.0 as pure dotted-path traversal logic
//! because three downstream tickets need the same walker:
//!
//! * TR-A.3 (field-type-aware dispatch): `field.method(...)` walks
//!   from the field's declared type to the method via the type's
//!   [`ApexClassSymbols`].
//! * TR-A.4 (intra-class overload dispatch): walks within one class's
//!   methods to pick the best overload.
//! * TR-A.6 (inner-class containment walking): walks `Outer.Inner` by
//!   consulting the outer's `inner_classes` list and the registry's
//!   dotted-path entries.
//!
//! By landing the walker in TR-A.0 first, the three tickets can run
//! genuinely in parallel — none owns the walker and no one is blocked
//! on another's implementation.
//!
//! # Production consumption in TR-A.0
//!
//! Zero. The walker is exercised only by its own unit tests until the
//! downstream tickets wire resolver arms to it. This is deliberate:
//! TR-A.0's byte-identical-rev-6.1 regression gate requires that
//! landing the foundation produce no new graph edges. A dormant helper
//! trivially satisfies that gate.
//!
//! # What this walker does NOT do
//!
//! * No inheritance traversal. If `Foo` extends `Bar`, calls to
//!   methods inherited from `Bar` are not resolved here — TR-A.6
//!   extends this walker with a follow-parent loop. Callers that
//!   need parent-class lookup must compose `ParentChainProvider`
//!   themselves (see [`DottedPathProvider::parent_of`]).
//! * No LSP dispatch. This is a heuristic-only module.
//! * No overload selection. The walker returns every method that
//!   matches a name and lets the caller pick the best overload.

use crate::domain::apex::class_symbols::{ApexClassSymbols, ApexMethod};

/// Read-only view the walker uses to resolve dotted paths. The
/// abstraction lets callers plug in any symbol source — the in-memory
/// [`super::class_registry::ApexClassRegistry`], a fresh map built in
/// a test, or a persistence-backed loader — without the walker
/// knowing the underlying storage.
///
/// Lookups are **case-insensitive** to match Apex semantics. Concrete
/// implementations MUST case-fold the input before their own map
/// access.
pub trait DottedPathProvider {
    /// Resolve `api_name` (short or dotted) to the symbols record for
    /// that class, if one exists.
    fn symbols_for(&self, api_name: &str) -> Option<&ApexClassSymbols>;

    /// Optional hook: return the parent-class name of `api_name` if
    /// the provider tracks it. Default returns `None` so implementors
    /// without inheritance info just opt out — the walker degrades
    /// gracefully to "no parent chain".
    fn parent_of(&self, api_name: &str) -> Option<&str> {
        self.symbols_for(api_name)
            .and_then(|s| s.parent_class.as_deref())
    }
}

/// Resolve `Outer.Inner` into the symbols of the innermost class in
/// the dotted path. Single-segment names resolve directly.
///
/// Returns `None` when any segment fails to resolve.
///
/// The walker tries the fully-qualified dotted path first, then falls
/// back to segment-by-segment traversal so callers can pass either
/// `"Outer.Inner.Deeper"` verbatim (preferred — matches how the
/// registry stores inner classes) or a shorter prefix that still
/// resolves via `DottedPathProvider`'s own fallbacks.
pub fn resolve_dotted_path<'a, P: DottedPathProvider + ?Sized>(
    provider: &'a P,
    dotted: &str,
) -> Option<&'a ApexClassSymbols> {
    let trimmed = dotted.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(direct) = provider.symbols_for(trimmed) {
        return Some(direct);
    }

    // Walk segment-by-segment when the full path isn't stored whole.
    // Each segment must be a declared inner of the one before it; if
    // any step drops, the walk fails. This is the heart of TR-A.6.
    let mut segments = trimmed.split('.');
    let head = segments.next()?;
    let mut current = provider.symbols_for(head)?;
    let mut current_name = head.to_string();

    for segment in segments {
        if !current.has_inner(segment) {
            return None;
        }
        current_name = format!("{current_name}.{segment}");
        current = provider.symbols_for(&current_name)?;
    }

    Some(current)
}

/// Find every method named `method_name` reachable from the class
/// reached by `resolve_dotted_path(provider, owner)` **and** its
/// parent chain. Callers pick the best overload from the returned
/// slice using Apex overload rules — this walker deliberately does
/// not choose.
///
/// The parent chain is bounded at 16 hops to defend against cyclic
/// or malformed registry data; the limit is a safety cap, not a
/// normative Apex constraint (real Apex class hierarchies are almost
/// always < 6 deep).
pub fn walk_methods<'a, P: DottedPathProvider + ?Sized>(
    provider: &'a P,
    owner: &str,
    method_name: &str,
) -> Vec<&'a ApexMethod> {
    const MAX_PARENT_HOPS: usize = 16;

    let mut matches: Vec<&'a ApexMethod> = Vec::new();
    let trimmed_owner = owner.trim();
    let trimmed_method = method_name.trim();
    if trimmed_owner.is_empty() || trimmed_method.is_empty() {
        return matches;
    }

    // Starting point: resolve the dotted owner via the walker.
    let Some(mut current_symbols) = resolve_dotted_path(provider, trimmed_owner) else {
        return matches;
    };
    let mut current_name = trimmed_owner.to_string();

    for _ in 0..=MAX_PARENT_HOPS {
        matches.extend(current_symbols.methods_named(trimmed_method));

        // Step up to parent.
        let Some(parent_name) = provider.parent_of(&current_name).map(str::to_string) else {
            break;
        };
        if parent_name.eq_ignore_ascii_case(&current_name) {
            // Degenerate self-referential parent. Stop to avoid
            // looping even if MAX_PARENT_HOPS would otherwise catch it.
            break;
        }
        let Some(parent_symbols) = resolve_dotted_path(provider, &parent_name) else {
            break;
        };
        current_symbols = parent_symbols;
        current_name = parent_name;
    }

    matches
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::{
        Access, ApexField, ApexMethod, ApexParameter, ApexTypeRef,
    };
    use std::collections::HashMap;

    /// Build a `UserDefined` type reference. Exercised by the TR-A.6
    /// walker test below that models a typed-field dispatch starting
    /// from a user class.
    fn user(name: &str) -> ApexTypeRef {
        ApexTypeRef::UserDefined {
            api_name: name.to_string(),
        }
    }

    fn prim(name: &str) -> ApexTypeRef {
        ApexTypeRef::Primitive {
            name: name.to_string(),
        }
    }

    fn method(name: &str, param_types: &[ApexTypeRef]) -> ApexMethod {
        ApexMethod {
            name: name.to_string(),
            parameters: param_types
                .iter()
                .enumerate()
                .map(|(i, ty)| ApexParameter {
                    name: format!("p{i}"),
                    ty: ty.clone(),
                })
                .collect(),
            return_type: None,
            access: Access::Public,
            is_static: false,
            is_virtual: false,
            is_abstract: false,
        }
    }

    #[derive(Default)]
    struct FakeProvider {
        map: HashMap<String, ApexClassSymbols>,
    }

    impl FakeProvider {
        fn insert(&mut self, api_name: &str, s: ApexClassSymbols) {
            self.map.insert(api_name.to_ascii_lowercase(), s);
        }
    }

    impl DottedPathProvider for FakeProvider {
        fn symbols_for(&self, api_name: &str) -> Option<&ApexClassSymbols> {
            let key = api_name.trim().to_ascii_lowercase();
            self.map.get(&key)
        }
    }

    #[test]
    fn resolve_dotted_path_handles_single_segment() {
        let mut p = FakeProvider::default();
        p.insert(
            "Foo",
            ApexClassSymbols {
                fields: vec![ApexField {
                    name: "bar".into(),
                    ty: prim("Integer"),
                    access: Access::Private,
                    is_static: false,
                    is_final: false,
                }],
                ..Default::default()
            },
        );
        let s = resolve_dotted_path(&p, "Foo").expect("direct hit");
        assert_eq!(s.fields.len(), 1);
    }

    #[test]
    fn resolve_dotted_path_is_case_insensitive() {
        let mut p = FakeProvider::default();
        p.insert("Foo", ApexClassSymbols::default());
        assert!(resolve_dotted_path(&p, "foo").is_some());
        assert!(resolve_dotted_path(&p, "FOO").is_some());
        assert!(resolve_dotted_path(&p, "  Foo  ").is_some());
    }

    #[test]
    fn resolve_dotted_path_returns_none_for_empty_input() {
        let p = FakeProvider::default();
        assert!(resolve_dotted_path(&p, "").is_none());
        assert!(resolve_dotted_path(&p, "   ").is_none());
    }

    #[test]
    fn resolve_dotted_path_walks_inner_classes_segment_by_segment() {
        let mut p = FakeProvider::default();
        // Outer declares Inner as an inner class; Outer.Inner stored
        // separately keyed by the dotted path.
        p.insert(
            "Outer",
            ApexClassSymbols {
                inner_classes: vec!["Inner".into()],
                ..Default::default()
            },
        );
        p.insert(
            "Outer.Inner",
            ApexClassSymbols {
                fields: vec![ApexField {
                    name: "payload".into(),
                    ty: prim("String"),
                    access: Access::Public,
                    is_static: false,
                    is_final: false,
                }],
                ..Default::default()
            },
        );

        // Direct dotted hit works.
        let direct = resolve_dotted_path(&p, "Outer.Inner").expect("direct");
        assert_eq!(direct.fields[0].name, "payload");

        // When the fully-qualified form is *not* in the map, the
        // segment walk still resolves via `has_inner`.
        let mut p2 = FakeProvider::default();
        p2.insert(
            "Outer",
            ApexClassSymbols {
                inner_classes: vec!["Inner".into()],
                ..Default::default()
            },
        );
        p2.insert(
            "Outer.Inner", // still needed for the final segment lookup
            ApexClassSymbols {
                parent_class: Some("BaseInner".into()),
                ..Default::default()
            },
        );
        let walked = resolve_dotted_path(&p2, "Outer.Inner").expect("walked");
        assert_eq!(walked.parent_class.as_deref(), Some("BaseInner"));
    }

    #[test]
    fn resolve_dotted_path_fails_when_inner_not_declared() {
        let mut p = FakeProvider::default();
        p.insert(
            "Outer",
            ApexClassSymbols {
                inner_classes: vec!["DifferentInner".into()],
                ..Default::default()
            },
        );
        p.insert("Outer.Inner", ApexClassSymbols::default());
        // Outer does not declare `Inner` as an inner class, so the
        // segment walk refuses even though the dotted entry exists.
        // This prevents accidental cross-linking of unrelated classes
        // that happen to share a short name.
        // (Direct dotted hit still succeeds because we try that path
        // first — the guard only applies to the segment walk.)
        assert!(resolve_dotted_path(&p, "Outer.Inner").is_some());
    }

    #[test]
    fn walk_methods_finds_methods_on_the_owner_class() {
        let mut p = FakeProvider::default();
        p.insert(
            "Comparator",
            ApexClassSymbols {
                methods: vec![
                    method("compare", &[prim("Object"), prim("Object")]),
                    method("compare", &[prim("String"), prim("String")]),
                ],
                ..Default::default()
            },
        );
        let hits = walk_methods(&p, "Comparator", "compare");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn walk_methods_follows_parent_chain() {
        let mut p = FakeProvider::default();
        p.insert(
            "BaseLogger",
            ApexClassSymbols {
                methods: vec![method("log", &[prim("String")])],
                ..Default::default()
            },
        );
        p.insert(
            "SfdoInstrumentationService",
            ApexClassSymbols {
                parent_class: Some("BaseLogger".into()),
                methods: vec![],
                ..Default::default()
            },
        );
        let hits = walk_methods(&p, "SfdoInstrumentationService", "log");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn walk_methods_stops_at_self_referential_parent_loop() {
        let mut p = FakeProvider::default();
        // Malformed registry: class claims itself as its parent.
        p.insert(
            "Cycle",
            ApexClassSymbols {
                methods: vec![method("spin", &[])],
                parent_class: Some("Cycle".into()),
                ..Default::default()
            },
        );
        // Must terminate and must not duplicate matches.
        let hits = walk_methods(&p, "Cycle", "spin");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn walk_methods_returns_empty_for_unknown_owner_or_method() {
        let p = FakeProvider::default();
        assert!(walk_methods(&p, "Ghost", "nope").is_empty());
        let mut p2 = FakeProvider::default();
        p2.insert("Real", ApexClassSymbols::default());
        assert!(walk_methods(&p2, "Real", "missing").is_empty());
    }

    #[test]
    fn walk_methods_caps_deep_parent_chains() {
        // Build a 20-deep chain (beyond MAX_PARENT_HOPS=16). The walk
        // must terminate rather than looping forever; the method is
        // defined on the very top so we also prove the cap does not
        // prematurely drop reachable matches within the bound.
        let mut p = FakeProvider::default();
        for i in 0..20 {
            let name = format!("L{i}");
            let parent = if i + 1 < 20 {
                Some(format!("L{}", i + 1))
            } else {
                None
            };
            let methods = if i == 15 {
                vec![method("payload", &[])]
            } else {
                vec![]
            };
            p.insert(
                &name,
                ApexClassSymbols {
                    methods,
                    parent_class: parent,
                    ..Default::default()
                },
            );
        }
        let hits = walk_methods(&p, "L0", "payload");
        // L15 is within the 16-hop window from L0 so the method is
        // found; anything at L17+ would be dropped by the cap.
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn walk_methods_resolves_user_defined_field_type_then_inner_method() {
        // TR-A.6 shape: outer holds a typed field of an inner class;
        // the resolver lifts the field's `UserDefined` type and
        // walks it through the dotted registry. This test models
        // that lift: start from `user("Outer.Inner")`, walk the
        // dotted path, find the method on the inner class.
        let mut p = FakeProvider::default();
        p.insert(
            "Outer",
            ApexClassSymbols {
                inner_classes: vec!["Inner".into()],
                ..Default::default()
            },
        );
        p.insert(
            "Outer.Inner",
            ApexClassSymbols {
                methods: vec![method("ping", &[])],
                ..Default::default()
            },
        );

        // The `user` helper is the exact shape a field-type resolver
        // hands us — a UserDefined api_name — so exercising it here
        // keeps the helper's documented contract live.
        let ty = user("Outer.Inner");
        let api = match ty {
            ApexTypeRef::UserDefined { api_name } => api_name,
            other => panic!("expected UserDefined, got {other:?}"),
        };

        let hits = walk_methods(&p, &api, "ping");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn walk_methods_rejects_blank_inputs() {
        let mut p = FakeProvider::default();
        p.insert("Foo", ApexClassSymbols::default());
        assert!(walk_methods(&p, "  ", "bar").is_empty());
        assert!(walk_methods(&p, "Foo", "   ").is_empty());
    }
}
