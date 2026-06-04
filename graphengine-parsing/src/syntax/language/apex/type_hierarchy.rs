//! Precomputed subtype indexes for Apex downward dispatch (TR-A.6).
//!
//! The forward registry ([`super::class_registry::ApexClassRegistry`]) answers
//! "given a class FQN, what are its declared members, its `parent_class`,
//! and the interfaces it implements?" — the **upward** view a type takes of
//! its own inheritance chain. Downward dispatch needs the inverse: given an
//! interface, who implements it? Given a parent class, who extends it?
//!
//! Computing that inverse live on every call-site resolution would mean an
//! O(N) iteration over every registered class per query, and the resolver
//! runs this lookup inside its per-call-site hot loop. This module builds
//! both inverses once, at the start of a resolve pass, so queries are
//! O(1) map lookups thereafter.
//!
//! # What we build
//!
//! * `implementers[iface_lowercase] → Vec<api_name>` — direct implementers
//!   of an interface (classes whose `implemented_interfaces` list contains
//!   `iface`, matched case-insensitively).
//! * `subclasses[parent_lowercase] → Vec<api_name>` — direct subclasses
//!   (classes whose `parent_class` equals `parent`, case-insensitive).
//!
//! Values are stored as original-case `api_name` strings so callers can
//! feed them straight into [`ApexClassRegistry::symbols_for`] without
//! re-case-folding.
//!
//! # Transitive rollup
//!
//! Direct relationships only. A class `C` that extends `B` which implements
//! `I` is **not** listed under `implementers["i"]` — only `B` is. This is
//! intentional for PR 5: every NPSP §4.11.1 revert-population FQN lives in
//! a shallow hierarchy (direct implementer, one parent hop max). If deeper
//! transitive fan-out proves necessary later, extend by walking the graph
//! on construction rather than teaching every caller to chase chains.
//!
//! # Non-goals
//!
//! * No method resolution. Callers combine the api_name list returned here
//!   with [`ApexClassSymbols::methods_named`] (or signature matching) to
//!   find the method overrides they need.
//! * No caching invalidation. The index is constructed once per resolve
//!   pass. A second resolve builds a fresh index against the (potentially
//!   different) registry state.

use std::collections::HashMap;

use super::class_registry::ApexClassRegistry;
#[cfg(test)]
use super::class_registry::ApexTypeKind;

/// Upper bound on transitive relationships the hierarchy will track in a
/// single query. Used by [`descendants_of`] and [`implementers_rollup`]
/// to guard against malformed cyclic registries; real Apex class graphs
/// are almost always < 5 deep.
const MAX_DESCENT_DEPTH: usize = 16;

/// Read-only inverse-index over an [`ApexClassRegistry`]. Construct once
/// per resolve pass via [`TypeHierarchy::build`]; query many times via
/// the lookup methods.
#[derive(Debug, Default, Clone)]
pub(super) struct TypeHierarchy {
    /// Keyed on lowercase interface api-name. Values are original-case
    /// class api-names that directly implement the interface.
    implementers: HashMap<String, Vec<String>>,
    /// Keyed on lowercase parent class api-name. Values are original-case
    /// child class api-names that directly extend the parent.
    subclasses: HashMap<String, Vec<String>>,
}

impl TypeHierarchy {
    /// Walk every entry in `registry`, inverting `parent_class` and
    /// `implemented_interfaces` into the two lookup tables. External
    /// entries (standard SObjects, system types, managed-package stubs)
    /// carry no symbols and are skipped — they never participate in
    /// user-defined implements/extends chains.
    ///
    /// The result preserves deterministic iteration order: the value
    /// vectors are sorted alphabetically (case-insensitive) so downstream
    /// fanout edges emit in a stable order across runs.
    pub(super) fn build(registry: &ApexClassRegistry) -> Self {
        let mut implementers: HashMap<String, Vec<String>> = HashMap::new();
        let mut subclasses: HashMap<String, Vec<String>> = HashMap::new();

        for (_key, entry) in registry.iter() {
            if !entry.kind.is_user_defined() {
                continue;
            }
            let Some(syms) = entry.symbols.as_ref() else {
                continue;
            };

            if let Some(parent) = syms.parent_class.as_deref() {
                let parent_trimmed = strip_generic_suffix(parent.trim());
                if !parent_trimmed.is_empty() {
                    subclasses
                        .entry(parent_trimmed.to_ascii_lowercase())
                        .or_default()
                        .push(entry.api_name.clone());
                }
            }
            for iface in &syms.implemented_interfaces {
                let iface_trimmed = strip_generic_suffix(iface.trim());
                if iface_trimmed.is_empty() {
                    continue;
                }
                implementers
                    .entry(iface_trimmed.to_ascii_lowercase())
                    .or_default()
                    .push(entry.api_name.clone());
            }
        }

        // Stable output ordering: alphabetical, case-insensitive.
        for list in implementers.values_mut() {
            list.sort_by_key(|a| a.to_ascii_lowercase());
            list.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        }
        for list in subclasses.values_mut() {
            list.sort_by_key(|a| a.to_ascii_lowercase());
            list.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        }

        Self {
            implementers,
            subclasses,
        }
    }

    /// Classes that directly implement the interface named by
    /// `interface_api`. Returns an empty slice when the interface has
    /// no known implementers (or is not an interface at all — the
    /// helper does not inspect the registry to enforce "must be an
    /// interface", leaving the caller free to decide the policy).
    pub(super) fn implementers_of(&self, interface_api: &str) -> &[String] {
        let key = strip_generic_suffix(interface_api.trim()).to_ascii_lowercase();
        self.implementers
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Classes that directly extend `parent_api`. Empty when `parent_api`
    /// has no known subclasses.
    pub(super) fn subclasses_of(&self, parent_api: &str) -> &[String] {
        let key = strip_generic_suffix(parent_api.trim()).to_ascii_lowercase();
        self.subclasses.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Transitive closure of `parent_api` → every descendant class.
    /// Includes direct subclasses, their subclasses, and so on, capped
    /// at [`MAX_DESCENT_DEPTH`] hops to defend against cyclic registries.
    /// Results are returned in deterministic (sorted, deduplicated)
    /// order.
    pub(super) fn descendants_of(&self, parent_api: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut frontier: Vec<String> = self.subclasses_of(parent_api).to_vec();
        for _ in 0..MAX_DESCENT_DEPTH {
            if frontier.is_empty() {
                break;
            }
            let mut next: Vec<String> = Vec::new();
            for api in &frontier {
                if out.iter().any(|s| s.eq_ignore_ascii_case(api)) {
                    continue;
                }
                out.push(api.clone());
                for child in self.subclasses_of(api) {
                    next.push(child.clone());
                }
            }
            frontier = next;
        }
        out.sort_by_key(|a| a.to_ascii_lowercase());
        out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        out
    }

    /// Direct implementers of `interface_api` plus every descendant of
    /// those implementers. A class `B` extending a direct implementer
    /// `A` inherits `A`'s `implements I` relationship, so dispatch from
    /// a receiver typed as `I` must consider `B`'s overrides as well.
    /// Used by the downward-dispatch resolver for interface fan-out.
    pub(super) fn implementers_rollup(&self, interface_api: &str) -> Vec<String> {
        let mut out: Vec<String> = self.implementers_of(interface_api).to_vec();
        for direct in self.implementers_of(interface_api) {
            for descendant in self.descendants_of(direct) {
                if !out.iter().any(|s| s.eq_ignore_ascii_case(&descendant)) {
                    out.push(descendant);
                }
            }
        }
        out.sort_by_key(|a| a.to_ascii_lowercase());
        out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        out
    }
}

/// Drop `<...>` suffix from an api-name. The registry keys drop generics
/// when parsing the type (see `fqn::canonical_type`), so the hierarchy
/// must match that convention to keep `List<T>` / `Map<K,V>` keys
/// aligned.
fn strip_generic_suffix(raw: &str) -> &str {
    match raw.find('<') {
        Some(idx) => raw[..idx].trim(),
        None => raw,
    }
}

/// Returns `true` when `entry_kind` is a kind that cannot be a class
/// declarant (interfaces and enums can be `implemented`/extended in
/// limited ways, but only `Class` can extend or implement non-trivially
/// in a user-defined chain). Exposed for tests that want to assert
/// the index is not polluted with non-class entries.
#[cfg(test)]
pub(super) fn should_contribute(kind: ApexTypeKind) -> bool {
    matches!(
        kind,
        ApexTypeKind::Class
            | ApexTypeKind::Interface
            | ApexTypeKind::Enum
            | ApexTypeKind::Trigger
            | ApexTypeKind::CustomSObject
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::ApexClassSymbols;
    use crate::syntax::language::apex::class_registry::{ApexClassRegistry, ApexTypeKind};
    use std::path::PathBuf;

    fn registry_with(entries: &[(&str, ApexTypeKind, ApexClassSymbols)]) -> ApexClassRegistry {
        let mut r = ApexClassRegistry::with_standard_preload();
        for (api, kind, _) in entries {
            let enclosing = api.rsplit_once('.').map(|(outer, _)| outer.to_string());
            r.insert_user_declared(api, *kind, PathBuf::from(format!("{api}.cls")), enclosing);
        }
        for (api, _, syms) in entries {
            assert!(
                r.attach_symbols(api, syms.clone()),
                "attach_symbols failed for {api}",
            );
        }
        r
    }

    #[test]
    fn implementers_of_returns_empty_when_no_implementers() {
        let r = registry_with(&[]);
        let h = TypeHierarchy::build(&r);
        assert!(h.implementers_of("Missing").is_empty());
    }

    #[test]
    fn implementers_of_lists_direct_implementers_case_insensitively() {
        let iface = ApexClassSymbols::default();
        let impl_a = ApexClassSymbols {
            implemented_interfaces: vec!["MyInterface".into()],
            ..Default::default()
        };
        let impl_b = ApexClassSymbols {
            implemented_interfaces: vec!["myinterface".into()],
            ..Default::default()
        };
        let r = registry_with(&[
            ("MyInterface", ApexTypeKind::Interface, iface),
            ("ClassA", ApexTypeKind::Class, impl_a),
            ("ClassB", ApexTypeKind::Class, impl_b),
        ]);
        let h = TypeHierarchy::build(&r);

        let impls = h.implementers_of("MyInterface");
        assert_eq!(impls.len(), 2);
        // Deterministic order (case-insensitive alphabetical).
        assert!(impls[0].eq_ignore_ascii_case("ClassA"));
        assert!(impls[1].eq_ignore_ascii_case("ClassB"));

        // Query case does not matter.
        assert_eq!(h.implementers_of("MYINTERFACE").len(), 2);
    }

    #[test]
    fn subclasses_of_lists_direct_children() {
        let parent = ApexClassSymbols::default();
        let child_a = ApexClassSymbols {
            parent_class: Some("Parent".into()),
            ..Default::default()
        };
        let child_b = ApexClassSymbols {
            parent_class: Some("PARENT".into()),
            ..Default::default()
        };
        let r = registry_with(&[
            ("Parent", ApexTypeKind::Class, parent),
            ("ChildA", ApexTypeKind::Class, child_a),
            ("ChildB", ApexTypeKind::Class, child_b),
        ]);
        let h = TypeHierarchy::build(&r);

        let kids = h.subclasses_of("Parent");
        assert_eq!(kids.len(), 2);
        assert!(kids.iter().any(|s| s.eq_ignore_ascii_case("ChildA")));
        assert!(kids.iter().any(|s| s.eq_ignore_ascii_case("ChildB")));
    }

    #[test]
    fn descendants_of_walks_transitively() {
        let mk_child = |parent: &str| ApexClassSymbols {
            parent_class: Some(parent.into()),
            ..Default::default()
        };
        let r = registry_with(&[
            ("Root", ApexTypeKind::Class, ApexClassSymbols::default()),
            ("Mid", ApexTypeKind::Class, mk_child("Root")),
            ("Leaf", ApexTypeKind::Class, mk_child("Mid")),
        ]);
        let h = TypeHierarchy::build(&r);

        let all = h.descendants_of("Root");
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|s| s.eq_ignore_ascii_case("Mid")));
        assert!(all.iter().any(|s| s.eq_ignore_ascii_case("Leaf")));
    }

    #[test]
    fn implementers_rollup_includes_descendants_of_direct_implementers() {
        let iface = ApexClassSymbols::default();
        let direct = ApexClassSymbols {
            implemented_interfaces: vec!["I".into()],
            ..Default::default()
        };
        let child = ApexClassSymbols {
            parent_class: Some("Direct".into()),
            ..Default::default()
        };
        let r = registry_with(&[
            ("I", ApexTypeKind::Interface, iface),
            ("Direct", ApexTypeKind::Class, direct),
            ("Child", ApexTypeKind::Class, child),
        ]);
        let h = TypeHierarchy::build(&r);

        let all = h.implementers_rollup("I");
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|s| s.eq_ignore_ascii_case("Direct")));
        assert!(all.iter().any(|s| s.eq_ignore_ascii_case("Child")));
    }

    #[test]
    fn generic_suffix_on_interface_name_does_not_block_match() {
        // Rare but possible: `implements Comparable<Foo>`. The registry
        // strips generics from member signatures but might keep them in
        // `implemented_interfaces`; normalize both sides.
        let iface = ApexClassSymbols::default();
        let impl_a = ApexClassSymbols {
            implemented_interfaces: vec!["MyCompare<Foo>".into()],
            ..Default::default()
        };
        let r = registry_with(&[
            ("MyCompare", ApexTypeKind::Interface, iface),
            ("ClassA", ApexTypeKind::Class, impl_a),
        ]);
        let h = TypeHierarchy::build(&r);

        let impls = h.implementers_of("MyCompare");
        assert_eq!(impls.len(), 1);
    }

    #[test]
    fn descendants_bounded_on_malformed_cycle() {
        let cyclic = ApexClassSymbols {
            parent_class: Some("Cyclic".into()),
            ..Default::default()
        };
        let r = registry_with(&[("Cyclic", ApexTypeKind::Class, cyclic)]);
        let h = TypeHierarchy::build(&r);

        // Querying descendants of the class itself: it lists itself as
        // its own subclass, so the frontier contains the class. The
        // cycle-guard must prevent infinite growth and produce a
        // stable-bounded result.
        let all = h.descendants_of("Cyclic");
        assert_eq!(all.len(), 1);
        assert!(all[0].eq_ignore_ascii_case("Cyclic"));
    }

    #[test]
    fn should_contribute_excludes_external_entries() {
        assert!(should_contribute(ApexTypeKind::Class));
        assert!(should_contribute(ApexTypeKind::Interface));
        assert!(should_contribute(ApexTypeKind::Enum));
        assert!(!should_contribute(ApexTypeKind::StandardSObject));
        assert!(!should_contribute(ApexTypeKind::SystemType));
        assert!(!should_contribute(ApexTypeKind::ManagedPackageExternal));
    }
}
