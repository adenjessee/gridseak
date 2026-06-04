//! Apex type-oracle symbol shapes (TR-A.0).
//!
//! Earlier Apex resolution in this engine operated over lightweight
//! [`ApexTypeEntry`](super::class_registry::ApexTypeEntry) records that
//! captured *existence* of a class but not its *shape*. That worked for
//! the initial FQN / managed-namespace bookkeeping the resolver needed
//! pre-rev-6.1, but the Phase-A AST-resolver gaps (R23) all require the
//! same missing ingredient: a declarative map of each user class's
//! fields, methods, constructors, inner classes, parent class, and
//! implemented interfaces. `ApexClassSymbols` is that map.
//!
//! # Scope
//!
//! This module defines **pure data shapes** only. It contains no
//! extraction logic (see `symbols_extractor.rs`), no persistence (see
//! `symbols_repository.rs`), and no resolver consumption — the TR-A.0
//! contract is "foundation lands dormant; resolvers in TR-A.1 … TR-A.6
//! consume it without changing their own shape". That split keeps the
//! byte-identical-rev-6.1 regression gate meaningful: populating these
//! structs must not, by itself, change a single emitted node or edge.
//!
//! # Triggers are out of scope
//!
//! Apex `.trigger` files carry implicit context variables (`Trigger.new`,
//! `Trigger.old`) whose types are determined by the trigger's SObject
//! target, plus a `when` event list. That shape doesn't fit
//! `ApexClassSymbols` without becoming a leaky catch-all. A dedicated
//! `TriggerSymbols { sobject_type, events, trigger_body_fqn }` struct
//! lives in Phase B. This module intentionally does nothing for
//! triggers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Version marker — persisted alongside symbols in the parse DB.
//
// Bumped whenever the shapes below change in a way that would make old
// persisted rows incorrect for a new resolver consumer. The parse-DB
// migration compares this to the stored `parse_meta.schema_version` to
// decide whether to emit `CAVEAT_STALE_PARSE_DB_V1`.
// ---------------------------------------------------------------------------

/// Schema version of the persisted `ApexClassSymbols` payload. See the
/// crate-level `SCHEMA_VERSION_KEY` / `SCHEMA_VERSION_VALUE` constants
/// wired in the SQLite persistence layer.
pub const APEX_CLASS_SYMBOLS_SCHEMA_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// ApexTypeRef
// ---------------------------------------------------------------------------

/// How a declared type (field type, method return type, parameter
/// type, etc.) is classified by the extractor. Deliberately narrow:
/// enough to drive resolver dispatch without becoming a full type
/// system.
///
/// `Unresolved` is the fallback when the name is not a known primitive,
/// SObject, or user class. Unresolved references are still recorded so
/// the resolver can follow them — downstream code decides whether to
/// treat them as "external" or "keep looking".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "variant", rename_all = "snake_case")]
pub enum ApexTypeRef {
    /// Apex primitive: `Boolean`, `Integer`, `Long`, `Double`,
    /// `Decimal`, `Date`, `Datetime`, `Time`, `String`, `Id`, `Blob`,
    /// `Object`. Case-insensitive; the canonical api-case name is
    /// stored.
    Primitive { name: String },
    /// Standard or custom SObject (`Account`, `Contact`, `Foo__c`).
    /// The resolver never dots into these for method dispatch; they
    /// participate only as field types, query references, and
    /// argument bindings.
    Sobject { api_name: String },
    /// User-declared Apex class/interface/enum (potentially inner:
    /// `Outer.Inner` is the dotted path). Stored exactly as declared
    /// in source; case-insensitivity is resolved at lookup time, not
    /// at storage time.
    UserDefined { api_name: String },
    /// `List<T>`, `Set<T>`.
    Collection {
        kind: CollectionKind,
        element: Box<ApexTypeRef>,
    },
    /// `Map<K, V>`. Split out from [`ApexTypeRef::Collection`] because
    /// the two type parameters need different handling in the resolver.
    Map {
        key: Box<ApexTypeRef>,
        value: Box<ApexTypeRef>,
    },
    /// Any other generic not reducible to a collection or map. Kept
    /// non-destructive so the resolver can still compare base names
    /// (`Iterable<T>`, `Iterator<T>`, managed-package `Foo<T>`).
    Generic {
        base: String,
        parameters: Vec<ApexTypeRef>,
    },
    /// Type whose shape was not reducible to the variants above. The
    /// raw source-text form is preserved so downstream resolver arms
    /// (LSP, class registry) can still attempt lookup.
    Unresolved { raw: String },
}

impl ApexTypeRef {
    /// Best-effort api-name rendering. Primarily used for display
    /// strings and for looking the type up in
    /// [`super::class_registry::ApexClassRegistry`]. Callers MUST NOT
    /// rely on this string being parseable back into an
    /// `ApexTypeRef` — use the structured variants for that.
    pub fn to_api_name(&self) -> String {
        match self {
            ApexTypeRef::Primitive { name } | ApexTypeRef::Sobject { api_name: name } => {
                name.clone()
            }
            ApexTypeRef::UserDefined { api_name } => api_name.clone(),
            ApexTypeRef::Collection { kind, element } => {
                format!("{}<{}>", kind.as_str(), element.to_api_name())
            }
            ApexTypeRef::Map { key, value } => {
                format!("Map<{}, {}>", key.to_api_name(), value.to_api_name())
            }
            ApexTypeRef::Generic { base, parameters } => {
                let rendered: Vec<String> = parameters.iter().map(Self::to_api_name).collect();
                format!("{}<{}>", base, rendered.join(", "))
            }
            ApexTypeRef::Unresolved { raw } => raw.clone(),
        }
    }
}

/// Discriminator for `List<T>` vs `Set<T>` inside
/// [`ApexTypeRef::Collection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionKind {
    List,
    Set,
}

impl CollectionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CollectionKind::List => "List",
            CollectionKind::Set => "Set",
        }
    }
}

// ---------------------------------------------------------------------------
// Access / visibility
// ---------------------------------------------------------------------------

/// Apex visibility / access. Apex adds `global` (package-public) and
/// `with sharing` metadata on classes; this enum only captures
/// member-level visibility because that is all the resolver needs for
/// dispatch correctness in Phase A. `with sharing` remains on the
/// class-level side in [`super::sharing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// `private` — default for class members in Apex.
    #[default]
    Private,
    /// `protected` — inheritable, not callable from outside the class
    /// hierarchy.
    Protected,
    /// `public` — callable from any Apex inside the same package.
    Public,
    /// `global` — callable across package boundaries. The only visibility
    /// that matters for managed-package consumer scans.
    Global,
}

// ---------------------------------------------------------------------------
// Field / parameter / method / constructor
// ---------------------------------------------------------------------------

/// A single field declared on an Apex class (top-level or inner).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexField {
    /// Field name exactly as declared. Apex identifiers are
    /// case-insensitive but the canonical declaration case is
    /// preserved for rendering.
    pub name: String,
    /// Declared type.
    pub ty: ApexTypeRef,
    /// Visibility. Affects whether TR-A.3 dispatch consults this field
    /// from outside the declaring class.
    pub access: Access,
    /// `true` for `static` fields.
    pub is_static: bool,
    /// `true` for `final` fields (immutability marker, not relevant
    /// for dispatch but cheap to record).
    pub is_final: bool,
}

/// A formal parameter on a method or constructor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexParameter {
    pub name: String,
    pub ty: ApexTypeRef,
}

/// A single method declared on an Apex class (top-level or inner).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexMethod {
    /// Method name exactly as declared.
    pub name: String,
    /// Formal parameters in declaration order. Overload dispatch
    /// (TR-A.4) needs the full parameter vector — arity alone is not
    /// enough to distinguish `compare(Object,Object)` from
    /// `compare(String,String)`.
    pub parameters: Vec<ApexParameter>,
    /// Return type. `None` for `void`.
    pub return_type: Option<ApexTypeRef>,
    pub access: Access,
    pub is_static: bool,
    /// `true` for `virtual` or `override` methods. Keeping the flag
    /// allows the resolver to reason about dynamic dispatch once
    /// TR-A.6 inner-class / inheritance walking lands.
    pub is_virtual: bool,
    /// `true` for `abstract` methods.
    pub is_abstract: bool,
}

/// A constructor declared on an Apex class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexConstructor {
    pub parameters: Vec<ApexParameter>,
    pub access: Access,
}

// ---------------------------------------------------------------------------
// ApexClassSymbols
// ---------------------------------------------------------------------------

/// Full declarative shape of one user-declared Apex class.
///
/// Inner classes are tracked by **name only** in `inner_classes`;
/// their full shape is stored as a separate `ApexClassSymbols` entry
/// keyed by the dotted `Outer.Inner` api name in the outer
/// registry / repository. This keeps the shapes flat and avoids
/// recursive JSON for persistence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexClassSymbols {
    /// Fields declared on the class, in declaration order.
    #[serde(default)]
    pub fields: Vec<ApexField>,
    /// Methods declared on the class, in declaration order.
    #[serde(default)]
    pub methods: Vec<ApexMethod>,
    /// Constructors, in declaration order. Empty when the class
    /// relies on the implicit default constructor.
    #[serde(default)]
    pub constructors: Vec<ApexConstructor>,
    /// Names of inner classes / interfaces / enums declared inside
    /// this class. **Short names only** — the full `Outer.Inner`
    /// dotted path is the map key at persistence time.
    #[serde(default)]
    pub inner_classes: Vec<String>,
    /// Name of the direct parent class, if any. Stored as declared
    /// (may be unqualified or dotted). Resolution across this pointer
    /// is TR-A.6 territory.
    #[serde(default)]
    pub parent_class: Option<String>,
    /// Interface names this class directly implements. As with
    /// `parent_class`, qualification is preserved exactly as declared.
    #[serde(default)]
    pub implemented_interfaces: Vec<String>,
}

impl ApexClassSymbols {
    /// Look up a field by its declared name (case-insensitive, Apex
    /// semantics). Returns the **first** matching field in
    /// declaration order so callers get deterministic behaviour
    /// regardless of HashMap iteration.
    pub fn find_field(&self, name: &str) -> Option<&ApexField> {
        let target = name.trim();
        self.fields
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(target))
    }

    /// Return every method with the given simple name (case-insensitive).
    /// Overload dispatch (TR-A.4) needs all matches; callers pick the
    /// winner by applying Apex overload rules over the parameter
    /// vectors.
    pub fn methods_named<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a ApexMethod> + 'a {
        let target = name.trim().to_string();
        self.methods
            .iter()
            .filter(move |m| m.name.eq_ignore_ascii_case(&target))
    }

    /// `true` when this class declares an inner class / interface / enum
    /// named `name` (case-insensitive short match).
    pub fn has_inner(&self, name: &str) -> bool {
        let target = name.trim();
        self.inner_classes
            .iter()
            .any(|inner| inner.eq_ignore_ascii_case(target))
    }
}

/// Convenience ordered map over `(api_name, symbols)` pairs. Used by
/// the extractor when it produces a per-file batch and by tests that
/// want deterministic iteration order.
pub type ApexSymbolsMap = BTreeMap<String, ApexClassSymbols>;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn default_symbols_is_empty_in_every_dimension() {
        let s = ApexClassSymbols::default();
        assert!(s.fields.is_empty());
        assert!(s.methods.is_empty());
        assert!(s.constructors.is_empty());
        assert!(s.inner_classes.is_empty());
        assert!(s.parent_class.is_none());
        assert!(s.implemented_interfaces.is_empty());
    }

    #[test]
    fn to_api_name_round_trips_simple_primitives_and_user_types() {
        assert_eq!(prim("Integer").to_api_name(), "Integer");
        assert_eq!(user("MyService").to_api_name(), "MyService");
        assert_eq!(user("Outer.Inner").to_api_name(), "Outer.Inner");
        assert_eq!(
            ApexTypeRef::Sobject {
                api_name: "Account".into()
            }
            .to_api_name(),
            "Account"
        );
    }

    #[test]
    fn to_api_name_renders_collections_and_maps() {
        let list_of_strings = ApexTypeRef::Collection {
            kind: CollectionKind::List,
            element: Box::new(prim("String")),
        };
        assert_eq!(list_of_strings.to_api_name(), "List<String>");

        let map_id_account = ApexTypeRef::Map {
            key: Box::new(prim("Id")),
            value: Box::new(ApexTypeRef::Sobject {
                api_name: "Account".into(),
            }),
        };
        assert_eq!(map_id_account.to_api_name(), "Map<Id, Account>");
    }

    #[test]
    fn to_api_name_handles_nested_generics() {
        // Map<Id, List<Foo>>
        let inner_list = ApexTypeRef::Collection {
            kind: CollectionKind::List,
            element: Box::new(user("Foo")),
        };
        let outer = ApexTypeRef::Map {
            key: Box::new(prim("Id")),
            value: Box::new(inner_list),
        };
        assert_eq!(outer.to_api_name(), "Map<Id, List<Foo>>");
    }

    #[test]
    fn find_field_is_case_insensitive() {
        let symbols = ApexClassSymbols {
            fields: vec![ApexField {
                name: "permissionsService".into(),
                ty: user("UTIL_Permissions"),
                access: Access::Private,
                is_static: false,
                is_final: false,
            }],
            ..Default::default()
        };
        assert!(symbols.find_field("PERMISSIONSSERVICE").is_some());
        assert!(symbols.find_field("permissionsservice").is_some());
        assert!(symbols.find_field("permissionsService").is_some());
        assert!(symbols.find_field("other").is_none());
    }

    #[test]
    fn methods_named_returns_every_overload_in_declaration_order() {
        let symbols = ApexClassSymbols {
            methods: vec![
                ApexMethod {
                    name: "compare".into(),
                    parameters: vec![
                        ApexParameter {
                            name: "a".into(),
                            ty: prim("Object"),
                        },
                        ApexParameter {
                            name: "b".into(),
                            ty: prim("Object"),
                        },
                    ],
                    return_type: Some(prim("Integer")),
                    access: Access::Public,
                    is_static: true,
                    is_virtual: false,
                    is_abstract: false,
                },
                ApexMethod {
                    name: "compare".into(),
                    parameters: vec![
                        ApexParameter {
                            name: "a".into(),
                            ty: prim("String"),
                        },
                        ApexParameter {
                            name: "b".into(),
                            ty: prim("String"),
                        },
                    ],
                    return_type: Some(prim("Integer")),
                    access: Access::Public,
                    is_static: true,
                    is_virtual: false,
                    is_abstract: false,
                },
                ApexMethod {
                    name: "toString".into(),
                    parameters: vec![],
                    return_type: Some(prim("String")),
                    access: Access::Public,
                    is_static: false,
                    is_virtual: false,
                    is_abstract: false,
                },
            ],
            ..Default::default()
        };
        let compare_overloads: Vec<_> = symbols.methods_named("compare").collect();
        assert_eq!(compare_overloads.len(), 2);
        // Declaration order preserved.
        assert_eq!(
            compare_overloads[0].parameters[0].ty.to_api_name(),
            "Object"
        );
        assert_eq!(
            compare_overloads[1].parameters[0].ty.to_api_name(),
            "String"
        );
        // Case-insensitive match.
        assert_eq!(symbols.methods_named("TOSTRING").count(), 1);
        // Miss returns empty.
        assert_eq!(symbols.methods_named("missing").count(), 0);
    }

    #[test]
    fn has_inner_is_case_insensitive_short_name_match() {
        let symbols = ApexClassSymbols {
            inner_classes: vec!["HouseholdMembers".into(), "Builder".into()],
            ..Default::default()
        };
        assert!(symbols.has_inner("householdmembers"));
        assert!(symbols.has_inner("BUILDER"));
        assert!(!symbols.has_inner("Outer"));
    }

    #[test]
    fn serde_round_trips_full_shape() {
        // If serde shape changes unintentionally, persisted rows under
        // the previous SCHEMA_VERSION become unreadable — catch that
        // regression early with an explicit round-trip test.
        let original = ApexClassSymbols {
            fields: vec![ApexField {
                name: "balance".into(),
                ty: prim("Decimal"),
                access: Access::Private,
                is_static: false,
                is_final: true,
            }],
            methods: vec![ApexMethod {
                name: "deposit".into(),
                parameters: vec![ApexParameter {
                    name: "amount".into(),
                    ty: prim("Decimal"),
                }],
                return_type: None,
                access: Access::Public,
                is_static: false,
                is_virtual: false,
                is_abstract: false,
            }],
            constructors: vec![ApexConstructor {
                parameters: vec![ApexParameter {
                    name: "opening".into(),
                    ty: prim("Decimal"),
                }],
                access: Access::Public,
            }],
            inner_classes: vec!["Transfer".into()],
            parent_class: Some("BaseAccount".into()),
            implemented_interfaces: vec!["IAuditable".into()],
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: ApexClassSymbols = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, parsed);
    }

    #[test]
    fn schema_version_is_bumped_from_v1_baseline() {
        // Guard against forgetting to bump the schema version when the
        // shapes above change. Any edit that breaks persisted
        // compatibility MUST raise this constant and add a migration.
        // Compile-time comparison (not `assert!(true)`) so clippy does
        // not elide it.
        const _: () = assert!(APEX_CLASS_SYMBOLS_SCHEMA_VERSION >= 2);
    }
}
