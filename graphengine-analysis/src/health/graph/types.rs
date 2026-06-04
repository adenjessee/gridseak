//! Pure data types backing the in-memory analysis graph.
//!
//! Originally part of `health/graph.rs`. Split out by R1
//! (v0.1.0-rc1 follow-up) so the data model lives in a file that does
//! not depend on `rusqlite::Connection`, on the application crate's
//! classification helpers, or on the language-detection logic. This
//! file is the **bottom of the dependency tree** inside `health::graph`:
//! everything else in the sub-module imports `super::types::*`.
//!
//! Every type here mirrors the corresponding type in
//! `graphengine-parsing::domain`. The two must move in lockstep —
//! see `t1_edgekind_roundtrip.rs` on the parsing side for the wire-
//! string pin.

// ---------------------------------------------------------------------------
// Node / Edge kinds (mirror the parsing domain, kept local to avoid coupling)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Function,
    Struct,
    Module,
    Interface,
    Enum,
    Variable,
    Type,
    Import,
    Project,
    Crate,
    File,
    Folder,
}

impl NodeKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "Function" => Some(Self::Function),
            "Struct" => Some(Self::Struct),
            "Module" => Some(Self::Module),
            "Interface" => Some(Self::Interface),
            "Enum" => Some(Self::Enum),
            "Variable" => Some(Self::Variable),
            "Type" => Some(Self::Type),
            "Import" => Some(Self::Import),
            "Project" => Some(Self::Project),
            "Crate" => Some(Self::Crate),
            "File" => Some(Self::File),
            "Folder" => Some(Self::Folder),
            _ => None,
        }
    }

    pub fn is_function_like(self) -> bool {
        matches!(self, Self::Function)
    }

    pub fn is_module_like(self) -> bool {
        matches!(self, Self::Folder | Self::File | Self::Module)
    }

    pub fn is_container(self) -> bool {
        matches!(
            self,
            Self::Folder | Self::File | Self::Module | Self::Project | Self::Crate
        )
    }
}

/// Mirror of `graphengine_parsing::domain::EdgeKind`. Kept as a local
/// enum in the analysis crate to avoid a compile-time dependency on
/// parsing's full domain module; the two must be updated in lockstep.
/// See `graphengine_parsing::domain::edge` for the canonical taxonomy
/// documentation.
///
/// Wire format: the SQLite `edges.kind` column stores the serde JSON
/// produced by the parsing crate's `#[serde(tag = "kind", content =
/// "sub")]` derive. This mirror derives matching serde attributes so
/// `serde_json::from_str::<EdgeKind>(&kind_column)` decodes directly.
/// If the parsing-side format ever drifts from this mirror, the
/// `t1_edgekind_roundtrip.rs` wire-string pin trips a test on the
/// parsing side before any analysis consumer is affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "sub")]
pub enum EdgeKind {
    Call,
    Import,
    Uses,
    Type,
    /// Class/interface inheritance. Distinct from `Type` since Sprint E.1
    /// so inheritance-specific analysis (depth, base-class blast radius)
    /// does not have to demux `Type` edges after the fact.
    Extends,
    /// Interface implementation. Distinct from `Extends` so single-
    /// inheritance class chains are separable from potentially many-to-many
    /// interface contracts.
    Implements,
    Contains,
    /// Framework-dispatch edge — see parsing-side docs. Added in
    /// universal-fidelity T1 so VF-binding / LWC-template / framework-
    /// runtime invocations stop being coerced into `Call`.
    Framework(FrameworkKind),
    /// Declarative-wiring edge — see parsing-side docs. Reserved
    /// taxonomy slot; no emitter ships today.
    Declarative(DeclarativeKind),
}

/// Mirror of the parsing-side `FrameworkKind`. Keep in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FrameworkKind {
    VisualforcePage,
}

/// Mirror of the parsing-side `DeclarativeKind`. Keep in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DeclarativeKind {
    Flow,
}

/// Analysis-side mirror of `graphengine_parsing::domain::edge::PersistedEdgeKind`.
///
/// Used only at the SQLite load boundary (`AnalysisGraph::load_edges`).
/// Any column value that fails to deserialise into the closed
/// `EdgeKind` falls into `Unknown(String)`, carrying the raw wire
/// value so the loader can count it and surface
/// `CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1` in the resulting report. This
/// is the forward-compat receiver for parse DBs written by newer
/// engine versions; see `NEW_ENGINEER_PRIMER.md` §8 Decision 1 and
/// `T1-rework.md` §4.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistedEdgeKind {
    Known(EdgeKind),
    Unknown(String),
}

impl PersistedEdgeKind {
    pub fn from_wire(s: &str) -> Self {
        match serde_json::from_str::<EdgeKind>(s) {
            Ok(k) => Self::Known(k),
            Err(_) => Self::Unknown(s.to_owned()),
        }
    }
}

impl EdgeKind {
    pub fn is_containment(self) -> bool {
        matches!(self, Self::Contains)
    }

    pub fn is_structural(self) -> bool {
        !self.is_containment()
    }

    /// True for inheritance-flavored edges (`Extends` / `Implements`).
    /// Callers that want to compute class-hierarchy metrics use this to
    /// pick the exact edges that participate in a hierarchy, without
    /// having to include general `Type` references (parameter types,
    /// return types, field types).
    pub fn is_inheritance(self) -> bool {
        matches!(self, Self::Extends | Self::Implements)
    }

    /// True for the static-dependency family: `Import`, `Type`, `Uses`.
    /// Mirrors the parsing-side predicate of the same name. See
    /// `graphengine_parsing::domain::edge::EdgeKind::is_dependency` for
    /// the taxonomy rationale and `NEW_ENGINEER_PRIMER.md` §8 Decision
    /// 5 for the predicate-driven metric-filter design.
    pub fn is_dependency(self) -> bool {
        matches!(self, Self::Import | Self::Type | Self::Uses)
    }

    /// True for the "something invokes something else" family:
    /// `Call`, `Framework(_)`, `Declarative(_)`. Metrics whose
    /// definition is "A reaches B via call-like semantics" filter on
    /// this predicate. See `docs/workstreams/universal-fidelity/
    /// DISCOVERY_REPORT.md` §8 Decision 5 for the per-metric table.
    pub fn is_call_like(self) -> bool {
        matches!(self, Self::Call | Self::Framework(_) | Self::Declarative(_))
    }
}

// ---------------------------------------------------------------------------
// Graph node
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GraphNode {
    pub id: String,
    pub kind: NodeKind,
    pub fqn: String,
    pub name: String,

    // Location
    pub file_path: Option<String>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,

    // Classification (from properties JSON — only set on File/Folder/Project nodes)
    pub path_repo_rel: Option<String>,
    pub role: Option<String>,
    pub is_test: bool,
    pub is_vendor: bool,
    pub is_build_output: bool,
    pub is_generated: bool,

    // Complexity (from properties JSON — set on Function nodes by parsing crate)
    pub cyclomatic_complexity: Option<u32>,
    pub cognitive_complexity: Option<u32>,

    // Visibility (from properties JSON — set by parsing crate's visibility detector)
    pub visibility: Option<String>,

    // Import source modules (from properties JSON — set on __file_module__ nodes)
    pub import_sources: Vec<String>,

    // Trait metadata (from trait_metadata column — set by TraitContextDetector)
    pub is_trait_impl: bool,
    pub trait_name: Option<String>,

    // Attribute/callback entry-point signals (from properties JSON)
    pub is_attribute_invoked: bool,
    pub is_callback_target: bool,

    /// Entry-point tags emitted by the parsing crate when it
    /// recognises a framework-specific signal on the declaration
    /// (Apex `@AuraEnabled`, `@InvocableMethod`, `@RestResource`,
    /// `global`, `webservice`, `implements Queueable`, etc.). Each
    /// tag is a short stable string (e.g. `"aura_enabled"`); see
    /// `graphengine-parsing::syntax::language::apex::entry_points`
    /// for the authoritative list. Empty for languages whose
    /// extractor does not emit the property.
    pub entry_point_tags: Vec<String>,

    /// Language of the file this node lives in (lower-case, e.g.
    /// `"apex"`, `"python"`, `"typescript"`). Materialised at graph
    /// load time: `File` nodes read it straight from their own
    /// `properties.language`, every other node inherits from its
    /// parent `File` via `file_path_index`. This replaces the
    /// previous pattern of calling `detect_ecosystem(conn)` and
    /// dispatching a single global `Ecosystem` value per graph —
    /// polyglot repos (e.g. NPSP Apex + LWC JavaScript + Python
    /// build scripts) need per-node language to dispatch
    /// classifier rules correctly. `None` when the extractor did
    /// not stamp `language` on the File (older DBs, unknown
    /// extensions).
    pub language: Option<String>,

    /// Frameworks detected on the file this node lives in (e.g.
    /// `["tdtm", "restresource"]` for an NPSP TDTM handler that
    /// also exposes a REST endpoint, `["django"]` for a
    /// `views.py`, `["lwc"]` for a `c-foo.js` under
    /// `lwc/foo/foo.js`). Populated in Wave 2.2 by
    /// `graphengine-parsing::domain::frameworks`. Stored as a
    /// `Vec<String>` rather than an enum so new frameworks can be
    /// added by the detector without the analysis crate having to
    /// recompile its taxonomy (R27). Duplicates are not removed
    /// at load — detectors are expected to produce a deduplicated
    /// list.
    pub frameworks: Vec<String>,

    /// True when the parsing crate emitted this node as a synthetic
    /// modeling artifact rather than a user-authored declaration
    /// (materialised from `properties.synthetic`). Synthetic nodes
    /// exist purely so the heuristic resolver has a caller to
    /// attribute enclosed call sites to — e.g. Apex's `__trigger__`
    /// body wrapper, R41 field-initializer `<field>.__init__()`
    /// wrappers, or R39 property-accessor `<prop>.__get__()` /
    /// `<prop>.__set__()` wrappers. Because they do not correspond
    /// to code the user can delete or rename, they MUST be excluded
    /// from dead-code candidacy (they would otherwise flood
    /// `no_callers_high_confidence` with fixable-looking but
    /// unfixable findings) and from surfaces that enumerate
    /// "meaningful" functions (API surface, cycle detection).
    pub is_synthetic: bool,
}

impl Default for GraphNode {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: NodeKind::Function,
            fqn: String::new(),
            name: String::new(),
            file_path: None,
            start_line: None,
            end_line: None,
            path_repo_rel: None,
            role: None,
            is_test: false,
            is_vendor: false,
            is_build_output: false,
            is_generated: false,
            cyclomatic_complexity: None,
            cognitive_complexity: None,
            visibility: None,
            import_sources: Vec::new(),
            is_trait_impl: false,
            trait_name: None,
            is_attribute_invoked: false,
            is_callback_target: false,
            entry_point_tags: Vec::new(),
            language: None,
            frameworks: Vec::new(),
            is_synthetic: false,
        }
    }
}

impl GraphNode {
    pub(super) fn extract_name(fqn: &str) -> String {
        fqn.rsplit("::").next().unwrap_or(fqn).to_string()
    }

    /// Returns true if this node is marked as publicly visible / exported.
    pub fn is_exported(&self) -> bool {
        matches!(
            self.visibility.as_deref(),
            Some("public" | "exported" | "pub_crate")
        )
    }

    /// Short but disambiguated name: includes the parent module segment.
    /// e.g. "router::encode" instead of just "encode".
    pub fn display_name(&self) -> String {
        let parts: Vec<&str> = self.fqn.rsplitn(3, "::").collect();
        if parts.len() >= 2 {
            format!("{}::{}", parts[1], parts[0])
        } else {
            self.name.clone()
        }
    }
}

// ---------------------------------------------------------------------------
// Graph edge
// ---------------------------------------------------------------------------

/// Mirror of `graphengine_parsing::domain::provenance::Confidence`.
/// Kept local to avoid a compile-time coupling to the parsing crate;
/// parsed from the `confidence` field of the edge's `provenance` JSON
/// blob stored in SQLite.
///
/// `Unknown` is the explicit fallback for old parse.dbs that did not
/// stamp a confidence level (pre-Sprint E.1 DBs) and for rows where the
/// JSON is malformed. Consumers that distinguish "authoritative" from
/// "heuristic" MUST match on `High` explicitly rather than on `!= Low`,
/// because a `Medium` edge is still a heuristic edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    High,
    Medium,
    Low,
    Unknown,
}

impl Confidence {
    pub fn parse(s: &str) -> Self {
        match s {
            "High" => Self::High,
            "Medium" => Self::Medium,
            "Low" => Self::Low,
            _ => Self::Unknown,
        }
    }

    /// True only for `High`. Load-bearing for T3's dual-metric emission:
    /// every Layer-3 metric is computed twice — once over all production
    /// structural edges, once over just the edges this returns `true`
    /// for — so the report can surface the "fidelity gap" between a
    /// graph whose call-resolution is authoritative and one whose call-
    /// resolution is dominated by Tree-sitter / heuristic fallbacks.
    pub fn is_high(self) -> bool {
        matches!(self, Self::High)
    }
}

#[derive(Debug, Clone)]
pub struct GraphEdge {
    pub from_id: String,
    pub to_id: String,
    pub kind: EdgeKind,
    /// Authority stamp attached by the parsing crate. `High` means the
    /// edge came from an authoritative source (LSP, compiler library,
    /// or a hand-proven heuristic arm). `Medium` / `Low` mean heuristic.
    /// `Unknown` indicates the edge row had a missing or malformed
    /// `provenance` column (old DBs or corrupted writes).
    pub confidence: Confidence,
}

#[cfg(test)]
mod edgekind_predicate_tests {
    use super::*;

    #[test]
    fn is_call_like_covers_call_family_only() {
        assert!(EdgeKind::Call.is_call_like());
        assert!(EdgeKind::Framework(FrameworkKind::VisualforcePage).is_call_like());
        assert!(EdgeKind::Declarative(DeclarativeKind::Flow).is_call_like());
        for v in [
            EdgeKind::Contains,
            EdgeKind::Import,
            EdgeKind::Type,
            EdgeKind::Uses,
            EdgeKind::Extends,
            EdgeKind::Implements,
        ] {
            assert!(!v.is_call_like(), "{v:?} must not be call-like");
        }
    }

    #[test]
    fn is_dependency_covers_static_ref_family_only() {
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
    fn is_inheritance_covers_subtyping_family_only() {
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
    fn is_structural_excludes_only_containment() {
        assert!(!EdgeKind::Contains.is_structural());
        for v in [
            EdgeKind::Call,
            EdgeKind::Import,
            EdgeKind::Type,
            EdgeKind::Uses,
            EdgeKind::Extends,
            EdgeKind::Implements,
            EdgeKind::Framework(FrameworkKind::VisualforcePage),
            EdgeKind::Declarative(DeclarativeKind::Flow),
        ] {
            assert!(v.is_structural(), "{v:?} must be structural");
        }
    }
}
