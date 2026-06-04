//! Inner-class dispatch resolution (TR-A.6).
//!
//! Apex uses short, unqualified names to refer to inner classes from
//! inside the enclosing outer class (and from inside sibling inner
//! classes of the same outer). The extractor records the declared
//! type verbatim — e.g. `private Inner inner;` stores its type as
//! `UserDefined { api_name: "Inner" }`. The
//! [`super::class_registry::ApexClassRegistry`], however, keys inner
//! classes under their dotted path (`Outer.Inner`) since that is the
//! globally unambiguous form the class-symbols extractor emits. A
//! plain `symbols_for("Inner")` therefore misses the registry entry
//! even when the class is known.
//!
//! TR-A.6 closes the gap with a small normalisation helper that
//! consults the enclosing class's `inner_classes` list and, when the
//! bare name matches, rewrites the api-name to its dotted form. This
//! mirrors the constructor resolver's
//! [`super::constructor_resolver`] `resolve_sibling_inner` and lives
//! in its own module so the method-dispatch path (TR-A.3 / TR-A.4)
//! does not have to reach across to the ctor module for it.
//!
//! # What this module does not do
//!
//! * **No inheritance traversal.** Callers that need to walk a
//!   `parent_class` chain call the [`super::containment_walker`]
//!   directly — this module answers "what is the registry key for
//!   this short name given the caller's enclosing class?" and that
//!   is all.
//! * **No constructor rewriting.** `new Foo(...)` dispatch lives in
//!   [`super::constructor_resolver`] and carries its own equivalent
//!   of this helper (the two stay decoupled by design — method
//!   dispatch and ctor dispatch have subtly different fallback
//!   semantics that we do not want to force into a single
//!   over-general helper).
//! * **No short-circuit for top-level dotted inputs.** Callers pass
//!   the short name they lifted from a field / local / param
//!   declaration; dotted paths bypass this module.

use super::class_registry::ApexClassRegistry;
use super::containment_walker::resolve_dotted_path;

/// Resolve a potentially-inner-class short name to the registry key
/// the enclosing outer would consult.
///
/// Returns `Some(dotted_api)` when:
///
/// 1. `short` is an unqualified name (no `.`).
/// 2. `enclosing_class_api` is known to the registry.
/// 3. The enclosing class (or any class on its outer chain) lists
///    `short` in its `inner_classes`.
///
/// Returns `None` otherwise — in which case the caller keeps the
/// short form and falls back to the global registry lookup, which
/// handles top-level classes, standard SObjects, and anything else
/// keyed directly.
///
/// The case-sensitivity rule mirrors the ctor-side helper: inner
/// class membership is compared case-insensitively because Apex
/// identifiers are case-insensitive, but the returned dotted form
/// preserves the enclosing class's declared casing (registry keys
/// are case-insensitive on lookup but the string we return goes
/// straight into registry access so either case works).
///
/// # Outer-chain walk
///
/// When the enclosing class is itself an inner (`Outer.Mid`), the
/// helper also walks the outer chain to let callers from a nested
/// inner reference siblings declared at any containing level. NPSP
/// does not currently ship a three-deep nesting on the §4.11.1
/// revert-population list, but the walk costs nothing extra and
/// closes the general-case correctness hole pre-emptively.
pub(super) fn normalise_short_class_name(
    short: &str,
    enclosing_class_api: &str,
    registry: &ApexClassRegistry,
) -> Option<String> {
    let short = short.trim();
    if short.is_empty() || short.contains('.') {
        return None;
    }

    // Walk the enclosing chain from innermost outward so a nested
    // inner class can reference siblings declared on any containing
    // class.
    let mut current = enclosing_class_api.to_string();
    for _ in 0..MAX_OUTER_HOPS {
        if let Some(syms) = registry.symbols_for(&current) {
            if syms
                .inner_classes
                .iter()
                .any(|n| n.eq_ignore_ascii_case(short))
            {
                return Some(format!("{current}.{short}"));
            }
        }
        // Step up one level of containment. `Outer.Mid.Inner` → `Outer.Mid`
        // → `Outer`; once we reach a dotted prefix that no longer has
        // a `.` we are at the top-level — the caller already tried the
        // short form against the global registry so we stop here.
        let Some((parent_path, _)) = current.rsplit_once('.') else {
            break;
        };
        current = parent_path.to_string();
    }

    None
}

/// Resolve a potentially-inner-class short name first via the
/// enclosing-class walk, then via direct registry lookup. Callers
/// use this when they want a single canonical api-name to consult
/// against the registry and don't care which resolution path hit.
///
/// Returns `None` when neither the inner-class normalisation nor
/// the direct registry lookup resolves the name.
pub(super) fn canonicalise_api_name(
    short_or_dotted: &str,
    enclosing_class_api: &str,
    registry: &ApexClassRegistry,
) -> Option<String> {
    let trimmed = short_or_dotted.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('.') {
        // Dotted input: trust the caller, verify the path resolves.
        return resolve_dotted_path(registry, trimmed).map(|_| trimmed.to_string());
    }
    // Short input: try sibling-inner normalisation first so the
    // dotted form wins when both are present in the registry.
    if let Some(dotted) = normalise_short_class_name(trimmed, enclosing_class_api, registry) {
        return Some(dotted);
    }
    // Fallback: the short form may itself be a top-level class.
    registry.symbols_for(trimmed).map(|_| trimmed.to_string())
}

/// Ceiling on how many outer-containment hops the walk follows. Three
/// hops is already well past NPSP's maximum nesting depth; the cap
/// mainly defends against malformed registry entries with cyclic
/// dotted parents.
const MAX_OUTER_HOPS: usize = 16;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::ApexClassSymbols;
    use crate::syntax::language::apex::class_registry::ApexTypeKind;
    use std::path::PathBuf;

    fn registry_with(entries: &[(&str, ApexClassSymbols)]) -> ApexClassRegistry {
        let mut r = ApexClassRegistry::with_standard_preload();
        for (api, _) in entries {
            let enclosing = api.rsplit_once('.').map(|(outer, _)| outer.to_string());
            r.insert_user_declared(
                api,
                ApexTypeKind::Class,
                PathBuf::from(format!("{api}.cls")),
                enclosing,
            );
        }
        for (api, syms) in entries {
            assert!(
                r.attach_symbols(api, syms.clone()),
                "attach_symbols failed for {api}",
            );
        }
        r
    }

    #[test]
    fn short_name_that_is_a_sibling_inner_normalises_to_dotted() {
        let outer = ApexClassSymbols {
            inner_classes: vec!["ChildLoader".into()],
            ..Default::default()
        };
        let child = ApexClassSymbols::default();
        let r = registry_with(&[
            ("OverrideOuter", outer),
            ("OverrideOuter.ChildLoader", child),
        ]);

        let got = normalise_short_class_name("ChildLoader", "OverrideOuter", &r);
        assert_eq!(got.as_deref(), Some("OverrideOuter.ChildLoader"));
    }

    #[test]
    fn short_name_is_case_insensitive_on_inner_match() {
        let outer = ApexClassSymbols {
            inner_classes: vec!["ChildLoader".into()],
            ..Default::default()
        };
        let child = ApexClassSymbols::default();
        let r = registry_with(&[
            ("OverrideOuter", outer),
            ("OverrideOuter.ChildLoader", child),
        ]);

        let got = normalise_short_class_name("childloader", "overrideouter", &r);
        assert_eq!(got.as_deref(), Some("overrideouter.childloader"));
    }

    #[test]
    fn short_name_not_an_inner_returns_none() {
        let outer = ApexClassSymbols {
            inner_classes: vec!["Other".into()],
            ..Default::default()
        };
        let r = registry_with(&[("OverrideOuter", outer)]);

        assert!(normalise_short_class_name("Missing", "OverrideOuter", &r).is_none());
    }

    #[test]
    fn dotted_input_short_circuits_to_none() {
        let r = registry_with(&[]);
        assert!(normalise_short_class_name("Outer.Inner", "Irrelevant", &r).is_none());
    }

    #[test]
    fn outer_chain_walk_finds_sibling_two_levels_up() {
        let outer = ApexClassSymbols {
            inner_classes: vec!["Mid".into(), "Sibling".into()],
            ..Default::default()
        };
        let mid = ApexClassSymbols {
            inner_classes: vec!["Inner".into()],
            ..Default::default()
        };
        let r = registry_with(&[
            ("Top", outer),
            ("Top.Mid", mid),
            ("Top.Mid.Inner", ApexClassSymbols::default()),
            ("Top.Sibling", ApexClassSymbols::default()),
        ]);

        // Caller inside Top.Mid.Inner references "Sibling" (declared
        // on Top, not on Top.Mid). Expected: walk up and find it.
        let got = normalise_short_class_name("Sibling", "Top.Mid.Inner", &r);
        assert_eq!(got.as_deref(), Some("Top.Sibling"));
    }

    #[test]
    fn canonicalise_prefers_dotted_inner_over_top_level_short_match() {
        // Registry carries BOTH a top-level `Inner` AND a sibling
        // inner `Outer.Inner`. When the caller's enclosing class is
        // `Outer`, the sibling-inner form wins — that is the Apex
        // name-resolution rule (inner-class declarations shadow
        // same-named top-level classes from within the outer).
        let outer = ApexClassSymbols {
            inner_classes: vec!["Inner".into()],
            ..Default::default()
        };
        let r = registry_with(&[
            ("Inner", ApexClassSymbols::default()),
            ("Outer", outer),
            ("Outer.Inner", ApexClassSymbols::default()),
        ]);

        let got = canonicalise_api_name("Inner", "Outer", &r);
        assert_eq!(got.as_deref(), Some("Outer.Inner"));
    }

    #[test]
    fn canonicalise_falls_back_to_top_level_when_no_sibling_match() {
        let outer = ApexClassSymbols {
            inner_classes: vec![],
            ..Default::default()
        };
        let r = registry_with(&[("TopLevel", ApexClassSymbols::default()), ("Outer", outer)]);

        let got = canonicalise_api_name("TopLevel", "Outer", &r);
        assert_eq!(got.as_deref(), Some("TopLevel"));
    }

    #[test]
    fn canonicalise_returns_none_for_unknown_short_name() {
        let r = registry_with(&[]);
        assert!(canonicalise_api_name("Ghost", "Outer", &r).is_none());
    }

    #[test]
    fn canonicalise_accepts_dotted_input_when_it_resolves() {
        let outer = ApexClassSymbols {
            inner_classes: vec!["Inner".into()],
            ..Default::default()
        };
        let r = registry_with(&[
            ("Outer", outer),
            ("Outer.Inner", ApexClassSymbols::default()),
        ]);
        let got = canonicalise_api_name("Outer.Inner", "Outer", &r);
        assert_eq!(got.as_deref(), Some("Outer.Inner"));
    }

    #[test]
    fn canonicalise_rejects_dotted_input_when_it_fails_to_resolve() {
        let r = registry_with(&[]);
        assert!(canonicalise_api_name("Ghost.Missing", "Anywhere", &r).is_none());
    }
}
