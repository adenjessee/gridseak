//! Edge representation for relationships between code elements
//!
//! Adapted from the old core system's UGEdge with validation invariants.
//! Represents relationships between nodes with provenance tracking.
//!
//! # EdgeKind taxonomy — universal-fidelity sprint T1
//!
//! The `EdgeKind` enum carries three families of variants today:
//!
//! 1. **Control-flow / dispatch** — `Call`, `Framework(FrameworkKind)`,
//!    `Declarative(DeclarativeKind)`. These are "something invokes
//!    something else" relationships. They share the `is_call_like()`
//!    predicate; most call-semantic metrics (depth, layers, resolution-
//!    health) filter to this family.
//! 2. **Structural / type** — `Type`, `Extends`, `Implements`, `Import`,
//!    `Uses`. These describe how the program is put together but do not
//!    themselves invoke anything at runtime.
//! 3. **Containment** — `Contains`. Purely structural parenthood; every
//!    structural metric filters this *out* via `is_containment()`.
//!
//! `Framework` and `Declarative` were split off from `Call` in T1 so
//! ecosystem-specific wiring (Visualforce page bindings, Salesforce Flow
//! XML, LWC templates, Spring XML, etc.) stops being coerced into
//! `Call(Low)` where downstream metrics cannot tell them apart. See
//! `docs/workstreams/universal-fidelity/DISCOVERY_REPORT.md` §D5 for the
//! taxonomy decision and §8 for the per-metric inclusion table.
//!
//! # Wire format (post-P1.b)
//!
//! `EdgeKind` is serialised via serde with `#[serde(tag = "kind",
//! content = "sub")]`. Unit variants round-trip as `{"kind":"Call"}`,
//! `{"kind":"Contains"}`, etc.; data-carrying variants round-trip as
//! `{"kind":"Framework","sub":"VisualforcePage"}` and
//! `{"kind":"Declarative","sub":"Flow"}`. The SQLite `edges.kind`
//! column stores that JSON verbatim. Do NOT use `Debug` format for
//! persistence — `Debug` is not stability-guaranteed across compiler
//! versions. The literal wire strings are pinned by
//! `graphengine-parsing/tests/t1_edgekind_roundtrip.rs` so a change to
//! derive order or a future `#[serde(rename)]` trips a test. The wire
//! format is a first-class contract, not an emergent property of the
//! derive.
//!
//! Pre-P1.b, the wire format was a hand-rolled colon-delimited form
//! (`"Framework:VisualforcePage"`). That code path has been deleted in
//! favour of serde. Pre-P1.b parse DBs must be re-parsed; the
//! schema_version bump in SQLite metadata enforces that.

use super::provenance::Provenance;

/// Type of relationship between nodes.
///
/// See the module-level doc for the three-family taxonomy and the
/// serde wire format. `EdgeKind` stays `Copy` (and the in-memory
/// domain type stays closed). Forward-compat with DBs written by a
/// newer engine version is the job of [`PersistedEdgeKind`] at the
/// SQLite boundary; do not extend `EdgeKind` itself with an
/// `Unknown(String)` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "sub")]
pub enum EdgeKind {
    /// Function call relationship
    Call,
    /// Containment relationship (e.g., module contains function)
    Contains,
    /// Import or use relationship
    Import,
    /// Class/interface inheritance: `class Child extends Parent` or
    /// `interface Child extends Parent`. A dedicated variant so downstream
    /// analysis (inheritance depth, LSP-class hierarchy, blast radius through
    /// base types) does not need to re-parse raw type refs.
    Extends,
    /// Interface implementation: `class Concrete implements MyInterface`.
    /// Separate from `Extends` so analysis can distinguish single-inheritance
    /// class chains from potentially many-to-many interface contracts.
    Implements,
    /// Generic type reference relationship — e.g. a function's parameter or
    /// return type references a class, a variable declaration's type, or
    /// any other type citation that is NOT an extends/implements clause.
    /// Extends and Implements used to collapse into this variant; they are
    /// now first-class (see `Extends` / `Implements` above).
    Type,
    /// Usage relationship (e.g., variable usage)
    Uses,
    /// Framework-dispatch edge — the Salesforce runtime (or analogous
    /// framework runtime) invokes the target at runtime based on a
    /// declarative binding in source code. The dispatch is real; the call
    /// is not visible in normal Apex-to-Apex control flow. Split from
    /// `Call` in universal-fidelity sprint T1 so downstream metrics can
    /// decide per-metric whether framework dispatch counts as a call.
    Framework(FrameworkKind),
    /// Declarative-wiring edge — configuration (XML / metadata / custom-
    /// metadata) invokes the target at runtime. Distinct from
    /// `Framework` because declarative wiring has no source-code binding
    /// site — the wiring lives entirely outside the language. Reserved
    /// for Salesforce Flow, Process Builder, Workflow Rules, and their
    /// non-Salesforce analogues (Spring XML, Django URLconf, etc.) as
    /// their readers land in future phases.
    Declarative(DeclarativeKind),
}

/// Concrete framework dispatch mechanism for `EdgeKind::Framework`.
///
/// **Only variants with a live emitter should be added here.** A variant
/// without an emitter is aspirational code — the exact pattern the
/// universal-fidelity sprint is designed to prevent. Add a variant in the
/// same PR as the reader / extractor / emission site that produces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FrameworkKind {
    /// Visualforce `{!binding}` on a `.page` file resolved to an Apex
    /// method on the page's controller chain. Emitted by the VF
    /// extraction stage (`application::use_cases::parse_repo::pipeline::
    /// vf_extraction`) as a
    /// [`UnresolvedReference::FrameworkBinding`] whose
    /// `framework` field carries this variant; the resolver pattern-
    /// matches on the enum to emit `EdgeKind::Framework(VisualforcePage)`.
    /// Pre-P1.d, this was threaded via a `CallSite::edge_kind_hint`
    /// field — replaced with a typed channel because the hint field was
    /// silently-broken-by-default.
    VisualforcePage,
}

// NOTE: `FrameworkKind` and `DeclarativeKind` are plain externally-
// tagged enums by default. A `VisualforcePage` variant serialises as
// the literal string `"VisualforcePage"`; combined with `EdgeKind`'s
// `#[serde(tag, content)]`, the full round-trip reads
// `{"kind":"Framework","sub":"VisualforcePage"}`.

/// Concrete declarative-wiring mechanism for `EdgeKind::Declarative`.
///
/// **Reserved taxonomy slot.** No variant has a live emitter today.
/// `Flow` is kept as the canonical first placeholder (see
/// `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md` — Salesforce Flow
/// XML → Apex dispatch is the first declarative reader scoped for the
/// post-sprint roadmap). Having one variant keeps `DeclarativeKind`
/// inhabited and lets downstream metric consumers compile their
/// inclusion decisions today, before any emitter ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DeclarativeKind {
    /// Salesforce Flow XML invoking an Apex class or invocable method.
    /// No emitter today; reader scoped for post-sprint.
    Flow,
}

impl EdgeKind {
    /// True for the "something invokes something else" family:
    /// `Call`, `Framework(_)`, `Declarative(_)`.
    ///
    /// Metrics whose definition is "A reaches B via call-like
    /// semantics" filter on this predicate (call-depth, layer topology,
    /// resolution-health). Metrics whose definition is narrower
    /// (e.g. "literal Apex method invocations") keep filtering on
    /// `== EdgeKind::Call`.
    pub fn is_call_like(self) -> bool {
        matches!(
            self,
            EdgeKind::Call | EdgeKind::Framework(_) | EdgeKind::Declarative(_)
        )
    }

    /// True only for `EdgeKind::Contains` — the purely structural
    /// parenthood relation. Every other variant is "structural" in the
    /// metric sense (participates in fan-in / fan-out / cycle / coupling
    /// analyses). See `is_structural()`.
    pub fn is_containment(self) -> bool {
        matches!(self, EdgeKind::Contains)
    }

    /// True for every variant except `Contains`. The default "include
    /// in structural metrics" predicate.
    pub fn is_structural(self) -> bool {
        !self.is_containment()
    }

    /// True for the static-dependency family: `Import`, `Type`, `Uses`.
    /// These edges represent "A statically references B" without a
    /// runtime invocation; they create incoming references for the
    /// purposes of dead-code fan-in but do not participate in call
    /// depth or layer topology.
    ///
    /// Predicate form (declared here, consumed by metric code) rather
    /// than a hardcoded variant list at every metric consumer. Adding
    /// a new dependency-flavoured variant means updating this one
    /// predicate; every caller picks the answer up automatically. See
    /// `NEW_ENGINEER_PRIMER.md` §8 Decision 5.
    pub fn is_dependency(self) -> bool {
        matches!(self, EdgeKind::Import | EdgeKind::Type | EdgeKind::Uses)
    }

    /// True for the subtyping family: `Extends`, `Implements`. Class-
    /// hierarchy-specific metrics (inheritance depth, base-class blast
    /// radius) filter on this predicate so `Type` references
    /// (parameter / return types, field types) do not bleed into
    /// hierarchy computations.
    pub fn is_inheritance(self) -> bool {
        matches!(self, EdgeKind::Extends | EdgeKind::Implements)
    }
}

/// Wire-boundary representation of an edge kind.
///
/// `EdgeKind` is the in-memory domain type: closed, `Copy`, used
/// throughout parsing and analysis. `PersistedEdgeKind` is the
/// read-side boundary representation at the SQLite column, introduced
/// in universal-fidelity sprint P1.c to dissolve Decision 1.
///
/// Forward-compat intent. A `parse.db` written by a newer engine
/// version may contain edge-kind JSON whose `kind` tag is not in this
/// engine's `EdgeKind` enum (e.g. a `Layer6` tag shipped after this
/// binary was built). Pre-P1.c, the reader dropped such edges
/// silently and without a count; now, `PersistedEdgeKind::from_wire`
/// captures the raw JSON as `Unknown(String)`, the caller counts
/// these occurrences, and the analysis pipeline surfaces the count
/// as `report::CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1` in the
/// `integrity_status.schema_caveats` of the resulting health report.
/// Downstream consumers can therefore distinguish "I saw a clean
/// graph" from "I saw a graph I was not fully equipped to read".
///
/// Why not `#[serde(untagged)]`. Serde's `untagged` only matches
/// structural shapes; a JSON object with an unknown `kind` tag is
/// still a JSON object, so an `Unknown(String)` arm under
/// `untagged` would never match. Doing the fallback manually in
/// `from_wire` is both simpler and more explicit, and it keeps
/// `EdgeKind` itself free of any wire-format derivation that the
/// in-memory type should not carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistedEdgeKind {
    /// The wire string deserialised into a known `EdgeKind` variant.
    Known(EdgeKind),
    /// The wire string did not match any known `EdgeKind` variant.
    /// Carries the raw column value so callers can log / count /
    /// re-persist it without loss.
    Unknown(String),
}

impl PersistedEdgeKind {
    /// Parse a SQLite `edges.kind` column value. Returns `Known`
    /// whenever the string deserialises cleanly into `EdgeKind`
    /// (via the serde tagged form); returns `Unknown(String)` for
    /// any value that cannot be deserialised (unknown tag, unknown
    /// sub-variant, malformed JSON, legacy hand-rolled format from
    /// pre-P1.b DBs). Never panics.
    pub fn from_wire(s: &str) -> Self {
        match serde_json::from_str::<EdgeKind>(s) {
            Ok(k) => Self::Known(k),
            Err(_) => Self::Unknown(s.to_owned()),
        }
    }
}

impl From<EdgeKind> for PersistedEdgeKind {
    fn from(k: EdgeKind) -> Self {
        Self::Known(k)
    }
}

impl TryFrom<PersistedEdgeKind> for EdgeKind {
    type Error = String;
    fn try_from(value: PersistedEdgeKind) -> Result<Self, Self::Error> {
        match value {
            PersistedEdgeKind::Known(k) => Ok(k),
            PersistedEdgeKind::Unknown(s) => Err(s),
        }
    }
}

/// An edge representing a relationship between two nodes
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Edge {
    /// ID of the source node
    pub from_id: String,
    /// ID of the destination node
    pub to_id: String,
    /// Type of relationship
    pub kind: EdgeKind,
    /// Provenance information
    pub provenance: Provenance,
}

impl Edge {
    /// Create a new edge with validation
    pub fn new(from_id: String, to_id: String, kind: EdgeKind, provenance: Provenance) -> Self {
        // Validate invariants
        Self::validate_edge(&from_id, &to_id, kind);

        Self {
            from_id,
            to_id,
            kind,
            provenance,
        }
    }

    /// Validate edge invariants
    fn validate_edge(from_id: &str, to_id: &str, kind: EdgeKind) {
        // Prevent self-loops for most edge types
        if from_id == to_id {
            match kind {
                EdgeKind::Contains => {
                    // Contains self-loops are allowed (e.g., module contains itself)
                }
                _ => {
                    // Production safety: never crash a parse run due to a single bad edge.
                    // In debug/test builds we still panic to catch the bug early.
                    if cfg!(debug_assertions) {
                        panic!("Invalid self-loop for edge kind: {:?}", kind);
                    } else {
                        tracing::warn!("Dropping invariant violation: self-loop edge {:?}", kind);
                    }
                }
            }
        }
    }

    /// Create a call edge
    pub fn call(from_id: String, to_id: String, provenance: Provenance) -> Self {
        Self::new(from_id, to_id, EdgeKind::Call, provenance)
    }

    /// Create a contains edge
    pub fn contains(from_id: String, to_id: String, provenance: Provenance) -> Self {
        Self::new(from_id, to_id, EdgeKind::Contains, provenance)
    }

    /// Create an import edge
    pub fn import(from_id: String, to_id: String, provenance: Provenance) -> Self {
        Self::new(from_id, to_id, EdgeKind::Import, provenance)
    }

    /// Create a type edge
    pub fn type_(from_id: String, to_id: String, provenance: Provenance) -> Self {
        Self::new(from_id, to_id, EdgeKind::Type, provenance)
    }

    /// Create a class-extends edge (single-inheritance chain).
    pub fn extends(from_id: String, to_id: String, provenance: Provenance) -> Self {
        Self::new(from_id, to_id, EdgeKind::Extends, provenance)
    }

    /// Create an interface-implements edge.
    pub fn implements(from_id: String, to_id: String, provenance: Provenance) -> Self {
        Self::new(from_id, to_id, EdgeKind::Implements, provenance)
    }

    /// Create a uses edge
    pub fn uses(from_id: String, to_id: String, provenance: Provenance) -> Self {
        Self::new(from_id, to_id, EdgeKind::Uses, provenance)
    }

    /// Create a framework-dispatch edge.
    ///
    /// Use this when the edge represents a framework-runtime invocation
    /// (e.g. the Salesforce runtime dispatching a Visualforce binding to
    /// an Apex controller method). The `kind` tells downstream metrics
    /// *which* framework emitted the edge so they can refine their
    /// inclusion decisions over time.
    pub fn framework(
        from_id: String,
        to_id: String,
        kind: FrameworkKind,
        provenance: Provenance,
    ) -> Self {
        Self::new(from_id, to_id, EdgeKind::Framework(kind), provenance)
    }

    /// Create a declarative-wiring edge. Reserved — no emitter today;
    /// the reader is scoped for post-sprint work. See
    /// `EdgeKind::Declarative` doc.
    pub fn declarative(
        from_id: String,
        to_id: String,
        kind: DeclarativeKind,
        provenance: Provenance,
    ) -> Self {
        Self::new(from_id, to_id, EdgeKind::Declarative(kind), provenance)
    }
}

#[cfg(test)]
mod copy_invariant {
    use super::*;

    /// Compile-time assertion that `EdgeKind` remains `Copy`. Deleting
    /// `#[derive(Copy)]` or adding a data-carrying variant that blocks
    /// `Copy` will fail this call at monomorphisation. Forward-compat
    /// with heap-allocated unknowns is the job of
    /// [`PersistedEdgeKind`] — not `EdgeKind` itself.
    fn _assert_copy<T: Copy>() {}

    #[test]
    fn edge_kind_is_copy() {
        _assert_copy::<EdgeKind>();
        _assert_copy::<FrameworkKind>();
        _assert_copy::<DeclarativeKind>();
    }
}

#[cfg(test)]
mod persisted_edge_kind_tests {
    use super::*;

    #[test]
    fn known_variants_round_trip_through_from_wire() {
        for kind in [
            EdgeKind::Call,
            EdgeKind::Contains,
            EdgeKind::Framework(FrameworkKind::VisualforcePage),
            EdgeKind::Declarative(DeclarativeKind::Flow),
        ] {
            let wire = serde_json::to_string(&kind).expect("serialize");
            let persisted = PersistedEdgeKind::from_wire(&wire);
            assert_eq!(persisted, PersistedEdgeKind::Known(kind));
            assert_eq!(EdgeKind::try_from(persisted), Ok(kind));
        }
    }

    #[test]
    fn unknown_wire_values_become_unknown_not_panic() {
        for wire in [
            r#"{"kind":"FutureLayer6","sub":"Whatever"}"#,
            r#"{"kind":"Framework","sub":"LwcTemplate"}"#, // valid tag, unknown sub
            r#"{"kind":""}"#,
            r#""Call""#,                 // legacy hand-rolled, string not object
            "Framework:VisualforcePage", // pre-P1.b hand-rolled
            "",
        ] {
            let persisted = PersistedEdgeKind::from_wire(wire);
            match persisted {
                PersistedEdgeKind::Unknown(s) => assert_eq!(s, wire),
                PersistedEdgeKind::Known(k) => {
                    panic!("wire value {wire} unexpectedly decoded as Known({k:?})")
                }
            }
        }
    }

    #[test]
    fn try_from_unknown_returns_err_with_raw_string() {
        let raw = r#"{"kind":"FutureLayer6"}"#.to_owned();
        let persisted = PersistedEdgeKind::Unknown(raw.clone());
        assert_eq!(EdgeKind::try_from(persisted), Err(raw));
    }
}

#[cfg(test)]
mod taxonomy_tests {
    use super::*;

    #[test]
    fn serde_round_trip_covers_every_variant() {
        let variants = [
            EdgeKind::Call,
            EdgeKind::Contains,
            EdgeKind::Import,
            EdgeKind::Extends,
            EdgeKind::Implements,
            EdgeKind::Type,
            EdgeKind::Uses,
            EdgeKind::Framework(FrameworkKind::VisualforcePage),
            EdgeKind::Declarative(DeclarativeKind::Flow),
        ];
        for v in variants {
            let s = serde_json::to_string(&v).expect("serialise");
            let back: EdgeKind = serde_json::from_str(&s).expect("deserialise");
            assert_eq!(back, v, "serde round-trip lost variant for {s}");
        }
    }

    #[test]
    fn is_call_like_covers_the_call_family_and_nothing_else() {
        assert!(EdgeKind::Call.is_call_like());
        assert!(EdgeKind::Framework(FrameworkKind::VisualforcePage).is_call_like());
        assert!(EdgeKind::Declarative(DeclarativeKind::Flow).is_call_like());
        for v in [
            EdgeKind::Contains,
            EdgeKind::Import,
            EdgeKind::Extends,
            EdgeKind::Implements,
            EdgeKind::Type,
            EdgeKind::Uses,
        ] {
            assert!(!v.is_call_like(), "{v:?} must not be call-like");
        }
    }

    #[test]
    fn is_dependency_covers_static_reference_family_and_nothing_else() {
        for v in [EdgeKind::Import, EdgeKind::Type, EdgeKind::Uses] {
            assert!(v.is_dependency(), "{v:?} must be a dependency");
        }
        for v in [
            EdgeKind::Call,
            EdgeKind::Contains,
            EdgeKind::Extends,
            EdgeKind::Implements,
            EdgeKind::Framework(FrameworkKind::VisualforcePage),
            EdgeKind::Declarative(DeclarativeKind::Flow),
        ] {
            assert!(!v.is_dependency(), "{v:?} must not be a dependency");
        }
    }

    #[test]
    fn is_inheritance_covers_subtyping_family_and_nothing_else() {
        for v in [EdgeKind::Extends, EdgeKind::Implements] {
            assert!(v.is_inheritance(), "{v:?} must be inheritance");
        }
        for v in [
            EdgeKind::Call,
            EdgeKind::Contains,
            EdgeKind::Import,
            EdgeKind::Type,
            EdgeKind::Uses,
            EdgeKind::Framework(FrameworkKind::VisualforcePage),
            EdgeKind::Declarative(DeclarativeKind::Flow),
        ] {
            assert!(!v.is_inheritance(), "{v:?} must not be inheritance");
        }
    }

    #[test]
    fn is_containment_is_strictly_contains() {
        assert!(EdgeKind::Contains.is_containment());
        for v in [
            EdgeKind::Call,
            EdgeKind::Import,
            EdgeKind::Extends,
            EdgeKind::Implements,
            EdgeKind::Type,
            EdgeKind::Uses,
            EdgeKind::Framework(FrameworkKind::VisualforcePage),
            EdgeKind::Declarative(DeclarativeKind::Flow),
        ] {
            assert!(!v.is_containment(), "{v:?} must not be containment");
            assert!(v.is_structural(), "{v:?} must be structural");
        }
    }
}
