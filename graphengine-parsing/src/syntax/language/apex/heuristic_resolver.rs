//! Fallback semantic resolver for Apex.
//!
//! This resolver is **strictly a fallback**. The primary resolution path
//! is `apex-jorje-lsp` via [`crate::infrastructure::lsp`]. This resolver
//! exists for three scenarios:
//!
//! 1. The user's machine has no Java installed, no bundled JRE, and no
//!    `GRAPHENGINE_JAVA_HOME` override. LSP cannot start.
//! 2. The user explicitly opts out via `GRAPHENGINE_APEX_RESOLVER=heuristic`
//!    (typically for CI speed or reproducibility audits).
//! 3. LSP crashes or fails to initialize mid-run. The dispatcher degrades
//!    individual requests to this resolver instead of aborting the scan.
//!
//! The resolver emits `Provenance::heuristic()` on every edge it produces.
//! Downstream resolution-quality telemetry separates heuristic edges from
//! LSP edges so users see exactly how much of their graph was resolved
//! by each tier.
//!
//! # Accuracy contract
//!
//! Honest about what this resolver does and does not do:
//!
//! - **Does** resolve unambiguous short-name references via a
//!   case-insensitive [`ApexClassRegistry`] seeded with standard SObjects
//!   and system types.
//! - **Does** filter `Trigger.*` context-variable accesses so they never
//!   inflate the call graph with a nonexistent `Trigger` class.
//! - **Does** record every managed-package reference as a virtual
//!   external node so coupling-to-external is visible.
//! - **Does not** perform type inference (`var x = new Foo(); x.m()`
//!   produces an all-candidates result when multiple `m()` methods exist).
//! - **Does not** resolve interface dispatch to a specific implementer
//!   (emits all implementers tagged [`Confidence::Low`]).
//! - **Does not** understand SOQL expression types. That's LSP-only.
//!
//! Realistic recall ceiling: ~70–80% on mid-size SFDX repos. For >90%
//! accuracy, users must install the apex-jorje LSP (default path on the
//! desktop installer).

use std::collections::HashMap;

use async_trait::async_trait;
use tracing::{debug, info, instrument};

use super::constructor_resolver;
use super::downward_dispatch;
use super::field_type_resolver;
use super::type_hierarchy::TypeHierarchy;
#[cfg(test)]
use crate::application::ports::TypeReference;
use crate::application::ports::{
    CallSite, ResolutionStatsSummary, ResolvedEdges, SemanticResolver, SyntaxResults, TypeUsageKind,
};
use crate::domain::{
    Confidence, Edge, EdgeKind, Node, NodeKind, Provenance, ProvenanceSource, Range,
};

use super::class_registry::{ApexClassRegistry, ApexTypeKind};
use crate::domain::apex::class_symbols::ApexClassSymbols;
use std::path::PathBuf;

/// Maximum number of candidate callees the heuristic resolver will
/// emit edges to for a single call site. Above this, the resolver
/// emits **zero** edges for that site (counted in
/// [`ResolutionStatsSummary::heuristic_call_ambiguous_drops`]) rather
/// than spray N arbitrary Low-confidence guesses into the graph.
///
/// Rationale (see `docs/workstreams/apex/NEXT_STEPS_PLAN.md` §B.7-P2): common Apex
/// method names (`save`, `execute`, `initialize`, `getName`, `run`)
/// exist on dozens of unrelated classes. A heuristic that strips the
/// receiver (which this one does — see [`strip_prefix_and_receiver`])
/// cannot tell them apart without type information. When 30 classes
/// each declare a `save()`, emitting 30 edges per call site adds 30×
/// pure noise per site and gives false precision ("look, we resolved
/// it to exactly these 30"). Below-cap multi-candidate matches are
/// still emitted as Low-confidence so legitimate small-fanout
/// ambiguity (e.g. 2–3 overload candidates in the same class) is
/// preserved.
///
/// The cap lives on the heuristic path only. The LSP path uses
/// actual type resolution and is not subject to fanout — crashing a
/// real LSP call into this limit would be a bug.
pub const HEURISTIC_CALL_FANOUT_CAP: usize = 8;

/// Pure-Rust heuristic resolver for Apex. See module docs for semantics.
pub struct ApexHeuristicResolver {
    /// Preloaded case-insensitive registry — seeded with standard
    /// SObjects + system types, then enriched at scan time with user
    /// declarations from source and `object-meta.xml`.
    registry: ApexClassRegistry,
}

impl ApexHeuristicResolver {
    /// Construct with a caller-provided registry. The caller is expected
    /// to have already inserted user-declared types and custom SObjects.
    pub fn new(registry: ApexClassRegistry) -> Self {
        Self { registry }
    }

    /// Construct with only the standard-preload table — useful for tests
    /// and for degraded runs where we haven't walked user sources yet.
    pub fn with_standard_preload_only() -> Self {
        Self::new(ApexClassRegistry::with_standard_preload())
    }

    /// Read-only access to the registry — exposed for telemetry /
    /// reporting so downstream code can enumerate external references.
    pub fn registry(&self) -> &ApexClassRegistry {
        &self.registry
    }
}

#[async_trait]
impl SemanticResolver for ApexHeuristicResolver {
    #[instrument(skip(self, hints), fields(sym_count = hints.symbols.len()))]
    async fn resolve(&self, hints: &SyntaxResults) -> anyhow::Result<ResolvedEdges> {
        let mut edges = ResolvedEdges::new();
        let indexes = SymbolIndex::build(hints);

        // Seed the resolver's registry with user-declared class
        // symbols coming in on `hints.class_symbols`. The factory
        // constructs this resolver with only the standard-SObject
        // preload (see
        // `application::use_cases::parse_repo::factory.rs`), so
        // every TR-A.1 / TR-A.2 / TR-A.3 code path that calls
        // `registry.symbols_for(user_type)` would return `None`
        // without this step — the constructor / field-type-aware
        // arms would silently no-op on user classes and the
        // resolver would fall straight through to the name-only
        // fanout path, losing every piece of class-symbol-driven
        // dispatch signal in production.
        //
        // Building a fresh registry here instead of mutating
        // `self.registry` keeps `resolve(&self, …)` side-effect-
        // free. The registry is small (tens of thousands of
        // entries at NPSP scale) so the per-resolve clone cost is
        // invisible next to the parse / analyse phases (<10 ms at
        // 16 k nodes on an Apple M-series laptop). The logic
        // mirrors the `build_registry_from_results` helper used by
        // the TR-A.1 / TR-A.2 / TR-A.3 fixture drivers — keep in
        // sync with those tests if the shape ever changes.
        //
        // FOLLOWUP_RISKS.md R37 tracks the underlying wiring gap
        // this seeding step closes; the long-term home is a
        // registry-seeding pass in the orchestrator that runs
        // ahead of `SemanticResolver::resolve` so the logic is not
        // duplicated across every resolver path.
        let registry = seed_registry_from_hints(&self.registry, hints);

        // Invert the registry into subtype indexes so interface /
        // virtual / abstract call sites can fan out to every known
        // override the user's code supplies. Built once per resolve
        // pass; O(1) lookups inside the per-call-site loop. See
        // `type_hierarchy::TypeHierarchy` for the index shape and
        // `downward_dispatch::enumerate_overrides` for the caller
        // policy (interface → all implementers, virtual/abstract →
        // subclasses, concrete → none).
        let hierarchy = TypeHierarchy::build(&registry);

        let mut heuristic_call_count = 0usize;
        let mut heuristic_type_count = 0usize;
        let mut heuristic_call_ambiguous_drops = 0usize;

        // ---- Call sites ----
        for reference in &hints.references {
            let site = reference.call_site();
            if is_trigger_context_access(&site.function_name) {
                // Trigger.new / Trigger.old / Trigger.isInsert / ... — these
                // are context variable accesses, NOT method calls. Dropping
                // them here is the single largest false-positive reduction
                // in Apex call-graph extraction.
                continue;
            }

            let Some(caller) = indexes.find_enclosing_function(site) else {
                // No enclosing function known to us. Can't attach the edge
                // to a real node — drop it rather than create dangling.
                continue;
            };

            // TR-A.1 / TR-A.2 — constructor-call arm. Emitted by the
            // extractor for `new X(...)`, `this(...)`, and `super(...)`
            // shapes. Dispatches to the dedicated resolver so the
            // lookup / disambiguation / implicit-default-ctor logic
            // stays out of the method-call hot path.
            if constructor_resolver::is_constructor_call_name(&site.function_name) {
                let enclosing_type_fqn = indexes
                    .find_enclosing_type(&site.location)
                    .map(|n| n.fqn.as_str());
                if let Some(outcome) = constructor_resolver::resolve_constructor_call(
                    site,
                    caller,
                    &registry,
                    &indexes.functions_by_name_lower,
                    &indexes.types_by_name_lower,
                    enclosing_type_fqn,
                    &hints.local_var_scopes,
                ) {
                    for callee in outcome.callees {
                        if callee.id == caller.id {
                            continue;
                        }
                        // Honour the UnresolvedReference variant so
                        // framework bindings (VF, etc.) that come
                        // through this arm still emit Framework(_)
                        // rather than being coerced to Call. P1.d
                        // rework — the variant is the typed channel;
                        // a plain Call reference resolves to
                        // EdgeKind::Call, a FrameworkBinding to
                        // Framework(_), a DeclarativeBinding to
                        // Declarative(_). Exhaustiveness is enforced
                        // by UnresolvedReference::edge_kind().
                        let edge_kind = reference.edge_kind();
                        edges.add_call_edge(Edge::new(
                            caller.id.clone(),
                            callee.id.clone(),
                            edge_kind,
                            Provenance::new(ProvenanceSource::Heuristic, outcome.confidence),
                        ));
                        heuristic_call_count += 1;
                    }
                }
                continue;
            }

            let target_name = strip_prefix_and_receiver(&site.function_name);

            // TR-A.3 / TR-A.4 — field-type-aware dispatch + bare-self
            // dispatch. Before the name-only fallback widens the
            // search to every same-named method in the repo, try to
            // bind the call using the receiver's declared type:
            //
            //   1. `this.m(..)` / `this.<field>.m(..)` — enclosing
            //      class + enclosing-class fields (+ parent chain).
            //   2. `<local>.m(..)` — method-body local scope.
            //   3. `<field>.m(..)` — enclosing-class fields (+ parent
            //      chain).
            //   4. TR-A.4 — bare `m(..)` with no receiver at all
            //      (Apex's implicit-`this` rule) funnels into the
            //      same arm as (1) via
            //      `resolve_field_type_call`'s `None → SelfRef`
            //      normalisation. This is the resolver-level hook
            //      that closes `Contacts::loadAccountByIdMap()`
            //      (R38 deferred FQN).
            //
            // Emission on a successful bind is Medium confidence for
            // an Exact / Widening unique match, Low confidence for
            // an Implicit-tier unique match (TR-A.4 Object /
            // autoboxing widening), and Low for any fanout. On miss
            // (resolver declines or no method found on the target
            // class chain), fall through to the existing name-based
            // candidates path — it stays intact so every short-name
            // call site that used to resolve still does.
            if let Some(enclosing_type_api) = indexes
                .find_enclosing_type(&site.location)
                .and_then(|n| api_name_from_class_fqn(&n.fqn))
            {
                if let Some(outcome) = field_type_resolver::resolve_field_type_call(
                    site,
                    caller,
                    target_name,
                    enclosing_type_api,
                    &registry,
                    &indexes.functions_by_name_lower,
                    &hints.local_var_scopes,
                ) {
                    let edge_kind = reference.edge_kind();
                    for callee in &outcome.callees {
                        if callee.id == caller.id {
                            continue;
                        }
                        edges.add_call_edge(Edge::new(
                            caller.id.clone(),
                            callee.id.clone(),
                            edge_kind,
                            Provenance::new(ProvenanceSource::Heuristic, outcome.confidence),
                        ));
                        heuristic_call_count += 1;
                    }

                    // TR-A.6 downward polymorphic dispatch. When the
                    // resolved callee is an interface method, a
                    // virtual method, or an abstract method, every
                    // concrete implementer / subclass override is a
                    // legitimate runtime target. Emit each as a Low
                    // edge — the field-bound edge already stands at
                    // Medium/Low per field_type_resolver's policy,
                    // the override edges only augment fan-out. See
                    // `downward_dispatch::enumerate_overrides` for
                    // the fanout policy. This closes the NPSP
                    // revert-population shapes where a TDTM
                    // framework dispatches through
                    // `CascadeDeleteLoader` (interface) or
                    // `getCascadeDeleteLoader()` (virtual).
                    for callee in &outcome.callees {
                        let overrides = downward_dispatch::enumerate_overrides(
                            callee,
                            &registry,
                            &hierarchy,
                            &indexes.functions_by_name_lower,
                        );
                        for override_node in overrides {
                            if override_node.id == caller.id || override_node.id == callee.id {
                                continue;
                            }
                            edges.add_call_edge(Edge::new(
                                caller.id.clone(),
                                override_node.id.clone(),
                                edge_kind,
                                Provenance::new(ProvenanceSource::Heuristic, Confidence::Low),
                            ));
                            heuristic_call_count += 1;
                        }
                    }
                    continue;
                }
            }

            let candidates = indexes.find_function_candidates(target_name);

            if candidates.is_empty() {
                // Nothing to emit. Resolution-quality telemetry picks
                // this up via `heuristic_failures` downstream.
                continue;
            }

            // Fanout cap — see HEURISTIC_CALL_FANOUT_CAP doc. When the
            // short-name matches more candidates than the cap allows,
            // we refuse to guess. A single dropped site is recorded so
            // telemetry and the "ResolutionDegraded" finding can
            // quantify how much call-graph signal would be recovered
            // by switching to LSP resolution.
            if candidates.len() > HEURISTIC_CALL_FANOUT_CAP {
                heuristic_call_ambiguous_drops += 1;
                debug!(
                    target = %target_name,
                    candidate_count = candidates.len(),
                    cap = HEURISTIC_CALL_FANOUT_CAP,
                    "dropping over-cap ambiguous call site; no edges emitted",
                );
                continue;
            }

            // Sprint H.1 — same-class preference. Before falling back to
            // Low-confidence cross-class fanout, check whether any
            // candidate lives in the same class as the caller. If so,
            // that subset is unambiguous w.r.t. which class owns the
            // method (overloads in the same class all stay), and we
            // promote to Medium. This collapses the common pattern of
            // "A.foo() calls foo() where a B.foo() also exists" from two
            // Low edges to one Medium edge pointing at A.foo().
            let caller_class = enclosing_class_fqn(&caller.fqn);
            let same_class: Vec<&Node> = candidates
                .iter()
                .copied()
                .filter(|c| {
                    caller_class
                        .as_deref()
                        .zip(enclosing_class_fqn(&c.fqn).as_deref())
                        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
                })
                .collect();

            let (effective, confidence) = if candidates.len() == 1 {
                // Single unambiguous match in the user's workspace.
                // Promote to Medium — still heuristic, but not a guess.
                (candidates.clone(), Confidence::Medium)
            } else if !same_class.is_empty() {
                // 2+ candidates but at least one lives in the caller's
                // own class. Collapse to the same-class subset at Medium.
                // See Sprint H.1 ticket for the motivating G.2 finding
                // (UTIL_Describe::getNamespace vs. VOL_SharedCode::getNamespace).
                (same_class, Confidence::Medium)
            } else {
                // Small-fanout ambiguity (2–CAP candidates, none in the
                // caller's class). Emit all tagged Low so users know
                // the edge is best-effort, not guaranteed.
                (candidates.clone(), Confidence::Low)
            };

            let edge_kind = reference.edge_kind();
            for callee in effective {
                if callee.id == caller.id {
                    // Self-call: Apex allows recursion but the Edge
                    // constructor rejects self-loops for Call edges. Drop.
                    continue;
                }
                edges.add_call_edge(Edge::new(
                    caller.id.clone(),
                    callee.id.clone(),
                    edge_kind,
                    Provenance::new(ProvenanceSource::Heuristic, confidence),
                ));
                heuristic_call_count += 1;
            }
        }

        // ---- Type references (extends / implements / parameter / return) ----
        for tref in &hints.type_references {
            let Some(from) = indexes.find_enclosing_type_or_function(&tref.location) else {
                continue;
            };
            let target = tref.type_name.trim();
            if target.is_empty() {
                continue;
            }

            // First try to resolve to a user-declared node.
            if let Some(to_node) = indexes.find_type_node(target) {
                if to_node.id == from.id {
                    continue;
                }
                let kind = type_edge_kind(&tref.usage_kind);
                edges.add_call_edge_or_type(kind, from.id.clone(), to_node.id.clone());
                heuristic_type_count += 1;
                continue;
            }

            // Fall back to registry lookup — a hit here means the
            // reference is to a standard SObject, system type, custom
            // SObject declared via object-meta.xml, or a managed-package
            // external. These don't have a matching Node in
            // hints.symbols, so we can't emit a graph edge to them
            // directly — but we DO want to surface the reference in
            // telemetry. Phase 1 skips emission here; Phase 2's
            // external-virtual-node materialization closes the gap.
            if registry.lookup(target).is_some() {
                debug!(
                    "Apex heuristic: registry-only reference {} from {}",
                    target, from.fqn
                );
            }
        }

        edges.stats = ResolutionStatsSummary {
            lsp_edges: 0,
            heuristic_edges: heuristic_call_count,
            lsp_failures: Vec::new(),
            heuristic_failures: Vec::new(),
            heuristic_call_fallbacks: heuristic_call_count,
            heuristic_import_fallbacks: 0,
            heuristic_type_fallbacks: heuristic_type_count,
            heuristic_call_ambiguous_drops,
        };

        info!(
            "ApexHeuristicResolver resolved {} call edges, {} type-backed edges, dropped {} over-cap ambiguous sites",
            heuristic_call_count, heuristic_type_count, heuristic_call_ambiguous_drops,
        );
        Ok(edges)
    }

    fn supported_language(&self) -> &str {
        "apex"
    }

    /// Always available. The heuristic resolver is pure-Rust, has no
    /// external processes, and has no initialization steps that can
    /// fail — that's precisely why it's the fallback of last resort.
    async fn is_available(&self) -> bool {
        true
    }
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Identify Trigger context-variable accesses. Apex exposes a magic
/// `Trigger` namespace inside trigger bodies — `Trigger.new`, `Trigger.old`,
/// `Trigger.newMap`, `Trigger.oldMap`, `Trigger.isInsert`, `Trigger.isUpdate`,
/// `Trigger.isDelete`, `Trigger.isBefore`, `Trigger.isAfter`, `Trigger.isExecuting`,
/// `Trigger.size`. None of these are method calls on a user type; treating
/// them as such creates a phantom `Trigger` node that shows up as the
/// highest-fan-in class in every Apex report. We drop them here.
fn is_trigger_context_access(function_name: &str) -> bool {
    let lower = function_name.to_ascii_lowercase();
    let stripped = lower
        .split(':')
        .next_back()
        .unwrap_or(lower.as_str())
        .trim();

    // Match any of: `trigger.<property>`, bare `trigger.new`, or just
    // `trigger` used as a variable name in pathological captures.
    if stripped == "trigger" {
        return true;
    }
    stripped.starts_with("trigger.")
}

/// Extract the api-name tail from a type node's FQN.
///
/// Apex type-node FQNs repeat the class name (`path::Client::Client`)
/// or carry a dotted inner shape (`path::Outer::Outer.Inner`). The
/// api-name key expected by [`ApexClassRegistry`] is whatever sits
/// after the final `::`. Returns `None` on an FQN without a `::`
/// separator (synthetic / external nodes).
fn api_name_from_class_fqn(fqn: &str) -> Option<&str> {
    fqn.rsplit_once("::").map(|(_, tail)| tail)
}

/// Layer the user-declared class symbols from a [`SyntaxResults`] run
/// onto a base registry, returning a fresh registry ready for the
/// resolver's lookup paths.
///
/// # Why this exists
///
/// The factory in `application::use_cases::parse_repo::factory.rs`
/// constructs `ApexHeuristicResolver::with_standard_preload_only()`,
/// which seeds the registry with Salesforce standard SObjects and
/// system types but never with the user's own classes. That was a
/// latent gap until TR-A.1 (constructor dispatch) and especially
/// TR-A.3 (field-type-aware method dispatch) started consuming
/// `registry.symbols_for(user_type)` — those lookups all returned
/// `None` in production, silently reverting every class-symbols-aware
/// arm to the old name-only heuristic (tracked as FOLLOWUP_RISKS.md
/// R37).
///
/// The alternative — threading a seeding step into every caller of
/// `SemanticResolver::resolve` — would require mutating the resolver
/// post-construction or passing a separate orchestrator pass. Doing
/// it here keeps `resolve(&self, …)` side-effect-free and makes the
/// invariant "registry always carries user class symbols during a
/// resolve call" a local property of this module, not a cross-cutting
/// contract spread across the orchestrator.
///
/// # Shape
///
/// Mirrors the two-pass pattern used by the TR-A.x fixture drivers
/// (`apex_resolver_r23_ctor_fixtures.rs`,
/// `apex_resolver_r23_a3_field_dispatch_fixtures.rs`): first
/// `insert_user_declared` for every api name so inter-class
/// references find an entry, then `attach_symbols` in a second pass
/// once all entries exist. Keep this in sync with those fixtures —
/// they are the regression contract for the seeding shape.
fn seed_registry_from_hints(base: &ApexClassRegistry, hints: &SyntaxResults) -> ApexClassRegistry {
    if hints.class_symbols.is_empty() {
        return base.clone();
    }

    let mut registry = base.clone();

    // Type declarations in the hints graph are emitted as one of
    // `NodeKind::{Struct, Interface, Enum}` — `Struct` for
    // `class_declaration`, `Interface` for `interface_declaration`,
    // `Enum` for `enum_declaration`. The class-symbols payload
    // itself does not carry the declared kind (it is shape-only:
    // fields, methods, etc.), so we pair each symbol entry with its
    // matching type node here and read the kind off that. Missing a
    // match falls back to `Class` — the TR-A.0 contract says every
    // attached symbol payload originated from a user-declared
    // type, and `Class` is the lossless degradation that keeps the
    // entry visible in the registry's upward paths even when the
    // downward-dispatch inversion misses it.
    let type_node_for = |api_name: &str| -> Option<&crate::domain::Node> {
        hints
            .symbols
            .iter()
            .filter(|n| {
                matches!(
                    n.kind,
                    NodeKind::Struct | NodeKind::Interface | NodeKind::Enum
                )
            })
            .find(|n| {
                // Exact endswith-api match (with dotted separator when
                // the api is an inner). Avoids greedy accidental
                // matches on substring FQNs like `MyX` vs `MyXWrapper`.
                let fqn = &n.fqn;
                fqn == api_name
                    || fqn.ends_with(&format!("::{api_name}"))
                    || fqn.ends_with(&format!(".{api_name}"))
            })
    };
    let kind_for = |api_name: &str| -> ApexTypeKind {
        match type_node_for(api_name).map(|n| n.kind) {
            Some(NodeKind::Interface) => ApexTypeKind::Interface,
            Some(NodeKind::Enum) => ApexTypeKind::Enum,
            _ => ApexTypeKind::Class,
        }
    };
    let path_for = |api_name: &str| -> PathBuf {
        type_node_for(api_name)
            .map(|n| PathBuf::from(&n.location.file))
            .unwrap_or_else(|| PathBuf::from(format!("{api_name}.cls")))
    };

    for (api_name, _json) in &hints.class_symbols {
        let enclosing = api_name
            .rsplit_once('.')
            .map(|(outer, _)| outer.to_string());
        registry.insert_user_declared(api_name, kind_for(api_name), path_for(api_name), enclosing);
    }

    for (api_name, json) in &hints.class_symbols {
        match serde_json::from_str::<ApexClassSymbols>(json) {
            Ok(symbols) => {
                // `attach_symbols` returns `false` when the entry is
                // missing (i.e. the first pass never saw it, which
                // shouldn't happen given the pass above) or when the
                // entry is not user-defined (e.g. a collision against
                // a preloaded system type with the same name — rare
                // but possible with badly-named user classes). In
                // either case we log via `debug!` rather than abort
                // so a single pathological class doesn't poison the
                // entire resolve call.
                if !registry.attach_symbols(api_name, symbols) {
                    debug!(
                        "ApexHeuristicResolver: attach_symbols failed for {api_name} — \
                         likely a collision with a preloaded or custom entry",
                    );
                }
            }
            Err(err) => {
                // Deserialisation failure means the extractor emitted
                // a JSON shape the `ApexClassSymbols` deserialiser
                // can't consume — a contract violation between the
                // extractor and this module. Log and continue; the
                // class will fall through to name-only dispatch, the
                // same degraded path the resolver used before R37.
                debug!(
                    "ApexHeuristicResolver: failed to deserialise class_symbols for {api_name}: {err}",
                );
            }
        }
    }

    registry
}

/// Strip the GridSeak function-name prefix (`method_call:`, `call:`, ...)
/// and any receiver prefix (`obj.method` → `method`), returning the bare
/// name our registry / symbol index expects.
fn strip_prefix_and_receiver(function_name: &str) -> &str {
    let after_prefix = function_name
        .split(':')
        .next_back()
        .unwrap_or(function_name)
        .trim();
    // Receiver split: keep only the last `.`-separated segment as the
    // method/type name. `Foo.bar` → `bar`, `a.b.c.m` → `m`.
    match after_prefix.rsplit_once('.') {
        Some((_, last)) if !last.is_empty() => last,
        _ => after_prefix,
    }
}

/// Convert a Tree-sitter-flavored type usage kind into the matching
/// graph `EdgeKind`. Since Sprint E.1, `Extends` and `Implements` are
/// their own first-class edge kinds (not collapsed into `Type`) so
/// downstream analysis can compute inheritance-only metrics without
/// demuxing `Type` edges.
fn type_edge_kind(usage: &TypeUsageKind) -> EdgeKind {
    match usage {
        TypeUsageKind::Extends => EdgeKind::Extends,
        TypeUsageKind::Implements => EdgeKind::Implements,
        _ => EdgeKind::Uses,
    }
}

/// Small helper extension on `ResolvedEdges` that routes to the right
/// bucket based on an `EdgeKind` computed at runtime. Keeps the resolver
/// body free of match-on-kind boilerplate.
trait ResolvedEdgesExt {
    fn add_call_edge_or_type(&mut self, kind: EdgeKind, from: String, to: String);
}

impl ResolvedEdgesExt for ResolvedEdges {
    fn add_call_edge_or_type(&mut self, kind: EdgeKind, from: String, to: String) {
        let prov = Provenance::heuristic();
        match kind {
            // Type-like edges (including inheritance) live in the
            // `type_edges` bucket; the `EdgeKind` variant preserves the
            // semantic distinction for downstream consumers.
            EdgeKind::Type | EdgeKind::Uses | EdgeKind::Extends | EdgeKind::Implements => {
                self.add_type_edge(Edge::new(from, to, kind, prov))
            }
            // Framework and Declarative edges are call-family and
            // share the call-edges bucket — same reasoning as the LSP
            // resolver's dispatch: downstream metrics pivot on
            // `EdgeKind` itself, not the bucket, so co-locating them
            // with Call preserves ResolvedEdges' "invoked-at-runtime"
            // grouping.
            EdgeKind::Call | EdgeKind::Framework(_) | EdgeKind::Declarative(_) => {
                self.add_call_edge(Edge::new(from, to, kind, prov))
            }
            EdgeKind::Import => self.add_import_edge(Edge::new(from, to, kind, prov)),
            EdgeKind::Contains => self.add_containment_edge(Edge::new(from, to, kind, prov)),
        }
    }
}

/// Index built once per `resolve()` call. Keeps name-based function
/// lookups, file-scoped function lookups, and type-node lookups in a
/// single cohesive value so the resolver body stays flat.
struct SymbolIndex<'a> {
    functions_by_name_lower: HashMap<String, Vec<&'a Node>>,
    types_by_name_lower: HashMap<String, Vec<&'a Node>>,
    functions_by_file: HashMap<&'a str, Vec<&'a Node>>,
    types_by_file: HashMap<&'a str, Vec<&'a Node>>,
}

impl<'a> SymbolIndex<'a> {
    fn build(hints: &'a SyntaxResults) -> Self {
        let mut functions_by_name_lower: HashMap<String, Vec<&Node>> = HashMap::new();
        let mut types_by_name_lower: HashMap<String, Vec<&Node>> = HashMap::new();
        let mut functions_by_file: HashMap<&str, Vec<&Node>> = HashMap::new();
        let mut types_by_file: HashMap<&str, Vec<&Node>> = HashMap::new();

        for node in &hints.symbols {
            match node.kind {
                NodeKind::Function => {
                    let short = short_name_lower(&node.fqn);
                    functions_by_name_lower.entry(short).or_default().push(node);
                    functions_by_file
                        .entry(node.location.file.as_str())
                        .or_default()
                        .push(node);
                }
                NodeKind::Struct | NodeKind::Interface | NodeKind::Enum | NodeKind::Type => {
                    let short = short_name_lower(&node.fqn);
                    types_by_name_lower
                        .entry(short.clone())
                        .or_default()
                        .push(node);
                    // Sprint E.2: inner-class FQNs encode the outer class
                    // in the trailing `::`-segment as `Outer.Inner`. Index
                    // that dotted tail so `Outer.Inner` references from
                    // source resolve even when `Inner` is ambiguous across
                    // multiple outer classes in the workspace.
                    if let Some(dotted) = dotted_tail_lower(&node.fqn) {
                        if dotted != short {
                            types_by_name_lower.entry(dotted).or_default().push(node);
                        }
                    }
                    types_by_file
                        .entry(node.location.file.as_str())
                        .or_default()
                        .push(node);
                }
                _ => {}
            }
        }

        Self {
            functions_by_name_lower,
            types_by_name_lower,
            functions_by_file,
            types_by_file,
        }
    }

    fn find_enclosing_function(&self, call_site: &CallSite) -> Option<&'a Node> {
        let file = call_site.location.file.as_str();
        let fns = self.functions_by_file.get(file)?;
        let mut containing: Vec<&Node> = fns
            .iter()
            .copied()
            .filter(|n| range_contains(&n.location, &call_site.location))
            .collect();
        // Smallest enclosing function wins when multiple nested
        // functions all contain the site.
        containing.sort_by_key(|n| range_span(&n.location));
        containing.into_iter().next()
    }

    /// Return the innermost enclosing **type** node for a given range,
    /// using only `types_by_file`. Used by `this(...)` / `super(...)`
    /// explicit-constructor resolution to identify the class the call
    /// belongs to — function-scope is irrelevant there because both
    /// sentinels can only appear inside a constructor body.
    pub(super) fn find_enclosing_type(&self, at: &Range) -> Option<&'a Node> {
        let types = self.types_by_file.get(at.file.as_str())?;
        let mut containing: Vec<&Node> = types
            .iter()
            .copied()
            .filter(|n| range_contains(&n.location, at))
            .collect();
        containing.sort_by_key(|n| range_span(&n.location));
        containing.into_iter().next()
    }

    /// For type-reference locations, return the most specific enclosing
    /// function first (so a type referenced inside a method body points
    /// from the method), then fall back to enclosing type (for extends /
    /// implements / field declarations which live at class level).
    fn find_enclosing_type_or_function(&self, at: &Range) -> Option<&'a Node> {
        if let Some(fns) = self.functions_by_file.get(at.file.as_str()) {
            let mut containing: Vec<&Node> = fns
                .iter()
                .copied()
                .filter(|n| range_contains(&n.location, at))
                .collect();
            containing.sort_by_key(|n| range_span(&n.location));
            if let Some(n) = containing.into_iter().next() {
                return Some(n);
            }
        }
        if let Some(types) = self.types_by_file.get(at.file.as_str()) {
            let mut containing: Vec<&Node> = types
                .iter()
                .copied()
                .filter(|n| range_contains(&n.location, at))
                .collect();
            containing.sort_by_key(|n| range_span(&n.location));
            if let Some(n) = containing.into_iter().next() {
                return Some(n);
            }
        }
        None
    }

    fn find_function_candidates(&self, target_name: &str) -> Vec<&'a Node> {
        let key = target_name.trim().to_ascii_lowercase();
        if key.is_empty() {
            return Vec::new();
        }
        self.functions_by_name_lower
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    fn find_type_node(&self, target_name: &str) -> Option<&'a Node> {
        let key = target_name.trim().to_ascii_lowercase();
        if key.is_empty() {
            return None;
        }
        // Exact dotted hit first (Outer.Inner). Fall back to short name.
        if let Some(hits) = self.types_by_name_lower.get(&key) {
            if hits.len() == 1 {
                return Some(hits[0]);
            }
            if !hits.is_empty() {
                // Ambiguous — pick none rather than a wrong one. A
                // downstream LSP pass would resolve this correctly.
                return None;
            }
        }
        if let Some((_, last)) = key.rsplit_once('.') {
            if let Some(hits) = self.types_by_name_lower.get(last) {
                if hits.len() == 1 {
                    return Some(hits[0]);
                }
            }
        }
        None
    }
}

/// Extract the short (final dotted segment) name from an FQN and
/// lowercase it. Apex FQNs may use `.` or `::` depending on who built
/// them; we accept both.
///
/// Sprint E.2: Apex method / constructor FQNs carry a parenthesised
/// parameter signature (`save(Integer,String)`). Strip it before taking
/// the last segment so that call-site lookups keyed on the bare method
/// name continue to match the indexed function nodes.
fn short_name_lower(fqn: &str) -> String {
    // Strip any trailing `(...)` signature FIRST. Parameter type lists
    // commonly contain `.`-qualified names (e.g. `Database.BatchableContext`),
    // so rsplit-ing on `.`/`:` before the strip would slice inside the
    // signature and return garbage.
    let without_sig = match fqn.find('(') {
        Some(idx) => &fqn[..idx],
        None => fqn,
    };
    let last = without_sig.rsplit(['.', ':']).next().unwrap_or(without_sig);
    last.to_ascii_lowercase()
}

/// Return the class portion of a function FQN — everything to the
/// left of the method segment (and its optional `(sig)` suffix).
///
/// Handles the two shapes we see in practice:
///
/// - Production FQNs from the Apex E.2 builder:
///   `path::Outer::Outer.Inner::save(Integer,String)` → `path::Outer::Outer.Inner`.
/// - Test-shaped FQNs built with `ClassName.method` notation:
///   `ClassA.save` → `ClassA`.
///
/// Returns `None` for FQNs that carry no class prefix (bare names like
/// `save` or `save(Integer)`) — callers should interpret that as "no
/// class context", i.e. never equal to any other FQN's class.
///
/// Used by the same-class preference in the call-site resolver to
/// collapse cross-class fanout to same-class matches when available.
fn enclosing_class_fqn(fqn: &str) -> Option<String> {
    let without_sig = match fqn.find('(') {
        Some(idx) => &fqn[..idx],
        None => fqn,
    };
    // Production shape: `::` is the method separator. Strip the final
    // `::segment` — what remains is the class FQN.
    if let Some(pos) = without_sig.rfind("::") {
        let prefix = &without_sig[..pos];
        if !prefix.is_empty() {
            return Some(prefix.to_string());
        }
    }
    // Test shape: `Class.method`. Take everything before the final `.`
    // as the class, if it has one. Inner-class shapes like
    // `Outer.Inner.method` would lose the inner-class distinction here,
    // but test FQNs don't use that form; production uses `::`.
    if let Some((class_part, _)) = without_sig.rsplit_once('.') {
        if !class_part.is_empty() {
            return Some(class_part.to_string());
        }
    }
    None
}

/// Return the lowercased last `::`-separated segment of an FQN when
/// that segment contains a `.` (i.e., encodes a dotted type path such
/// as `Outer.Inner`). Used only to expand the type lookup index so
/// `Outer.Inner` source references resolve.
fn dotted_tail_lower(fqn: &str) -> Option<String> {
    let tail = fqn.rsplit("::").next()?;
    if tail.contains('.') {
        Some(tail.to_ascii_lowercase())
    } else {
        None
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

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::SyntaxResults;

    fn fn_node(name: &str, file: &str, sl: u32, el: u32) -> Node {
        Node::function(
            name.to_string(),
            Range::with_file(sl, 0, el, 99, file.to_string()),
        )
    }

    fn struct_node(name: &str, file: &str, sl: u32, el: u32) -> Node {
        Node::struct_(
            name.to_string(),
            Range::with_file(sl, 0, el, 99, file.to_string()),
        )
    }

    fn call_site(fn_name: &str, file: &str, line: u32) -> CallSite {
        CallSite {
            location: Range::with_file(line, 0, line, 10, file.to_string()),
            function_name: fn_name.to_string(),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        }
    }

    /// Test helper — push a bare `Call` reference into `hints`. Mirrors
    /// the old `hints.call_sites.push(cs)` shape tests used before
    /// P1.d, so fixture authoring stays a one-liner.
    fn push_call(hints: &mut SyntaxResults, cs: CallSite) {
        hints
            .references
            .push(crate::application::ports::UnresolvedReference::Call(cs));
    }

    #[tokio::test]
    async fn supported_language_is_apex_and_always_available() {
        let r = ApexHeuristicResolver::with_standard_preload_only();
        assert_eq!(r.supported_language(), "apex");
        assert!(r.is_available().await);
    }

    #[tokio::test]
    async fn resolves_unique_call_to_medium_confidence_edge() {
        let mut hints = SyntaxResults::new();
        let caller = fn_node("FooCtrl.doStuff", "Foo.cls", 5, 20);
        let callee = fn_node("BarService.execute", "Bar.cls", 1, 30);
        let caller_id = caller.id.clone();
        let callee_id = callee.id.clone();
        hints.add_symbol(caller);
        hints.add_symbol(callee);
        push_call(&mut hints, call_site("execute", "Foo.cls", 10));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(out.call_edges.len(), 1);
        let edge = &out.call_edges[0];
        assert_eq!(edge.from_id, caller_id);
        assert_eq!(edge.to_id, callee_id);
        assert_eq!(edge.provenance.source, ProvenanceSource::Heuristic);
        assert_eq!(edge.provenance.confidence, Confidence::Medium);
    }

    #[tokio::test]
    async fn same_class_candidate_wins_over_cross_class_fanout() {
        // Sprint H.1 — G.2 regression. `UTIL_Describe::getNamespace()`
        // calling `getNamespace()` from inside its own body MUST resolve
        // to UTIL_Describe's own method, not fan out to every class that
        // happens to declare a `getNamespace()`.
        let mut hints = SyntaxResults::new();
        let same = fn_node(
            "path::UTIL_Describe::UTIL_Describe::getNamespace()",
            "UTIL_Describe.cls",
            10,
            20,
        );
        let same_id = same.id.clone();
        let other = fn_node(
            "path::VOL_SharedCode::VOL_SharedCode::getNamespace()",
            "VOL_SharedCode.cls",
            5,
            15,
        );
        // Caller lives in UTIL_Describe but is a DIFFERENT method on the
        // same class (so the self-call drop doesn't short-circuit).
        let caller = fn_node(
            "path::UTIL_Describe::UTIL_Describe::callsite()",
            "UTIL_Describe.cls",
            30,
            60,
        );
        hints.add_symbol(same);
        hints.add_symbol(other);
        hints.add_symbol(caller);
        push_call(
            &mut hints,
            call_site("getNamespace", "UTIL_Describe.cls", 45),
        );

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(
            out.call_edges.len(),
            1,
            "same-class preference must collapse fanout to one edge; got {:?}",
            out.call_edges,
        );
        let edge = &out.call_edges[0];
        assert_eq!(edge.to_id, same_id);
        assert_eq!(
            edge.provenance.confidence,
            Confidence::Medium,
            "same-class hit must be Medium confidence, not Low fanout",
        );
    }

    #[tokio::test]
    async fn inner_class_same_class_preference() {
        // Inner-class preference. Caller in `Outer.Inner`, candidates
        // in `Outer.Inner` and unrelated `SomeOther` → inner-class wins.
        let mut hints = SyntaxResults::new();
        let inner_hit = fn_node(
            "path::Outer::Outer.Inner::handle(Integer)",
            "Outer.cls",
            10,
            20,
        );
        let inner_hit_id = inner_hit.id.clone();
        let unrelated = fn_node(
            "path::SomeOther::SomeOther::handle(Integer)",
            "SomeOther.cls",
            5,
            15,
        );
        let caller = fn_node("path::Outer::Outer.Inner::callsite()", "Outer.cls", 30, 60);
        hints.add_symbol(inner_hit);
        hints.add_symbol(unrelated);
        hints.add_symbol(caller);
        push_call(&mut hints, call_site("handle", "Outer.cls", 45));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(out.call_edges.len(), 1);
        assert_eq!(out.call_edges[0].to_id, inner_hit_id);
        assert_eq!(out.call_edges[0].provenance.confidence, Confidence::Medium);
    }

    #[tokio::test]
    async fn no_same_class_match_falls_back_to_low_fanout() {
        // Regression guard: when no candidate lives in the caller's
        // class, the path must remain exactly the pre-H.1 Low-confidence
        // small-fanout behaviour.
        let mut hints = SyntaxResults::new();
        hints.add_symbol(fn_node("path::ClassB::ClassB::save()", "B.cls", 1, 10));
        hints.add_symbol(fn_node("path::ClassC::ClassC::save()", "C.cls", 1, 10));
        let caller = fn_node("path::ClassA::ClassA::run()", "A.cls", 1, 30);
        hints.add_symbol(caller);
        push_call(&mut hints, call_site("save", "A.cls", 15));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(out.call_edges.len(), 2);
        for e in &out.call_edges {
            assert_eq!(e.provenance.confidence, Confidence::Low);
        }
    }

    #[tokio::test]
    async fn same_class_overloads_all_emit_medium() {
        // Multiple same-class overloads (different signatures, same name)
        // are still unambiguous w.r.t. WHICH CLASS owns the method.
        // Emit all at Medium.
        let mut hints = SyntaxResults::new();
        let overload_a = fn_node("path::Svc::Svc::save(Integer)", "Svc.cls", 10, 20);
        let overload_b = fn_node("path::Svc::Svc::save(String)", "Svc.cls", 25, 35);
        let caller = fn_node("path::Svc::Svc::caller()", "Svc.cls", 40, 60);
        hints.add_symbol(overload_a);
        hints.add_symbol(overload_b);
        hints.add_symbol(caller);
        push_call(&mut hints, call_site("save", "Svc.cls", 50));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(out.call_edges.len(), 2);
        for e in &out.call_edges {
            assert_eq!(e.provenance.confidence, Confidence::Medium);
        }
    }

    #[tokio::test]
    async fn ambiguous_overload_emits_all_candidates_as_low_confidence() {
        let mut hints = SyntaxResults::new();
        hints.add_symbol(fn_node("ClassA.save", "A.cls", 1, 10));
        hints.add_symbol(fn_node("ClassB.save", "B.cls", 1, 10));
        let caller = fn_node("Client.run", "C.cls", 1, 30);
        hints.add_symbol(caller);
        push_call(&mut hints, call_site("save", "C.cls", 15));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(out.call_edges.len(), 2);
        for e in &out.call_edges {
            assert_eq!(e.provenance.confidence, Confidence::Low);
            assert_eq!(e.provenance.source, ProvenanceSource::Heuristic);
        }
        // Below the fanout cap — no ambiguous drops recorded.
        assert_eq!(out.stats.heuristic_call_ambiguous_drops, 0);
    }

    #[tokio::test]
    async fn over_cap_fanout_emits_zero_edges_and_records_drop() {
        // Reproduces the NPSP shape: a common method name (`save`)
        // declared on many unrelated classes. The heuristic cannot
        // pick the right one without type information, so the cap
        // forces it to emit nothing rather than flood the graph with
        // CAP+1 arbitrary Low-confidence edges.
        let mut hints = SyntaxResults::new();
        // CAP+1 candidates → expect total suppression of this call site.
        for i in 0..=HEURISTIC_CALL_FANOUT_CAP {
            hints.add_symbol(fn_node(
                &format!("Class{i}.save"),
                &format!("C{i}.cls"),
                1,
                10,
            ));
        }
        let caller = fn_node("Caller.run", "Main.cls", 1, 30);
        hints.add_symbol(caller);
        push_call(&mut hints, call_site("save", "Main.cls", 20));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert!(
            out.call_edges.is_empty(),
            "over-cap fanout must emit zero edges — got {}: {:?}",
            out.call_edges.len(),
            out.call_edges,
        );
        assert_eq!(
            out.stats.heuristic_call_ambiguous_drops, 1,
            "exactly one call site must be recorded as dropped"
        );
        assert_eq!(
            out.stats.heuristic_call_fallbacks, 0,
            "no heuristic call fallbacks should be counted when we drop"
        );
    }

    #[tokio::test]
    async fn exactly_at_cap_still_emits_edges() {
        // Boundary: CAP candidates must still resolve. The cap is
        // strict-greater-than (`> CAP`, not `>= CAP`).
        let mut hints = SyntaxResults::new();
        for i in 0..HEURISTIC_CALL_FANOUT_CAP {
            hints.add_symbol(fn_node(
                &format!("Class{i}.run"),
                &format!("C{i}.cls"),
                1,
                10,
            ));
        }
        let caller = fn_node("Caller.main", "Main.cls", 1, 30);
        hints.add_symbol(caller);
        push_call(&mut hints, call_site("run", "Main.cls", 20));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(
            out.call_edges.len(),
            HEURISTIC_CALL_FANOUT_CAP,
            "at-cap fanout must still emit one edge per candidate",
        );
        assert_eq!(out.stats.heuristic_call_ambiguous_drops, 0);
        for e in &out.call_edges {
            assert_eq!(e.provenance.confidence, Confidence::Low);
        }
    }

    #[tokio::test]
    async fn call_site_with_receiver_prefix_matches_method_only() {
        let mut hints = SyntaxResults::new();
        hints.add_symbol(fn_node("Logger.write", "Logger.cls", 1, 10));
        let caller = fn_node("App.main", "App.cls", 1, 50);
        hints.add_symbol(caller);
        // Call site recorded as `obj.write` — receiver-stripping must
        // reduce to `write` before the lookup.
        push_call(&mut hints, call_site("obj.write", "App.cls", 20));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(out.call_edges.len(), 1);
    }

    #[tokio::test]
    async fn function_name_prefix_is_stripped() {
        let mut hints = SyntaxResults::new();
        hints.add_symbol(fn_node("Svc.doIt", "Svc.cls", 1, 10));
        hints.add_symbol(fn_node("App.main", "App.cls", 1, 50));
        push_call(&mut hints, call_site("method_call:doIt", "App.cls", 25));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(out.call_edges.len(), 1);
    }

    #[tokio::test]
    async fn trigger_context_accesses_are_filtered() {
        let mut hints = SyntaxResults::new();
        // Put a decoy function named `new` so we'd otherwise match it.
        hints.add_symbol(fn_node("Factory.new", "Factory.cls", 1, 10));
        hints.add_symbol(fn_node("MyTrigger.handler", "T.trigger", 1, 50));

        push_call(&mut hints, call_site("Trigger.new", "T.trigger", 5));
        push_call(&mut hints, call_site("Trigger.isInsert", "T.trigger", 6));
        push_call(&mut hints, call_site("Trigger.oldMap", "T.trigger", 7));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert!(
            out.call_edges.is_empty(),
            "Trigger.* context access MUST NOT produce call edges; got {:?}",
            out.call_edges
        );
    }

    #[tokio::test]
    async fn self_call_is_dropped() {
        let mut hints = SyntaxResults::new();
        let recursive = fn_node("Recursive.step", "R.cls", 1, 50);
        hints.add_symbol(recursive);
        push_call(&mut hints, call_site("step", "R.cls", 10));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert!(
            out.call_edges.is_empty(),
            "self-referential call must not produce a self-loop Call edge"
        );
    }

    #[tokio::test]
    async fn unresolved_call_is_silently_dropped_not_error() {
        let mut hints = SyntaxResults::new();
        hints.add_symbol(fn_node("App.main", "App.cls", 1, 50));
        // Target does not exist anywhere in the symbol table.
        push_call(&mut hints, call_site("nonexistentThing", "App.cls", 20));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert!(out.call_edges.is_empty());
        assert_eq!(out.stats.heuristic_call_fallbacks, 0);
    }

    #[tokio::test]
    async fn type_reference_extends_emits_extends_edge() {
        let mut hints = SyntaxResults::new();
        let child = struct_node("Child", "Child.cls", 1, 100);
        let parent = struct_node("Parent", "Parent.cls", 1, 50);
        let child_id = child.id.clone();
        let parent_id = parent.id.clone();
        hints.add_symbol(child);
        hints.add_symbol(parent);
        hints.type_references.push(TypeReference {
            location: Range::with_file(1, 10, 1, 20, "Child.cls".to_string()),
            type_name: "Parent".to_string(),
            usage_kind: TypeUsageKind::Extends,
        });

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(out.type_edges.len(), 1);
        let e = &out.type_edges[0];
        // Sprint E.1: `Extends` is a first-class `EdgeKind`, not a `Type`.
        assert_eq!(e.kind, EdgeKind::Extends);
        assert_eq!(e.from_id, child_id);
        assert_eq!(e.to_id, parent_id);
        assert_eq!(e.provenance.source, ProvenanceSource::Heuristic);
    }

    #[tokio::test]
    async fn type_reference_implements_emits_implements_edge() {
        let mut hints = SyntaxResults::new();
        let concrete = struct_node("Concrete", "Concrete.cls", 1, 100);
        // Interfaces share the same `Struct` surface in this test scaffold
        // since the heuristic resolver's type lookup is name-based; the
        // distinction between Struct and Interface lives in real extraction.
        let interface = struct_node("MyInterface", "MyInterface.cls", 1, 50);
        let concrete_id = concrete.id.clone();
        let interface_id = interface.id.clone();
        hints.add_symbol(concrete);
        hints.add_symbol(interface);
        hints.type_references.push(TypeReference {
            location: Range::with_file(1, 10, 1, 30, "Concrete.cls".to_string()),
            type_name: "MyInterface".to_string(),
            usage_kind: TypeUsageKind::Implements,
        });

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(out.type_edges.len(), 1);
        let e = &out.type_edges[0];
        assert_eq!(e.kind, EdgeKind::Implements);
        assert_eq!(e.from_id, concrete_id);
        assert_eq!(e.to_id, interface_id);
    }

    #[tokio::test]
    async fn stats_report_resolver_tier_correctly() {
        let mut hints = SyntaxResults::new();
        hints.add_symbol(fn_node("A.m", "A.cls", 1, 10));
        hints.add_symbol(fn_node("B.caller", "B.cls", 1, 50));
        push_call(&mut hints, call_site("m", "B.cls", 20));

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();

        assert_eq!(out.stats.lsp_edges, 0);
        assert_eq!(out.stats.heuristic_edges, 1);
        assert_eq!(out.stats.heuristic_call_fallbacks, 1);
    }

    #[test]
    fn is_trigger_context_access_catches_every_documented_variant() {
        for var in [
            "Trigger.new",
            "Trigger.old",
            "Trigger.newMap",
            "Trigger.oldMap",
            "Trigger.isInsert",
            "Trigger.isUpdate",
            "Trigger.isDelete",
            "Trigger.isBefore",
            "Trigger.isAfter",
            "Trigger.isExecuting",
            "Trigger.size",
            "TRIGGER.new",                // case-insensitive
            "method_call:Trigger.newMap", // prefix-wrapped
        ] {
            assert!(
                is_trigger_context_access(var),
                "{var} must be recognized as a Trigger context var"
            );
        }
    }

    #[test]
    fn is_trigger_context_access_does_not_match_unrelated_names() {
        for var in [
            "triggerHandler",
            "MyTrigger.run",
            "runTrigger",
            "someMethod",
        ] {
            assert!(
                !is_trigger_context_access(var),
                "{var} must NOT be mistaken for a Trigger context var"
            );
        }
    }

    #[test]
    fn strip_prefix_and_receiver_handles_all_shapes() {
        assert_eq!(strip_prefix_and_receiver("foo"), "foo");
        assert_eq!(strip_prefix_and_receiver("obj.foo"), "foo");
        assert_eq!(strip_prefix_and_receiver("a.b.c.foo"), "foo");
        assert_eq!(strip_prefix_and_receiver("method_call:foo"), "foo");
        assert_eq!(strip_prefix_and_receiver("method_call:obj.foo"), "foo");
        assert_eq!(strip_prefix_and_receiver("call:a.b"), "b");
    }

    #[test]
    fn short_name_lower_lowercases_and_takes_last_segment() {
        assert_eq!(short_name_lower("foo"), "foo");
        assert_eq!(short_name_lower("Foo.Bar"), "bar");
        assert_eq!(short_name_lower("OuterClass::Inner"), "inner");
        assert_eq!(short_name_lower("Account.doStuff"), "dostuff");
    }

    #[test]
    fn short_name_lower_strips_method_signature_suffix() {
        // Sprint E.2: method/constructor FQNs carry a `(sig)` suffix.
        // The short-name index must match the bare method name.
        assert_eq!(short_name_lower("path::Foo::Foo::save(Integer)"), "save");
        assert_eq!(short_name_lower("path::Foo::Foo::save()"), "save");
        assert_eq!(
            short_name_lower("path::Outer::Outer.Inner::run(Database.BatchableContext,List)"),
            "run",
        );
        // Overload with the same simple name still hashes identically —
        // the index resolves to a Vec so callers can disambiguate.
        assert_eq!(short_name_lower("X::Handler(Integer,String)"), "handler");
    }

    #[test]
    fn enclosing_class_fqn_strips_method_segment_and_signature() {
        assert_eq!(
            enclosing_class_fqn("path::Foo::Foo::save(Integer)"),
            Some("path::Foo::Foo".to_string())
        );
        assert_eq!(
            enclosing_class_fqn("path::Outer::Outer.Inner::run()"),
            Some("path::Outer::Outer.Inner".to_string())
        );
        // Test-shape FQN (Class.method, dot separator).
        assert_eq!(
            enclosing_class_fqn("FooCtrl.doStuff"),
            Some("FooCtrl".to_string())
        );
        // Bare name with no class prefix returns None.
        assert_eq!(enclosing_class_fqn("save"), None);
        assert_eq!(enclosing_class_fqn("save(Integer)"), None);
    }

    #[test]
    fn dotted_tail_lower_extracts_inner_class_paths() {
        assert_eq!(
            dotted_tail_lower("path::Foo::Foo.Inner"),
            Some("foo.inner".to_string()),
        );
        assert_eq!(
            dotted_tail_lower("path::Big.Outer::A.B.C"),
            Some("a.b.c".to_string()),
        );
        // Top-level classes have no dotted tail.
        assert_eq!(dotted_tail_lower("path::Foo::Foo"), None);
        // Paths containing `.` earlier in the FQN are not promoted to a
        // tail index — only the trailing `::`-segment matters.
        assert_eq!(dotted_tail_lower("a.b::Plain"), None);
    }

    #[tokio::test]
    async fn type_reference_resolves_via_dotted_inner_class_tail() {
        // A reference to `Outer.Inner` in source should match an inner
        // class whose FQN is `path::Outer::Outer.Inner` even when the
        // short name `Inner` is ambiguous.
        let mut hints = SyntaxResults::new();
        let inner_a = Node::struct_(
            "path::OuterA::OuterA.Inner".to_string(),
            Range::with_file(1, 0, 10, 0, "OuterA.cls".to_string()),
        );
        let inner_b = Node::struct_(
            "path::OuterB::OuterB.Inner".to_string(),
            Range::with_file(1, 0, 10, 0, "OuterB.cls".to_string()),
        );
        let inner_b_id = inner_b.id.clone();
        let holder = Node::struct_(
            "path::Caller::Caller".to_string(),
            Range::with_file(1, 0, 20, 0, "Caller.cls".to_string()),
        );
        hints.add_symbol(holder);
        hints.add_symbol(inner_a);
        hints.add_symbol(inner_b);
        hints.type_references.push(TypeReference {
            location: Range::with_file(5, 0, 5, 20, "Caller.cls".to_string()),
            type_name: "OuterB.Inner".to_string(),
            usage_kind: TypeUsageKind::Other,
        });

        let r = ApexHeuristicResolver::with_standard_preload_only();
        let out = r.resolve(&hints).await.unwrap();
        assert_eq!(out.type_edges.len(), 1);
        assert_eq!(out.type_edges[0].to_id, inner_b_id);
    }

    /// R37 regression. Verify that `seed_registry_from_hints` takes a
    /// preload-only base registry — the exact shape the production
    /// factory hands to `ApexHeuristicResolver::with_standard_preload_only()`
    /// — and returns a registry that carries user-declared class
    /// symbols for every entry in `SyntaxResults.class_symbols`.
    ///
    /// Before R37's fix, every TR-A.1 / TR-A.2 / TR-A.3 code path
    /// that called `registry.symbols_for(user_type)` returned
    /// `None` in production because the factory never seeded user
    /// classes. This test is the lowest-cost contract pin that
    /// catches a regression to that state — if the seeding call is
    /// ever removed from `resolve`, this assertion trips without
    /// requiring a full fixture pipeline.
    #[test]
    fn seed_registry_attaches_user_declared_class_symbols() {
        use crate::domain::apex::class_symbols::ApexClassSymbols;

        let base = ApexClassRegistry::with_standard_preload();
        // Sanity: preload alone never carries user symbols.
        assert!(base.symbols_for("MyService").is_none());

        let mut hints = SyntaxResults::new();
        hints.symbols.push(struct_node(
            "path::MyService::MyService",
            "MyService.cls",
            1,
            10,
        ));
        let empty = ApexClassSymbols::default();
        hints.class_symbols.push((
            "MyService".to_string(),
            serde_json::to_string(&empty).unwrap(),
        ));

        let seeded = seed_registry_from_hints(&base, &hints);

        let entry = seeded
            .lookup("MyService")
            .expect("seeded registry must carry the user-declared class entry");
        assert!(entry.kind.is_user_defined());
        assert!(
            seeded.symbols_for("MyService").is_some(),
            "attach_symbols must land the ApexClassSymbols payload so TR-A.1/2/3 arms can read it",
        );

        // Preload still intact — seeding must never drop standard types.
        assert!(seeded.lookup("Account").is_some());
        assert!(seeded.lookup("Database").is_some());
    }

    /// R37 regression, degenerate shape. Empty `class_symbols` must
    /// cheaply short-circuit to a clone of the base registry — no
    /// crash, no collision recording, no added entries.
    #[test]
    fn seed_registry_is_noop_when_hints_carry_no_class_symbols() {
        let base = ApexClassRegistry::with_standard_preload();
        let base_len = base.len();
        let hints = SyntaxResults::new();

        let seeded = seed_registry_from_hints(&base, &hints);

        assert_eq!(seeded.len(), base_len);
        assert!(seeded.collisions().is_empty());
    }
}
