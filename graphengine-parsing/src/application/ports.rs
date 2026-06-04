//! Port definitions for dependency inversion
//!
//! These traits define the interfaces that infrastructure adapters must implement.
//! The application layer depends on these abstractions, not concrete implementations.
//! This enables testability (via mocks) and extensibility (swap adapters).

use crate::domain::apex::class_symbols::ApexTypeRef;
use crate::domain::{DeclarativeKind, Edge, FrameworkKind, Graph, Node, NodeKind, Range};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

/// A call site with the function name being called.
///
/// Post-P1.d (universal-fidelity sprint): framework-dispatch context
/// no longer lives on `CallSite` as an `Option<EdgeKind>` hint; it
/// lives in sibling variants of [`UnresolvedReference`]. A
/// `CallSite` now represents *exactly* "a plain call site that emits
/// `EdgeKind::Call` when resolved" — it cannot carry or request any
/// other variant. See `docs/workstreams/universal-fidelity/tasks/
/// T1-rework.md` §4.4 for the reasoning.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CallSite {
    /// Location of the call site
    pub location: Range,
    /// Name of the function being called
    pub function_name: String,
    /// Location of the receiver (for method calls), if available
    pub receiver_range: Option<Range>,
    /// Raw source-text of the receiver expression, if available.
    /// Populated alongside `receiver_range` by the call-site extractor
    /// whenever the YAML query captured a `@receiver` node. Apex's
    /// TR-A.3 field-type-aware dispatch consumes this text to look up
    /// the receiver in the local-var scope / enclosing-class fields /
    /// parent-chain. Kept as a plain string (not a pre-parsed
    /// expression) because the resolver needs to distinguish
    /// `permissionsService`, `this.permissionsService`, and
    /// `Outer.Inner` lexically before it decides how to resolve it.
    ///
    /// Non-Apex languages still populate this field when the query
    /// captured `@receiver` — the string is byte-identical with what
    /// the LSP fallback would have computed — but no resolver arm
    /// consumes it outside Apex today.
    pub receiver_text: Option<String>,
    /// Per-argument type inference for Apex overload disambiguation
    /// (TR-A.1 / TR-A.2). Populated by
    /// `LanguageSpecificExtractor::infer_call_site_arg_types`; empty for
    /// languages whose extractors do not infer argument types.
    pub arg_types: Vec<ApexTypeRef>,
}

/// A framework-dispatch binding discovered at extraction time.
///
/// Introduced in universal-fidelity sprint P1.d (T1 rework) to replace
/// `CallSite.edge_kind_hint: Option<EdgeKind>`. The field-on-CallSite
/// approach was silently-broken-by-default: any resolver arm that
/// forgot to consult the hint quietly emitted `EdgeKind::Call` where a
/// `Framework(_)` was intended, with no compile error and no runtime
/// warning. The typed-variant [`UnresolvedReference::FrameworkBinding`]
/// makes that failure impossible to express: a resolver that matches
/// exhaustively on [`UnresolvedReference`] *must* handle the
/// `FrameworkBinding` arm, and the `framework: FrameworkKind` field
/// unambiguously identifies which `Framework(_)` variant the edge must
/// carry. See `NEW_ENGINEER_PRIMER.md` §8 Decision 3.
///
/// Shape note. The binding wraps a `CallSite` rather than replicating
/// its fields so the existing LSP / heuristic resolution pipeline can
/// re-use the same `CallSite` contract (location, receiver text, arg
/// types for overload disambiguation) without branching its internals.
/// Only the *edge emission* differs between `Call` and `FrameworkBinding`
/// — the semantic lookup is identical.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FrameworkBinding {
    /// Which framework's runtime dispatches this binding. Determines
    /// the `EdgeKind::Framework(_)` variant emitted on resolution.
    pub framework: FrameworkKind,
    /// The call-site-shaped context the resolver consumes. Reuses
    /// `CallSite` so LSP / heuristic resolution paths remain a single
    /// implementation; only the post-resolution edge-emission differs.
    pub call_site: CallSite,
}

/// A declarative-wiring binding discovered outside source code (e.g.
/// Salesforce Flow XML invoking an Apex `@InvocableMethod`). Reserved
/// taxonomy slot introduced alongside [`FrameworkBinding`] so the
/// resolver match is exhaustive across the full "something outside
/// the language invokes a method" family. No emitter today; the first
/// emitter lands when the Flow reader ships (post-sprint).
///
/// Carrying this variant at the port layer now (with zero producers)
/// is a deliberate discipline call: it forces every future resolver to
/// decide how to handle declarative dispatch at authoring time — not
/// at "oh, we forgot to update the resolver when Flow shipped" time.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeclarativeBinding {
    /// Which declarative mechanism emitted the binding.
    pub declarative: DeclarativeKind,
    /// Call-site-shaped context for the resolver. Mirrors
    /// [`FrameworkBinding::call_site`].
    pub call_site: CallSite,
}

/// A reference discovered during syntax extraction that a later
/// resolver pass must bind to a concrete node.
///
/// Every syntactic artefact that will (or won't, on failure) become an
/// edge in the graph flows through this enum. The resolver pipeline
/// dispatches on the variant:
///
/// - [`UnresolvedReference::Call`] resolves to an `EdgeKind::Call` edge.
/// - [`UnresolvedReference::FrameworkBinding`] resolves to an
///   `EdgeKind::Framework(_)` edge whose sub-kind is carried by
///   `FrameworkBinding::framework`.
/// - [`UnresolvedReference::DeclarativeBinding`] resolves to an
///   `EdgeKind::Declarative(_)` edge.
///
/// The compile-time contract. A resolver that pattern-matches on this
/// enum cannot silently drop a new variant: adding a variant without
/// updating the match fails to compile (see the `exhaustive_dispatch`
/// unit test below for an enforcement fixture). This is the
/// dissolving-element for Decision 3 — the "extract-time knows
/// framework, resolve-time knows target" split stops being a
/// negotiation between `edge_kind_hint` and a plain `CallSite`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum UnresolvedReference {
    /// A plain call site. Emits `EdgeKind::Call` on resolution.
    Call(CallSite),
    /// A framework-runtime binding (e.g. Visualforce `{!method}`,
    /// LWC template dispatch). Emits `EdgeKind::Framework(_)`.
    FrameworkBinding(FrameworkBinding),
    /// A declarative-wiring binding (e.g. Flow XML → Apex method).
    /// Emits `EdgeKind::Declarative(_)`. Reserved; no emitter today.
    DeclarativeBinding(DeclarativeBinding),
}

impl UnresolvedReference {
    /// The underlying call-site-shaped context common to every
    /// variant. Every resolver consumes `location`, `function_name`,
    /// `receiver_range`, `receiver_text`, `arg_types` identically;
    /// exposing them through this accessor keeps the resolver
    /// implementation free of per-variant field-access plumbing.
    pub fn call_site(&self) -> &CallSite {
        match self {
            Self::Call(cs) => cs,
            Self::FrameworkBinding(fb) => &fb.call_site,
            Self::DeclarativeBinding(db) => &db.call_site,
        }
    }

    /// The `EdgeKind` the resolver must emit when binding to a target
    /// node succeeds. Determined by the variant — never by a hint
    /// field. Delete an arm and the compiler refuses to build; that
    /// property is the dissolving element for Decision 3.
    pub fn edge_kind(&self) -> crate::domain::EdgeKind {
        use crate::domain::EdgeKind;
        match self {
            Self::Call(_) => EdgeKind::Call,
            Self::FrameworkBinding(fb) => EdgeKind::Framework(fb.framework),
            Self::DeclarativeBinding(db) => EdgeKind::Declarative(db.declarative),
        }
    }
}

/// A type reference with the type name and its usage context
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TypeReference {
    /// Location of the type reference
    pub location: Range,
    /// Name of the type being referenced
    pub type_name: String,
    /// Kind of type usage (parameter, return type, property, extends, implements)
    pub usage_kind: TypeUsageKind,
}

/// How a type is being used
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TypeUsageKind {
    /// Type annotation on a parameter: `(x: MyType)`
    Parameter,
    /// Return type annotation: `(): MyType`
    ReturnType,
    /// Property/field type: `name: MyType`
    Property,
    /// Generic type argument: `Array<MyType>`
    Generic,
    /// Extends clause: `extends MyType`
    Extends,
    /// Implements clause: `implements MyInterface`
    Implements,
    /// Variable type annotation: `const x: MyType`
    Variable,
    /// General type reference (unknown context)
    Other,
}

/// Confidence level attached to an extraction-coverage record.
///
/// Introduced in T8 (universal-fidelity sprint) alongside
/// [`FileExtractionCoverage`]. Kept local to `graphengine-parsing`
/// rather than imported from `graphengine-git-signals` because the
/// parsing crate must not depend on Layer-0 signal machinery — the
/// two crates can evolve their confidence semantics independently
/// (parsing confidence reflects parse quality; Layer-0 confidence
/// reflects repository shape and history depth).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CoverageConfidence {
    /// Coverage count is authoritative: whole-file parse, no tree-sitter
    /// errors, extractor query set ran to completion.
    High,
    /// Coverage count is approximate: partial parse, extractor timed out,
    /// or a post-extract pass reported non-fatal inconsistency.
    Medium,
    /// Coverage count cannot be trusted: tree-sitter returned a parse
    /// error, the file is binary/encoding-broken, or the extractor
    /// panicked. Consumers must treat the record as evidence only that
    /// the file exists.
    Low,
}

/// A specific unwalked AST region identified in a parsed file.
///
/// Introduced in T8 (universal-fidelity sprint) as the dissolving
/// element for the "R39 / R41 / …" hard-coded variant-list problem
/// (see `docs/workstreams/universal-fidelity/tasks/T8-coverage-awareness.md`
/// §3 Q2). Each variant carries a count of occurrences in the file;
/// classifiers query predicates (`invalidates_*`) rather than matching
/// on variants directly, so adding a new gap kind is a one-line
/// predicate-update rather than an audit of every metric.
///
/// Adding a new variant without extending the predicate set is
/// intentionally a review signal — see §4 "Compile-time guarantees"
/// of the T8 design doc. Wire format uses serde tag/content so a
/// persisted older variant round-trips cleanly even after new
/// variants land.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "sub")]
pub enum CoverageGap {
    /// R39 — Apex property-accessor body (`get { ... }` / `set { ... }`).
    /// The current Apex extractor walks the property declaration but
    /// not the accessor body, so calls inside accessors are invisible
    /// to the graph.
    ApexPropertyAccessor { count: u32 },
    /// R41 — Apex map/list-literal field initializer containing method
    /// calls (`Map<Id, Foo> m = new Map<Id, Foo>{ key => f() };`).
    /// Initializer expression calls are dropped by the extractor.
    ApexMapLiteralInitializer { count: u32 },
    /// Apex trigger bodies whose call sites are not captured by the
    /// current extractor query set. Declared now so classifiers that
    /// consult trigger coverage can do so without a schema migration
    /// when the emitter lands.
    ApexTriggerBodyUncaptured { count: u32 },
}

impl CoverageGap {
    /// Number of occurrences of this gap shape in the file.
    pub fn count(&self) -> u32 {
        match self {
            CoverageGap::ApexPropertyAccessor { count }
            | CoverageGap::ApexMapLiteralInitializer { count }
            | CoverageGap::ApexTriggerBodyUncaptured { count } => *count,
        }
    }

    /// Does this gap invalidate `dead_code.no_callers` for the file?
    ///
    /// Yes for gaps that hide *outgoing* call edges — if the extractor
    /// did not walk a region that might contain `someMethod(…)`, then
    /// `someMethod` will appear uncalled in the graph even when the
    /// source text shows otherwise. R39 and R41 match this shape; the
    /// trigger-body variant is declared for future coverage without
    /// yet having an emitter.
    ///
    /// A gap with `count == 0` is treated as non-invalidating — a
    /// variant carrying no observed instances records an intentional
    /// reservation of the gap shape for the schema but contributes
    /// no evidence. This lets callers materialise a `CoverageGap`
    /// variant without downgrading anything (useful for test
    /// fixtures and future telemetry-only extensions).
    pub fn invalidates_no_callers(&self) -> bool {
        self.count() > 0
            && matches!(
                self,
                CoverageGap::ApexPropertyAccessor { .. }
                    | CoverageGap::ApexMapLiteralInitializer { .. }
                    | CoverageGap::ApexTriggerBodyUncaptured { .. }
            )
    }

    /// Does this gap invalidate `cycles`?
    ///
    /// No. `cycles` only consumes edges that *exist* in the graph; a
    /// missing edge cannot produce a false cycle, only hide a real
    /// one. Hiding is a different failure mode that `fan_in` covers.
    pub fn invalidates_cycles(&self) -> bool {
        false
    }

    /// Does this gap invalidate fan-in-based metrics (`fan_in_total`,
    /// `call_graph_depth`, …)?
    ///
    /// Yes. Fan-in is the sum of incoming call edges; an unwalked
    /// region can hide any number of them. More sensitive than
    /// `no_callers` because fan-in is continuous, not boolean — even
    /// a single missed edge perturbs the metric.
    pub fn invalidates_fan_in_metrics(&self) -> bool {
        self.count() > 0
            && matches!(
                self,
                CoverageGap::ApexPropertyAccessor { .. }
                    | CoverageGap::ApexMapLiteralInitializer { .. }
                    | CoverageGap::ApexTriggerBodyUncaptured { .. }
            )
    }

    /// Does this gap invalidate coupling metrics (cross-module edge
    /// density, module cohesion)?
    ///
    /// Yes when the unwalked region may contain cross-file references.
    /// Property accessors and map-literal initializers regularly call
    /// helper classes, so they count. Trigger bodies likewise.
    pub fn invalidates_coupling(&self) -> bool {
        self.count() > 0
            && matches!(
                self,
                CoverageGap::ApexPropertyAccessor { .. }
                    | CoverageGap::ApexMapLiteralInitializer { .. }
                    | CoverageGap::ApexTriggerBodyUncaptured { .. }
            )
    }

    /// Stable, grep-able discriminator used in health-report caveats.
    /// Wire format is the serde tag; this helper is for non-serde
    /// consumers (log lines, caveat strings).
    pub fn discriminator(&self) -> &'static str {
        match self {
            CoverageGap::ApexPropertyAccessor { .. } => "apex_property_accessor",
            CoverageGap::ApexMapLiteralInitializer { .. } => "apex_map_literal_initializer",
            CoverageGap::ApexTriggerBodyUncaptured { .. } => "apex_trigger_body_uncaptured",
        }
    }
}

/// Per-file extraction-coverage record.
///
/// Introduced in T8 (universal-fidelity sprint) — see
/// `docs/workstreams/universal-fidelity/tasks/T8-coverage-awareness.md`
/// §4.1. Attached to [`SyntaxResults::extraction_coverage`] by each
/// language's post-extract pass; consumed by
/// `graphengine-analysis::health::dead_code_classifier` to decide
/// whether a `no_callers` candidate in this file deserves
/// `High` confidence or must be downgraded.
///
/// The record separates two responsibilities the old `ExtractionStats`
/// conflated (§3 Q1 dissolving element): *what AST nodes exist* vs
/// *which of them the extractor walked*. `walked_node_count` and
/// `unwalked_node_count` are the raw axis; `coverage_gaps` is the
/// classifier-facing axis populated only with named, D2-validated
/// gap shapes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileExtractionCoverage {
    /// File the record describes. Repository-relative when the
    /// `SyntaxResults` workspace root is set, else absolute.
    pub file_path: std::path::PathBuf,
    /// Source language (matches `SyntaxResults::language` when the
    /// parse is homogeneous; a per-file field is kept so a future
    /// multi-language scan does not need schema rework).
    pub language: String,
    /// Count of AST nodes matched by the extractor's query set.
    pub walked_node_count: u32,
    /// Count of AST nodes the extractor did *not* walk. Includes
    /// structural nodes (block, expression, …) that never warrant an
    /// extractor query, so the raw ratio is not itself a useful
    /// metric — consumers should filter via the named `coverage_gaps`
    /// list.
    pub unwalked_node_count: u32,
    /// Named, classifier-relevant gap shapes. Empty when the file's
    /// unwalked regions contain nothing metric-sensitive; non-empty
    /// when the post-extract pass matched a known gap pattern.
    pub coverage_gaps: Vec<CoverageGap>,
    /// Confidence the counts above are accurate. See
    /// [`CoverageConfidence`].
    pub confidence: CoverageConfidence,
}

impl FileExtractionCoverage {
    /// Does this record contain any coverage gap that invalidates the
    /// `no_callers` metric? Classifiers that care only about the
    /// binary answer can call this helper rather than iterating
    /// `coverage_gaps` by hand.
    pub fn has_invalidating_no_callers_gap(&self) -> bool {
        self.coverage_gaps
            .iter()
            .any(CoverageGap::invalidates_no_callers)
    }

    /// Sum of counts across all gap variants in this file.
    pub fn total_gap_count(&self) -> u32 {
        self.coverage_gaps.iter().map(CoverageGap::count).sum()
    }
}

/// Results from syntax extraction phase
/// Contains preliminary nodes and hints for semantic resolution
#[derive(Debug, Clone)]
pub struct SyntaxResults {
    /// Extracted symbols (functions, structs, modules, etc.)
    pub symbols: Vec<Node>,
    /// All discovered source files for this parse (including files with zero symbols)
    pub source_files: Vec<String>,
    /// Workspace root for this parse run
    pub workspace_root: Option<String>,
    /// Language identifier for this parse run
    pub language: Option<String>,
    /// Identifier uses (variable references) for usage edges
    pub identifier_uses: Vec<IdentifierUse>,
    /// Unresolved references discovered at extraction time.
    ///
    /// Post-P1.d: the old `call_sites: Vec<CallSite>` field was renamed
    /// to `references: Vec<UnresolvedReference>`. The rename is
    /// deliberate — the collection now holds Call sites, framework
    /// bindings, and declarative bindings as separate variants rather
    /// than a bag of `CallSite`s with an `Option<EdgeKind>` hint.
    /// Consumers that want to iterate only the `Call` variant use
    /// [`SyntaxResults::call_sites`] (method, not field). See
    /// `docs/workstreams/universal-fidelity/tasks/T1-rework.md` §4.4.
    pub references: Vec<UnresolvedReference>,
    /// Legacy import ranges (deprecated; use `import_specs`)
    pub imports: Vec<Range>,
    /// Type references that need resolution (legacy - just ranges)
    pub type_refs: Vec<Range>,
    /// Structured type references with names and usage context
    pub type_references: Vec<TypeReference>,
    /// Structured import specifications extracted from `use` declarations
    pub import_specs: Vec<ImportSpec>,
    /// Module declarations (`mod foo;` / inline modules)
    pub mod_decls: Vec<ModDecl>,
    /// Edges produced deterministically during syntax extraction that do
    /// not require any semantic-resolver pass.
    ///
    /// The canonical use is Apex's managed-package detection: the parser
    /// identifies consumer → external-namespace dependencies directly from
    /// token text (e.g. `npsp.Foo`, `npe01__Something__c`), and there is
    /// nothing for an LSP or heuristic resolver to resolve — the
    /// external Module node is synthesized with a stable namespace-keyed
    /// id, and the Import edge connects the enclosing file-module node to
    /// it with `ProvenanceSource::Heuristic` / `Confidence::High`.
    ///
    /// `GraphBuilder::build_from_results` adds these edges to the final
    /// graph alongside the resolver-produced edges.
    pub synthesized_edges: Vec<Edge>,
    /// Per-class structural symbol tables produced by
    /// [`crate::syntax::language::LanguageSpecificExtractor::extract_class_symbols`]
    /// and keyed as `(dotted_api_name, symbols_json)`.
    ///
    /// Apex populates this in TR-A.0 to back the type oracle that
    /// later PRs consume for constructor / field-type / overload /
    /// inner-class dispatch. The json payload is the language's
    /// stable serialisation of its class-symbols struct — opaque to
    /// the generic pipeline, which only persists it to a per-language
    /// DB table (`apex_class_symbols` for Apex).
    ///
    /// Non-Apex languages default to an empty vec because their
    /// `extract_class_symbols` hook is a no-op. An empty vec at
    /// persistence time is a no-op SQL write, so the byte-identical
    /// rev-6.1 regression gate remains untouched for all non-Apex
    /// DBs.
    pub class_symbols: Vec<(String, String)>,
    /// Per-method local-variable scopes emitted by
    /// [`crate::syntax::language::LanguageSpecificExtractor::extract_local_var_scopes`].
    /// Ephemeral (never persisted to the parse DB) — consumed during
    /// semantic resolution only. Apex populates this in TR-A.3 so the
    /// field-type-aware dispatch resolver can look up
    /// `someLocalVar.method(...)` by walking the local scope first
    /// (before enclosing-class fields / parent-chain). Non-Apex
    /// languages default to an empty vec.
    pub local_var_scopes: Vec<LocalVarScope>,
    /// Per-file extraction-coverage records. One entry per parsed
    /// file for languages whose extractor produces a coverage pass;
    /// empty for languages not yet instrumented. Consumed by
    /// `graphengine-analysis::health::dead_code_classifier` to
    /// downgrade `no_callers` confidence on files with named
    /// invalidating gaps. See T8 design doc §4.2.
    pub extraction_coverage: Vec<FileExtractionCoverage>,
}

impl SyntaxResults {
    /// Create empty syntax results
    pub fn new() -> Self {
        Self {
            symbols: Vec::new(),
            source_files: Vec::new(),
            workspace_root: None,
            language: None,
            identifier_uses: Vec::new(),
            references: Vec::new(),
            imports: Vec::new(),
            type_refs: Vec::new(),
            type_references: Vec::new(),
            import_specs: Vec::new(),
            mod_decls: Vec::new(),
            synthesized_edges: Vec::new(),
            class_symbols: Vec::new(),
            local_var_scopes: Vec::new(),
            extraction_coverage: Vec::new(),
        }
    }

    /// Register an edge produced deterministically during syntax
    /// extraction (no resolver needed). See the `synthesized_edges`
    /// field docs for the Apex managed-package use case.
    pub fn add_synthesized_edge(&mut self, edge: Edge) {
        self.synthesized_edges.push(edge);
    }

    /// Set discovered source files for this parse run
    pub fn set_source_files(&mut self, files: Vec<String>) {
        self.source_files = files;
    }

    /// Set the workspace root for this parse run
    pub fn set_workspace_root(&mut self, root: String) {
        self.workspace_root = Some(root);
    }

    /// Set the language identifier for this parse run
    pub fn set_language(&mut self, language: String) {
        self.language = Some(language);
    }

    /// Add an identifier use (variable reference)
    pub fn add_identifier_use(&mut self, location: Range, name: String) {
        self.identifier_uses.push(IdentifierUse { location, name });
    }

    /// Add a plain call site (emits `EdgeKind::Call` on resolution).
    pub fn add_call_site(&mut self, location: Range, function_name: String) {
        self.add_call_site_with_receiver(location, function_name, None);
    }

    /// Add a plain call site with a receiver range.
    pub fn add_call_site_with_receiver(
        &mut self,
        location: Range,
        function_name: String,
        receiver_range: Option<Range>,
    ) {
        self.push_call(CallSite {
            location,
            function_name,
            receiver_range,
            receiver_text: None,
            arg_types: Vec::new(),
        });
    }

    /// Add a plain call site carrying inferred per-argument types.
    /// Used by Apex (TR-A.1 / TR-A.2) to feed the overload-
    /// disambiguation resolver; other languages use the receiver-only
    /// form above.
    pub fn add_call_site_full(
        &mut self,
        location: Range,
        function_name: String,
        receiver_range: Option<Range>,
        arg_types: Vec<ApexTypeRef>,
    ) {
        self.push_call(CallSite {
            location,
            function_name,
            receiver_range,
            receiver_text: None,
            arg_types,
        });
    }

    /// Add a plain call site carrying receiver text and inferred
    /// arg types. Used by Apex (TR-A.3) — the resolver needs both the
    /// receiver's raw source text (to look up field / local-var
    /// declared type) and the argument types (for overload
    /// disambiguation at the same site).
    pub fn add_call_site_with_receiver_text(
        &mut self,
        location: Range,
        function_name: String,
        receiver_range: Option<Range>,
        receiver_text: Option<String>,
        arg_types: Vec<ApexTypeRef>,
    ) {
        self.push_call(CallSite {
            location,
            function_name,
            receiver_range,
            receiver_text,
            arg_types,
        });
    }

    /// Push a pre-built `CallSite` as the `Call` variant of
    /// [`UnresolvedReference`]. Convenience helper for tests and
    /// extractors that already have a populated `CallSite` in hand.
    pub fn push_call(&mut self, call_site: CallSite) {
        self.references.push(UnresolvedReference::Call(call_site));
    }

    /// Register a framework-dispatch binding. Used by framework-aware
    /// extraction stages (e.g. the Visualforce page extractor) so the
    /// resolver emits the typed `Framework(_)` variant by construction,
    /// not via a fallible hint check. See [`FrameworkBinding`] for the
    /// type-channel rationale (P1.d, Decision 3).
    pub fn add_framework_binding(&mut self, framework: FrameworkKind, call_site: CallSite) {
        self.references
            .push(UnresolvedReference::FrameworkBinding(FrameworkBinding {
                framework,
                call_site,
            }));
    }

    /// Register a declarative-wiring binding. Reserved; no emitter
    /// today. Matches the shape of [`Self::add_framework_binding`] so
    /// the first declarative reader (Flow XML) plugs in without new
    /// ports-surface churn.
    pub fn add_declarative_binding(&mut self, declarative: DeclarativeKind, call_site: CallSite) {
        self.references
            .push(UnresolvedReference::DeclarativeBinding(
                DeclarativeBinding {
                    declarative,
                    call_site,
                },
            ));
    }

    /// Iterator over just the plain-call references. Most existing
    /// consumers only care about this subset (LSP call resolution,
    /// heuristic call resolution); offering a pre-filtered view keeps
    /// them free of per-variant match plumbing.
    pub fn iter_call_sites(&self) -> impl Iterator<Item = &CallSite> + '_ {
        self.references.iter().filter_map(|r| match r {
            UnresolvedReference::Call(cs) => Some(cs),
            _ => None,
        })
    }

    /// Iterator over framework bindings. Paired accessor to
    /// [`Self::iter_call_sites`]; the LSP / heuristic call resolvers
    /// consume both iterators rather than scanning the combined
    /// `references` vec with a match at every consumer.
    pub fn iter_framework_bindings(&self) -> impl Iterator<Item = &FrameworkBinding> + '_ {
        self.references.iter().filter_map(|r| match r {
            UnresolvedReference::FrameworkBinding(fb) => Some(fb),
            _ => None,
        })
    }

    /// Iterator over declarative bindings. Empty for every parse that
    /// runs before the first Flow reader lands; included now so
    /// resolvers exhaustively handle the variant at compile time.
    pub fn iter_declarative_bindings(&self) -> impl Iterator<Item = &DeclarativeBinding> + '_ {
        self.references.iter().filter_map(|r| match r {
            UnresolvedReference::DeclarativeBinding(db) => Some(db),
            _ => None,
        })
    }

    /// Total number of unresolved references (all variants). Used by
    /// pipeline telemetry that historically logged `call_sites.len()`
    /// as "how much raw material did extraction hand the resolver".
    pub fn total_references(&self) -> usize {
        self.references.len()
    }

    /// Iterator over the underlying `CallSite` of every reference,
    /// regardless of variant. Used by consumers that treat all
    /// references uniformly at the shape level (e.g. "which files did
    /// extraction touch?") and by fixtures/tests that were phrased
    /// against the pre-P1.d `call_sites: Vec<CallSite>` shape. Unlike
    /// [`Self::iter_call_sites`], this iterator does *not* filter to
    /// the `Call` variant.
    pub fn iter_all_call_sites(&self) -> impl Iterator<Item = &CallSite> + '_ {
        self.references.iter().map(|r| r.call_site())
    }

    /// Add a symbol to the results
    pub fn add_symbol(&mut self, symbol: Node) {
        self.symbols.push(symbol);
    }

    /// Add an import for resolution
    pub fn add_import(&mut self, location: Range) {
        self.imports.push(location);
    }

    /// Add a type reference for resolution (legacy - just range)
    pub fn add_type_ref(&mut self, location: Range) {
        self.type_refs.push(location);
    }

    /// Add a structured type reference with name and usage context
    pub fn add_type_reference(
        &mut self,
        location: Range,
        type_name: String,
        usage_kind: TypeUsageKind,
    ) {
        self.type_references.push(TypeReference {
            location,
            type_name,
            usage_kind,
        });
    }

    /// Add a structured import specification
    pub fn add_import_spec(&mut self, spec: ImportSpec) {
        self.import_specs.push(spec);
    }

    /// Add a module declaration
    pub fn add_mod_decl(&mut self, decl: ModDecl) {
        self.mod_decls.push(decl);
    }

    /// Check if results are empty
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
            && self.source_files.is_empty()
            && self.identifier_uses.is_empty()
            && self.references.is_empty()
            && self.imports.is_empty()
            && self.type_refs.is_empty()
            && self.type_references.is_empty()
            && self.import_specs.is_empty()
            && self.mod_decls.is_empty()
            && self.synthesized_edges.is_empty()
            && self.class_symbols.is_empty()
            && self.local_var_scopes.is_empty()
    }
}

/// Identifier usage information for variable references
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IdentifierUse {
    pub location: Range,
    pub name: String,
}

/// A single local variable or formal parameter with a declared type,
/// visible inside one method / constructor body.
///
/// TR-A.3 consumes this to resolve `someLocal.method(...)` by
/// declared type without re-parsing the method body. The name is
/// kept in its source-original case (Apex identifiers are
/// case-insensitive; resolver lookups normalise at compare-time).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LocalVarDecl {
    /// Declared name, exactly as it appears in source.
    pub name: String,
    /// Declared type. Produced by the language extractor's type
    /// classification (the same path that populates
    /// `ApexField.ty` / `ApexParameter.ty`) so local / field /
    /// parameter types collapse to the same [`ApexTypeRef`] variant
    /// space at resolver time.
    pub ty: ApexTypeRef,
    /// Range of the variable declaration itself (the `SomeType x`
    /// token pair). Kept for diagnostics and for future use by a
    /// stricter scope-end determination; the current resolver treats
    /// declarations as method-wide because Apex has no block-scoped
    /// `let` redeclaration semantics a heuristic resolver needs to
    /// honour.
    pub declared_at: Range,
}

/// Per-method / per-constructor local-variable scope. Emitted by
/// [`crate::syntax::language::LanguageSpecificExtractor::extract_local_var_scopes`]
/// and consumed by Apex's TR-A.3 field-type-aware dispatch resolver.
///
/// Scope is keyed by the enclosing method/constructor body's range;
/// the resolver picks the scope whose `body` contains the call-site
/// location. This mirrors how `SymbolIndex::find_enclosing_function`
/// already picks the innermost enclosing function node — keeping the
/// lookup shape identical avoids introducing a second notion of
/// "enclosing method" into the resolver.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LocalVarScope {
    /// Range of the method / constructor body (block node). The
    /// resolver binds a call site to this scope when
    /// `body.contains(call.location)`.
    pub body: Range,
    /// All locals declared inside the body, plus the method's formal
    /// parameters. Ordered in source order. Duplicate names are
    /// possible (block-scoped shadowing); TR-A.3 picks the **first
    /// match** whose declared-at precedes the call-site location,
    /// which matches Apex's declare-before-use shadowing convention.
    pub locals: Vec<LocalVarDecl>,
}

impl Default for SyntaxResults {
    fn default() -> Self {
        Self::new()
    }
}

/// Visibility qualifiers for Rust `use` declarations
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImportVisibility {
    Private,
    Pub,
    PubCrate,
    PubSuper(u8),
    PubIn(String),
}

impl ImportVisibility {
    pub fn is_public(&self) -> bool {
        !matches!(self, ImportVisibility::Private)
    }
}

/// Categorises use statements between normal imports and re-exports
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImportKind {
    Use,
    Reexport,
}

/// Root qualifier for an import path
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PathRoot {
    Crate,
    SelfPath,
    Super(u8),
    Absolute,
    ExternalCrate(String),
    Unqualified,
}

/// Parsed representation of a `use` path
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportPath {
    pub root: PathRoot,
    pub segments: Vec<String>,
}

impl ImportPath {
    pub fn new(root: PathRoot, segments: Vec<String>) -> Self {
        Self { root, segments }
    }
}

/// Structured representation of a `use` binding (post-expansion of nested trees)
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportSpec {
    pub range: Range,
    pub path: ImportPath,
    pub alias: Option<String>,
    pub visibility: ImportVisibility,
    pub kind: ImportKind,
    pub is_glob: bool,
    pub source_file: String,
}

/// Distinguishes inline modules from external file declarations
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModKind {
    Inline,
    External,
}

/// Parsed representation of a `mod` declaration
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModDecl {
    pub name: String,
    pub source_file: String,
    pub range: Range,
    pub kind: ModKind,
    pub resolved_file: Option<String>,
}

/// Results from semantic resolution phase
/// Contains resolved relationships between symbols
#[derive(Debug, Clone, Default)]
pub struct ResolutionStatsSummary {
    pub lsp_edges: usize,
    pub heuristic_edges: usize,
    pub lsp_failures: Vec<String>,
    pub heuristic_failures: Vec<String>,
    pub heuristic_call_fallbacks: usize,
    pub heuristic_import_fallbacks: usize,
    pub heuristic_type_fallbacks: usize,
    /// Number of call sites where the heuristic resolver found more
    /// candidates than its fanout cap allowed, and therefore emitted
    /// **zero** edges rather than a batch of arbitrary Low-confidence
    /// guesses. Surfaced as first-class telemetry so users can see how
    /// much call-graph signal they would recover by switching to LSP.
    pub heuristic_call_ambiguous_drops: usize,
}

impl ResolutionStatsSummary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn total_call_edges(&self) -> usize {
        self.lsp_edges + self.heuristic_edges
    }

    pub fn total_fallbacks(&self) -> usize {
        self.heuristic_call_fallbacks
            + self.heuristic_import_fallbacks
            + self.heuristic_type_fallbacks
    }
}

/// Port-layer snapshot of LSP session-lifecycle metrics.
///
/// Mirrors [`crate::infrastructure::lsp::session::SessionMetrics`] but
/// lives in the application layer so the pipeline orchestrator and
/// `ResolvedGraph` consumer (CLI, analysis) don't have to depend on
/// any concrete LSP implementation. The infra-layer `SessionMetrics`
/// can grow internal-only fields (timers, async state, etc.) without
/// leaking into the public port surface.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMetricsSnapshot {
    /// Total attempts the supervisor made to spawn the LSP process.
    pub start_attempts: u64,
    /// Attempts that transitioned to `Running`.
    pub successful_starts: u64,
    /// Attempts that ended in `Failed` or spawn error.
    pub failed_starts: u64,
    /// Most recent initialization / restart error message.
    pub last_error: Option<String>,
    /// Sprint F.1 — number of JSON-RPC notifications observed from
    /// the server since supervisor start. Zero after a successful
    /// `initialize` is a strong signal the server is silently broken
    /// (the classic jorje "started but never indexes" failure).
    pub notifications_received: u64,
    /// Sprint F.1 — number of stderr lines read from the LSP child
    /// process. Zero after start means the stderr pipe was never
    /// wired up; non-zero with no notifications usually means the
    /// server is emitting Java stack traces instead of LSP frames.
    pub stderr_lines_observed: u64,
    /// Sprint F.1 — subset of notifications classified as indexing
    /// signals (`$/progress`, `apex/*`, `window/logMessage` that
    /// mention "indexing"). Used by the F.2 readiness barrier as
    /// evidence the server has begun real work.
    pub indexing_messages_seen: u64,
}

impl SessionMetricsSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when at least one spawn attempt has landed the session
    /// in a usable state. Useful for asserting on telemetry without
    /// duplicating the "did LSP actually run?" logic across consumers.
    pub fn was_ever_running(&self) -> bool {
        self.successful_starts > 0
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedEdges {
    /// Resolved call relationships
    pub call_edges: Vec<Edge>,
    /// Resolved import relationships
    pub import_edges: Vec<Edge>,
    /// Resolved type relationships
    pub type_edges: Vec<Edge>,
    /// Resolved containment relationships
    pub containment_edges: Vec<Edge>,
    /// Summary statistics for call-resolution provenance
    pub stats: ResolutionStatsSummary,
    /// Set of `UnresolvedReference::Call` sites whose semantic resolver
    /// already produced a `Confidence::High` (or `Medium`) edge. The
    /// heuristic `FallbackEdgeBuilder` skips any reference whose
    /// `call_site.location` is in this set to avoid emitting a
    /// low-confidence sibling edge to a *different* name-match target
    /// once an authoritative target has already been resolved.
    ///
    /// Populated by the Rust Layer 2 adapter (T6 Gate 1.2) and by any
    /// future Layer 2 resolver that runs *ahead of* the heuristic
    /// fallback inside `SemanticResolverService::resolve_with_fallback`.
    /// The existing subprocess-LSP path leaves this set empty — its
    /// `ResolvedEdges` output is deduped by the existing
    /// `(caller_id, callee_id)` set instead, which is enough because
    /// the LSP path emits exactly one target per call site and
    /// heuristic name-matches to the same target already dedupe.
    pub resolved_call_sites: HashSet<Range>,
}

impl ResolvedEdges {
    /// Create empty resolved edges
    pub fn new() -> Self {
        Self {
            call_edges: Vec::new(),
            import_edges: Vec::new(),
            type_edges: Vec::new(),
            containment_edges: Vec::new(),
            stats: ResolutionStatsSummary::new(),
            resolved_call_sites: HashSet::new(),
        }
    }

    /// Record that an `UnresolvedReference::Call` at `location` has
    /// already been resolved by a Layer 2 semantic resolver. See the
    /// field-level docs on `ResolvedEdges::resolved_call_sites` for
    /// the contract the heuristic fallback enforces.
    pub fn mark_call_site_resolved(&mut self, location: Range) {
        self.resolved_call_sites.insert(location);
    }

    /// Add a call edge
    pub fn add_call_edge(&mut self, edge: Edge) {
        self.call_edges.push(edge);
    }

    /// Add an import edge
    pub fn add_import_edge(&mut self, edge: Edge) {
        self.import_edges.push(edge);
    }

    /// Add a type edge
    pub fn add_type_edge(&mut self, edge: Edge) {
        self.type_edges.push(edge);
    }

    /// Add a containment edge
    pub fn add_containment_edge(&mut self, edge: Edge) {
        self.containment_edges.push(edge);
    }

    /// Get all edges as a flat vector
    pub fn all_edges(&self) -> Vec<Edge> {
        let mut all = Vec::new();
        all.extend(self.call_edges.clone());
        all.extend(self.import_edges.clone());
        all.extend(self.type_edges.clone());
        all.extend(self.containment_edges.clone());
        all
    }

    /// Check if results are empty
    pub fn is_empty(&self) -> bool {
        self.call_edges.is_empty()
            && self.import_edges.is_empty()
            && self.type_edges.is_empty()
            && self.containment_edges.is_empty()
    }
}

impl Default for ResolvedEdges {
    fn default() -> Self {
        Self::new()
    }
}

/// Global symbol information for cross-file resolution
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub id: String,
    pub name: String,
    pub fqn: String,
    pub file: String,
    pub range: Range,
    pub kind: NodeKind,
    /// Trait metadata if this is a trait method
    pub trait_metadata: Option<crate::domain::TraitMetadata>,
}

/// Global symbol table for cross-file resolution
#[derive(Debug, Clone)]
pub struct GlobalSymbolTable {
    pub symbols_by_name: HashMap<String, Vec<SymbolInfo>>,
    pub symbols_by_file: HashMap<String, Vec<SymbolInfo>>,
    pub call_sites_by_file: HashMap<String, Vec<CallSite>>,
}

impl Default for GlobalSymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalSymbolTable {
    pub fn new() -> Self {
        Self {
            symbols_by_name: HashMap::new(),
            symbols_by_file: HashMap::new(),
            call_sites_by_file: HashMap::new(),
        }
    }
    pub fn add_symbol(&mut self, symbol: SymbolInfo) {
        let name = symbol.name.clone();
        let file = symbol.file.clone();
        self.symbols_by_name
            .entry(name)
            .or_default()
            .push(symbol.clone());
        self.symbols_by_file.entry(file).or_default().push(symbol);
    }
    pub fn add_call_site(&mut self, file: String, call_site: CallSite) {
        self.call_sites_by_file
            .entry(file)
            .or_default()
            .push(call_site);
    }
    pub fn find_symbols_by_name(&self, name: &str) -> Vec<&SymbolInfo> {
        self.symbols_by_name
            .get(name)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }
}

/// Port for syntax extraction (e.g., Tree-sitter)
/// Extracts symbols and identifies locations that need semantic resolution
#[async_trait]
pub trait SyntaxExtractor: Send + Sync {
    /// Extract syntax information from source files
    ///
    /// # Arguments
    /// * `files` - List of source files to analyze
    ///
    /// # Returns
    /// * `SyntaxResults` - Extracted symbols and resolution hints
    /// * `anyhow::Error` - If extraction fails
    async fn extract(&self, files: &[std::path::PathBuf]) -> anyhow::Result<SyntaxResults>;

    /// Get the language this extractor supports
    fn supported_language(&self) -> &str;

    /// Check if a file extension is supported
    fn supports_extension(&self, ext: &str) -> bool;

    /// Run language-specific post-syntax hooks after `extract` has
    /// populated `SyntaxResults`, before semantic resolution.
    ///
    /// See the T5 design document at
    /// `docs/workstreams/universal-fidelity/tasks/T5-orchestrator-collapse.md`.
    /// The parse-repo orchestrator calls this method through the
    /// `SyntaxExtractor` port so language dispatch stays inside the
    /// extractor implementation. A concrete tree-sitter-based
    /// extractor typically delegates to its
    /// [`crate::syntax::language::LanguageSpecificExtractor::post_syntax_hooks`].
    ///
    /// Default implementation is a no-op. Mock extractors and test
    /// doubles may inherit the default; only real pipeline-participating
    /// extractors need to override.
    fn post_syntax_hooks(
        &self,
        _workspace_root: &std::path::Path,
        _syntax_results: &mut SyntaxResults,
    ) -> crate::syntax::language::extractor::HookOutcome {
        crate::syntax::language::extractor::HookOutcome::NoOp
    }
}

/// Port for semantic resolution (e.g., LSP)
/// Resolves calls, imports, and type references to concrete definitions
#[async_trait]
pub trait SemanticResolver: Send + Sync {
    /// Resolve semantic relationships from syntax hints
    ///
    /// # Arguments
    /// * `hints` - Syntax results containing resolution hints
    ///
    /// # Returns
    /// * `ResolvedEdges` - Resolved relationships between symbols
    /// * `anyhow::Error` - If resolution fails
    async fn resolve(&self, hints: &SyntaxResults) -> anyhow::Result<ResolvedEdges>;

    /// Get the language this resolver supports
    fn supported_language(&self) -> &str;

    /// Check if the resolver is available (e.g., LSP server running)
    async fn is_available(&self) -> bool;

    /// Snapshot of the underlying LSP session's lifecycle metrics, if
    /// the resolver is LSP-backed. Default implementation returns
    /// `None` for non-LSP resolvers (mocks, pure-heuristic, etc.) so
    /// no existing implementation needs to change.
    ///
    /// Called by the pipeline orchestrator at the end of a scan so the
    /// CLI (and downstream consumers via telemetry JSON) can answer
    /// "did the LSP actually come up, or did we silently fall back to
    /// heuristics?" — which is one of the top failure modes the
    /// desktop pilot needs to detect.
    async fn session_metrics(&self) -> Option<SessionMetricsSnapshot> {
        None
    }
}

/// Port for graph persistence
/// Stores and retrieves parsed graphs
#[async_trait]
pub trait GraphRepository: Send + Sync {
    /// Store a graph in the repository
    ///
    /// # Arguments
    /// * `graph` - The graph to store
    ///
    /// # Returns
    /// * `()` - On success
    /// * `anyhow::Error` - If storage fails
    async fn upsert(&self, graph: &Graph) -> anyhow::Result<()>;

    /// Retrieve a graph by identifier
    ///
    /// # Arguments
    /// * `id` - Graph identifier
    ///
    /// # Returns
    /// * `Option<Graph>` - The graph if found
    /// * `anyhow::Error` - If retrieval fails
    async fn get(&self, id: &str) -> anyhow::Result<Option<Graph>>;

    /// List all stored graphs
    ///
    /// # Returns
    /// * `Vec<String>` - List of graph identifiers
    /// * `anyhow::Error` - If listing fails
    async fn list(&self) -> anyhow::Result<Vec<String>>;

    /// Delete a graph by identifier
    ///
    /// # Arguments
    /// * `id` - Graph identifier to delete
    ///
    /// # Returns
    /// * `()` - On success
    /// * `anyhow::Error` - If deletion fails
    async fn delete(&self, id: &str) -> anyhow::Result<()>;

    /// Clear all data from the repository
    ///
    /// # Returns
    /// * `()` - On success
    /// * `anyhow::Error` - If clearing fails
    async fn clear(&self) -> anyhow::Result<()>;

    /// Persist per-language class-symbols payloads emitted by
    /// [`SyntaxResults::class_symbols`] during the syntax pass.
    ///
    /// Default implementation is a no-op so non-SQLite backends (mocks,
    /// in-memory dev fixtures, future alternatives) can ignore the
    /// payload entirely. The SQLite backend persists into the
    /// `apex_class_symbols` table (TR-A.0). The payload is
    /// language-opaque: a list of `(dotted_api_name, symbols_json)`
    /// tuples whose shape is owned by the producing language's
    /// extractor.
    ///
    /// Empty input is valid and must be handled as a no-op.
    async fn upsert_apex_class_symbols(&self, _symbols: &[(String, String)]) -> anyhow::Result<()> {
        Ok(())
    }

    /// Persist per-file extraction-coverage records (T8 — universal
    /// fidelity sprint). Default impl is a no-op so non-SQLite
    /// backends (tests, in-memory dev) compile unchanged; the
    /// SQLite trampoline in `sqlite_repository.rs` dispatches to
    /// `upsert_file_extraction_coverage_sync` for the real write.
    ///
    /// Empty input is valid and must be handled as a no-op.
    async fn upsert_file_extraction_coverage(
        &self,
        _records: &[FileExtractionCoverage],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Read every cached `file_cache` row for the S1 incremental scan
    /// planner. Default impl returns an empty map so backends without
    /// a cache table (mocks, in-memory dev) behave as if every scan
    /// were cold. The SQLite backend dispatches to
    /// `FileCacheRepository::load_all`.
    async fn read_file_cache(
        &self,
    ) -> anyhow::Result<
        std::collections::BTreeMap<String, crate::infrastructure::storage::FileCacheRow>,
    > {
        Ok(std::collections::BTreeMap::new())
    }

    /// Upsert a batch of `file_cache` rows after a successful scan
    /// (S1). Default impl is a no-op so non-SQLite backends compile
    /// unchanged. Empty input must be handled as a no-op.
    async fn upsert_file_cache(
        &self,
        _rows: &[crate::infrastructure::storage::FileCacheRow],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Drop cache rows owned by `language` whose `file_path` is not
    /// in `current_paths` (S1 end-of-scan prune). The language scope
    /// is essential when the parse DB is persistent (S1-ε): the
    /// `file_cache` table is shared across language passes, and an
    /// unscoped prune would let the Rust pass delete Python cache
    /// rows simply because Rust's discovery didn't see any `.py`
    /// files. Default impl is a no-op so non-SQLite backends
    /// compile unchanged. Returns the number of rows removed for
    /// telemetry / progress emission.
    async fn prune_file_cache_missing(
        &self,
        _language: &str,
        _current_paths: &std::collections::HashSet<String>,
    ) -> anyhow::Result<usize> {
        Ok(0)
    }

    /// Delete all rows in `nodes`, `edges` (via cascade), and
    /// `file_extraction_coverage` whose source file appears in
    /// `file_paths`. Called by the S1-ε orchestrator before
    /// re-extracting changed-or-removed files in the persistent
    /// parse-DB world, so the next UPSERT pass doesn't leave stale
    /// rows behind. Returns the number of `nodes` rows deleted for
    /// telemetry.
    ///
    /// Default impl is a no-op so non-SQLite backends compile
    /// unchanged. Empty input must be a no-op. Implementations must
    /// batch the deletion in a single transaction to keep the
    /// orchestrator's incremental-step latency low.
    ///
    /// Note: `apex_class_symbols` is not pruned here today because
    /// its primary key is the Apex `api_name`, not a file path. A
    /// deleted Apex file leaves a stale row in that table until the
    /// next full scan; this is a documented S1-ε limitation
    /// (see `docs/02-strategy/V0_1_0_RC1_FOLLOWUP_ISSUES.md`).
    async fn prune_files_from_graph(&self, _file_paths: &[String]) -> anyhow::Result<usize> {
        Ok(0)
    }

    /// Persist incremental scan statistics for S2 analysis fast-path
    /// (`parse_meta.incremental_scan_stats`). Default impl is a no-op.
    async fn write_incremental_scan_stats(
        &self,
        _stats: &crate::infrastructure::storage::parse_meta_store::IncrementalScanStats,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Edge, Node, Provenance, Range};

    #[test]
    fn test_syntax_results_creation() {
        let mut results = SyntaxResults::new();
        assert!(results.is_empty());

        let node = Node::function(
            "test::func".to_string(),
            Range::with_file(1, 0, 5, 10, "test.rs".to_string()),
        );
        results.add_symbol(node.clone());
        assert!(!results.is_empty());
        assert_eq!(results.symbols.len(), 1);

        let range = Range::with_file(10, 5, 15, 20, "test.rs".to_string());
        results.add_call_site(range.clone(), "mock_function".to_string());
        results.add_import(range.clone());
        results.add_type_ref(range);

        assert_eq!(results.references.len(), 1);
        assert_eq!(results.imports.len(), 1);
        assert_eq!(results.type_refs.len(), 1);
    }

    #[test]
    fn test_syntax_results_default() {
        let results = SyntaxResults::default();
        assert!(results.is_empty());
    }

    /// Compile-time exhaustiveness fixture for
    /// [`UnresolvedReference`]. This function uses a plain `match`
    /// without a `_` arm, so deleting / renaming any variant makes
    /// the workspace fail to build. That is the dissolving-element
    /// for Decision 3 in the P1.d rework: the call resolver cannot
    /// silently drop a variant the way the old `edge_kind_hint`
    /// field invited it to.
    ///
    /// If you are here because the compiler pointed you at this
    /// function, do NOT add a `_` arm. Add an arm that names the new
    /// variant, then mirror the update in every resolver that
    /// dispatches on `UnresolvedReference` (grep `match ` on the
    /// type). The documented exhaustiveness check at
    /// `docs/workstreams/universal-fidelity/tasks/T1-rework.md` §4.4
    /// relies on this property.
    #[allow(dead_code)]
    fn _exhaustive_dispatch_over_unresolved_reference(r: &UnresolvedReference) -> &'static str {
        match r {
            UnresolvedReference::Call(_) => "call",
            UnresolvedReference::FrameworkBinding(_) => "framework",
            UnresolvedReference::DeclarativeBinding(_) => "declarative",
        }
    }

    #[test]
    fn framework_binding_edge_kind_reflects_variant() {
        use crate::domain::{EdgeKind, FrameworkKind};
        let cs = CallSite {
            location: Range::with_file(1, 0, 1, 10, "p.page".to_string()),
            function_name: "Ctrl::save".to_string(),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        };
        let fb = UnresolvedReference::FrameworkBinding(FrameworkBinding {
            framework: FrameworkKind::VisualforcePage,
            call_site: cs,
        });
        assert_eq!(
            fb.edge_kind(),
            EdgeKind::Framework(FrameworkKind::VisualforcePage),
        );
    }

    #[test]
    fn test_resolved_edges_creation() {
        let mut edges = ResolvedEdges::new();
        assert!(edges.is_empty());

        let edge = Edge::call(
            "func1".to_string(),
            "func2".to_string(),
            Provenance::tree_sitter(),
        );
        edges.add_call_edge(edge.clone());
        assert!(!edges.is_empty());
        assert_eq!(edges.call_edges.len(), 1);

        let import_edge = Edge::import("file1".to_string(), "file2".to_string(), Provenance::lsp());
        edges.add_import_edge(import_edge.clone());

        let type_edge = Edge::type_("struct".to_string(), "trait".to_string(), Provenance::lsp());
        edges.add_type_edge(type_edge.clone());

        let contains_edge = Edge::contains(
            "module".to_string(),
            "func".to_string(),
            Provenance::tree_sitter(),
        );
        edges.add_containment_edge(contains_edge);

        let all_edges = edges.all_edges();
        assert_eq!(all_edges.len(), 4);
    }

    #[test]
    fn test_resolved_edges_default() {
        let edges = ResolvedEdges::default();
        assert!(edges.is_empty());
    }

    // --- T8: CoverageGap predicate contract ---
    //
    // These tests lock the predicate contract enumerated in T8 §4.3
    // "Predicate contracts". The invariant is *every* variant answers
    // every predicate without panicking and without falling through
    // to a default branch. Adding a new variant without updating the
    // match arms will fail to compile because `matches!` must see the
    // new variant; adding it but forgetting the test here is caught
    // by `coverage_gap_predicates_exhaustive` below.

    fn all_gap_variants() -> Vec<CoverageGap> {
        vec![
            CoverageGap::ApexPropertyAccessor { count: 1 },
            CoverageGap::ApexMapLiteralInitializer { count: 1 },
            CoverageGap::ApexTriggerBodyUncaptured { count: 1 },
        ]
    }

    #[test]
    fn coverage_gap_predicates_exhaustive() {
        for gap in all_gap_variants() {
            // Exercising every predicate proves each variant has a
            // non-panicking answer. This is the "no default branch"
            // safety net named in T8 §6.1.
            let _ = gap.invalidates_no_callers();
            let _ = gap.invalidates_cycles();
            let _ = gap.invalidates_fan_in_metrics();
            let _ = gap.invalidates_coupling();
            let _ = gap.discriminator();
            let _ = gap.count();
        }
    }

    #[test]
    fn coverage_gap_no_callers_is_true_for_extraction_gaps() {
        for gap in all_gap_variants() {
            assert!(
                gap.invalidates_no_callers(),
                "{:?} should invalidate no_callers",
                gap,
            );
        }
    }

    #[test]
    fn coverage_gap_cycles_is_false_for_all_variants() {
        for gap in all_gap_variants() {
            assert!(
                !gap.invalidates_cycles(),
                "{:?} must not invalidate cycles (missing edges cannot forge cycles)",
                gap,
            );
        }
    }

    #[test]
    fn coverage_gap_count_preserves_input() {
        assert_eq!(CoverageGap::ApexPropertyAccessor { count: 7 }.count(), 7);
        assert_eq!(
            CoverageGap::ApexMapLiteralInitializer { count: 0 }.count(),
            0
        );
        assert_eq!(
            CoverageGap::ApexTriggerBodyUncaptured { count: 42 }.count(),
            42
        );
    }

    #[test]
    fn coverage_gap_discriminators_are_stable_and_distinct() {
        let mut discriminators = std::collections::HashSet::new();
        for gap in all_gap_variants() {
            let d = gap.discriminator();
            // Caveat strings in the health report rely on these being
            // stable, so the test locks the current wire values.
            assert!(
                !d.is_empty(),
                "discriminator must be non-empty for {:?}",
                gap,
            );
            assert!(
                discriminators.insert(d),
                "duplicate discriminator {d} for {:?}",
                gap,
            );
        }
        // Pin the current strings so drift is a review-visible change.
        assert_eq!(
            CoverageGap::ApexPropertyAccessor { count: 1 }.discriminator(),
            "apex_property_accessor",
        );
        assert_eq!(
            CoverageGap::ApexMapLiteralInitializer { count: 1 }.discriminator(),
            "apex_map_literal_initializer",
        );
        assert_eq!(
            CoverageGap::ApexTriggerBodyUncaptured { count: 1 }.discriminator(),
            "apex_trigger_body_uncaptured",
        );
    }

    #[test]
    fn coverage_gap_serde_roundtrips_with_tagged_wire_format() {
        // T8 §4.3 "Compile-time guarantees" requires the wire format
        // to be a first-class contract so future serde-derive tweaks
        // trip this test rather than silently drifting.
        let gap = CoverageGap::ApexPropertyAccessor { count: 3 };
        let wire = serde_json::to_string(&gap).expect("serialize");
        assert_eq!(wire, r#"{"kind":"ApexPropertyAccessor","sub":{"count":3}}"#);
        let parsed: CoverageGap = serde_json::from_str(&wire).expect("deserialize");
        assert_eq!(parsed, gap);
    }

    #[test]
    fn file_extraction_coverage_has_invalidating_gap_mirrors_any_predicate() {
        let cov_with_gap = FileExtractionCoverage {
            file_path: std::path::PathBuf::from("/fake/A.cls"),
            language: "apex".to_string(),
            walked_node_count: 100,
            unwalked_node_count: 3,
            coverage_gaps: vec![CoverageGap::ApexPropertyAccessor { count: 3 }],
            confidence: CoverageConfidence::High,
        };
        assert!(cov_with_gap.has_invalidating_no_callers_gap());
        assert_eq!(cov_with_gap.total_gap_count(), 3);

        let cov_without_gap = FileExtractionCoverage {
            file_path: std::path::PathBuf::from("/fake/B.cls"),
            language: "apex".to_string(),
            walked_node_count: 100,
            unwalked_node_count: 0,
            coverage_gaps: Vec::new(),
            confidence: CoverageConfidence::High,
        };
        assert!(!cov_without_gap.has_invalidating_no_callers_gap());
        assert_eq!(cov_without_gap.total_gap_count(), 0);
    }
}
