//! Rust Layer-2 semantic resolver.
//!
//! Wraps [`graphengine_ra_ide_adapter::RustAnalyzerSemanticResolver`]
//! and implements the engine's [`SemanticResolver`] port so the
//! existing pipeline (`ParsingPipeline::execute` →
//! `SemanticResolverService::resolve_with_fallback`) can consume it
//! transparently. Every successfully-resolved call site produces a
//! single `Edge { kind: Call, provenance: Compiler / High|Medium }`
//! edge; the call-site range is recorded in
//! [`ResolvedEdges::resolved_call_sites`] so the heuristic fallback
//! downstream does not emit a contradicting Low-confidence edge to a
//! name-match sibling.
//!
//! ## Two-jobs rule at work (T6 design doc §3 / §4)
//!
//! The existing [`LspResolver`] trait conflates *transport* (stdio
//! JSON-RPC to a rust-analyzer subprocess) with *authority*
//! (semantic-grade confidence). This adapter delivers the authority
//! **without** the transport — it links `ra_ap_ide` as a library and
//! calls `goto_definition` against the in-process `AnalysisHost`. No
//! subprocess, no wire-protocol parsing, no timeout tuning. The
//! subprocess-LSP path continues to live in `infrastructure::lsp`
//! for TypeScript / Apex.
//!
//! ## Measured-fallback discipline (T6 §5.5)
//!
//! Every error from the adapter is counted, not swallowed. The
//! counter surfaces in the `ResolutionStatsSummary::heuristic_*`
//! bucket by design (a Layer-2 miss is a Layer-1 fallback) so
//! existing CLI / analysis consumers do not need schema changes to
//! see the effect. Successfully emitted call edges increment
//! `compiler_edges` (not `lsp_edges` — this path is not subprocess LSP).
//!
//! ## Performance envelope (UF-FU-011)
//!
//! Cold `load_workspace_at` on `gridseak-self` (~56 kloc production
//! code, Rust 1.91.1, aarch64-apple-darwin) must complete within the
//! kill-criterion tightened in T6 §5.5 (0.5 s/kloc). The per-query
//! cost is a salsa-memoised snapshot; we cache the resolver for the
//! lifetime of the scan.
//!
//! Not `Sync`: rust-analyzer snapshots are thread-local by
//! construction. The `SemanticResolver` trait takes `&self` but the
//! resolver is driven serially from one scan task.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use graphengine_ra_ide_adapter::{
    Confidence as AdapterConfidence, ResolvedTarget, RustAnalyzerSemanticResolver,
    SemanticQueryInput, SemanticResolverError,
};
use tracing::{debug, info, warn};

use crate::application::ports::{
    CallSite, IndexTarget, ResolvedEdges, SemanticResolver, SessionMetricsSnapshot, SyntaxResults,
    UnresolvedReference,
};
use crate::domain::{Confidence, Edge, EdgeKind, Provenance, ProvenanceSource, Range};
use crate::infrastructure::semantic::symbol_mapping::SymbolIndex;

/// Counters the resolver mutates as it processes a single `resolve`
/// call. Captured as a struct so the tiered-dogfood measurement
/// (T6 §5.5 / plan Gate 1.2) has a single place to read the "how
/// many High edges did we actually emit?" answer.
#[derive(Debug, Default)]
struct ResolveCounters {
    /// Number of `UnresolvedReference::Call` references inspected
    /// (framework / declarative bindings are not dispatched to the
    /// Rust adapter — they belong to Apex paths).
    call_refs_seen: AtomicU64,
    /// Successful resolutions at `Confidence::High`.
    high_resolutions: AtomicU64,
    /// Successful resolutions downgraded to `Confidence::Medium`
    /// because rust-analyzer returned multiple plausible candidates.
    medium_resolutions: AtomicU64,
    /// Calls the adapter returned `Ok(None)` on — no target found.
    /// These fall through to the heuristic resolver.
    no_target_misses: AtomicU64,
    /// Adapter errors — each increment corresponds to a named failure
    /// mode in [`SemanticResolverError`]. Falls through to heuristic.
    adapter_errors: AtomicU64,
    /// Emitted edges we actually stamped on the output `ResolvedEdges`.
    /// Distinct from `high + medium` because caller/callee lookup can
    /// still fail after the adapter resolves (e.g. a target symbol
    /// that the tree-sitter extractor never emitted as a Node).
    edges_emitted: AtomicU64,
    /// Adapter resolved (high+medium) but no enclosing caller symbol.
    symbol_caller_misses: AtomicU64,
    /// Adapter resolved but no callee symbol, and the target file is
    /// **inside** the scanned workspace — genuine in-repo mapping loss
    /// (line/name skew, extractor gap). This is the bucket Plan 01
    /// owns and the mapping-conversion bar measures.
    symbol_callee_misses: AtomicU64,
    /// Adapter resolved but the definition lives **outside** the
    /// scanned workspace (cargo registry, generated code under
    /// `target/`). Not mappable by definition — same split as
    /// `FallbackReason::ExternalDefinition` in the LSP channel, kept
    /// separate so the miss histogram does not cry wolf.
    external_target_misses: AtomicU64,
    /// Adapter resolved to the enclosing function itself (recursion).
    self_loop_drops: AtomicU64,
    /// Second+ call site resolving to the same `(caller, callee)` pair.
    duplicate_edge_drops: AtomicU64,
}

impl ResolveCounters {
    fn as_snapshot(&self) -> ResolveSnapshot {
        ResolveSnapshot {
            call_refs_seen: self.call_refs_seen.load(Ordering::Relaxed),
            high_resolutions: self.high_resolutions.load(Ordering::Relaxed),
            medium_resolutions: self.medium_resolutions.load(Ordering::Relaxed),
            no_target_misses: self.no_target_misses.load(Ordering::Relaxed),
            adapter_errors: self.adapter_errors.load(Ordering::Relaxed),
            edges_emitted: self.edges_emitted.load(Ordering::Relaxed),
            symbol_caller_misses: self.symbol_caller_misses.load(Ordering::Relaxed),
            symbol_callee_misses: self.symbol_callee_misses.load(Ordering::Relaxed),
            external_target_misses: self.external_target_misses.load(Ordering::Relaxed),
            self_loop_drops: self.self_loop_drops.load(Ordering::Relaxed),
            duplicate_edge_drops: self.duplicate_edge_drops.load(Ordering::Relaxed),
            callee_miss_samples: Vec::new(),
        }
    }

    fn as_snapshot_with_samples(&self, samples: Vec<CalleeMissSample>) -> ResolveSnapshot {
        let mut snap = self.as_snapshot();
        snap.callee_miss_samples = samples;
        snap
    }
}

/// One recorded `symbol_callee_miss` for plan-01 attribution (P3).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CalleeMissSample {
    pub call_site: String,
    pub target: String,
    /// `plan01_mappable` = symbol exists in tree-sitter index but lookup failed;
    /// `plan02_out_of_scope` = no matching Function node in the index.
    pub classification: String,
}

/// Snapshot of the resolver's internal counters. Used by the
/// `gridseak-self` dogfood harness and any future analysis consumer
/// to compute `high_ratio_on_calls` = `high / max(1, call_refs_seen)`
/// — the threshold the plan's tiered response keys off.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ResolveSnapshot {
    pub call_refs_seen: u64,
    pub high_resolutions: u64,
    pub medium_resolutions: u64,
    pub no_target_misses: u64,
    pub adapter_errors: u64,
    pub edges_emitted: u64,
    pub symbol_caller_misses: u64,
    pub symbol_callee_misses: u64,
    /// Resolutions whose definition sits outside the scanned workspace
    /// (registry crates, `target/` build output).
    #[serde(default)]
    pub external_target_misses: u64,
    pub self_loop_drops: u64,
    pub duplicate_edge_drops: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callee_miss_samples: Vec<CalleeMissSample>,
}

impl ResolveSnapshot {
    /// Adapter-resolved call sites (high + medium) before SymbolIndex mapping.
    pub fn adapter_resolved(&self) -> u64 {
        self.high_resolutions + self.medium_resolutions
    }

    /// Sum of every post-adapter resolution bucket (must equal `adapter_resolved`).
    pub fn resolution_bucket_sum(&self) -> u64 {
        self.edges_emitted
            + self.symbol_caller_misses
            + self.symbol_callee_misses
            + self.external_target_misses
            + self.self_loop_drops
            + self.duplicate_edge_drops
    }

    /// P1 identity: every adapter resolution lands in exactly one named bucket.
    pub fn accounting_identity_holds(&self) -> bool {
        self.resolution_bucket_sum() == self.adapter_resolved()
    }

    /// Resolutions that are correct behaviour to not emit: recursion
    /// self-loops, duplicate `(caller, callee)` pairs, and definitions
    /// outside the scanned workspace. None of these are mapping loss.
    pub fn legitimate_drops(&self) -> u64 {
        self.self_loop_drops + self.duplicate_edge_drops + self.external_target_misses
    }

    /// Raw fraction including legitimate drops in the denominator.
    pub fn conversion_rate(&self) -> f64 {
        let denom = self.adapter_resolved();
        if denom == 0 {
            return 0.0;
        }
        self.edges_emitted as f64 / denom as f64
    }

    /// Mapping conversion over the **mappable** universe: adapter
    /// resolutions minus legitimate drops (self-loops, duplicates,
    /// external-workspace targets). Measures Plan 01's actual claim —
    /// "we do not lose in-repo mappable edges" — without letting
    /// external-crate resolutions, which can never map to an in-repo
    /// node, masquerade as loss (P9, mirrors the LSP channel's
    /// `ExternalDefinition` split).
    pub fn mapping_conversion_rate(&self) -> f64 {
        let denom = self
            .adapter_resolved()
            .saturating_sub(self.legitimate_drops());
        if denom == 0 {
            return 0.0;
        }
        self.edges_emitted as f64 / denom as f64
    }
}

/// `SemanticResolver` impl driven by
/// [`RustAnalyzerSemanticResolver`]. Constructed at factory time
/// with the scan's workspace root; queries are served from the
/// in-process `AnalysisHost` that the adapter loaded once.
///
/// ## Sync / Send contract
///
/// The engine's [`SemanticResolver`] trait requires `Send + Sync`
/// because the pipeline hands the resolver to multiple async tasks.
/// The underlying `RustAnalyzerSemanticResolver` (and its
/// `AnalysisHost`) are `Send` but **not** `Sync` — salsa's
/// per-snapshot caches use `UnsafeCell` and `RefCell` interiors. We
/// therefore wrap the adapter in a [`Mutex`], which makes the whole
/// resolver `Sync` (`Mutex<T>: Sync` whenever `T: Send`). The lock
/// is never held across an `.await`, so the synchronous critical
/// section is safe for `#[async_trait]` dispatch.
pub struct RustLayer2SemanticResolver {
    inner: Mutex<RustAnalyzerSemanticResolver>,
    load_elapsed_ms: u128,
    workspace_root: PathBuf,
    counters: ResolveCounters,
    callee_miss_samples: Mutex<Vec<CalleeMissSample>>,
    /// All callee misses for the current `resolve()` pass; stratified
    /// down to [`CALLEE_MISS_SAMPLE_TARGET`] at finalize time (P7).
    callee_miss_pending: Mutex<Vec<CalleeMissRecord>>,
}

/// Internal miss record before P7 stratified sampling.
#[derive(Debug, Clone)]
struct CalleeMissRecord {
    call_site: CallSite,
    target_file: PathBuf,
    target_line: u32,
    target_symbol_name: String,
}

/// P7: how many misses to persist in telemetry; P3 adjudication minimum.
const CALLEE_MISS_SAMPLE_TARGET: usize = 30;
/// P7: minimum distinct call-site files represented in the sample.
const CALLEE_MISS_MIN_FILES: usize = 10;

impl RustLayer2SemanticResolver {
    /// Build the resolver against `workspace_root`. The path may
    /// point either at a directory containing `Cargo.toml` or at the
    /// `Cargo.toml` itself. Returns `Err` if `load_workspace_at`
    /// fails for any reason — callers surface this as "Layer 2
    /// unavailable; heuristic fallback will carry" rather than a
    /// scan-fatal error.
    pub fn new(workspace_root: &std::path::Path) -> Result<Self, SemanticResolverError> {
        let inner = RustAnalyzerSemanticResolver::from_workspace_root(workspace_root)?;
        let load_elapsed_ms = inner.load_elapsed_ms();
        let ws = inner.workspace_root().to_path_buf();
        info!(
            target: "rust_layer2::init",
            "rust Layer-2 adapter loaded in {} ms (workspace={})",
            load_elapsed_ms,
            ws.display(),
        );
        Ok(Self {
            inner: Mutex::new(inner),
            load_elapsed_ms,
            workspace_root: ws,
            counters: ResolveCounters::default(),
            callee_miss_samples: Mutex::new(Vec::new()),
            callee_miss_pending: Mutex::new(Vec::new()),
        })
    }

    /// Wall-clock cost of the one-time `load_workspace_at` call.
    /// Exposed for the dogfood measurement script.
    pub fn load_elapsed_ms(&self) -> u128 {
        self.load_elapsed_ms
    }

    /// Absolute path of the workspace the resolver was built against.
    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    /// Read-only snapshot of the resolver's telemetry counters.
    /// Exposed for the dogfood harness (Gate 1.2 measurement) so the
    /// tiered-response decision keys off real numbers, not an
    /// inferred / log-scraped estimate.
    pub fn snapshot(&self) -> ResolveSnapshot {
        let samples = self
            .callee_miss_samples
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        self.counters.as_snapshot_with_samples(samples)
    }

    /// Resolve one `UnresolvedReference::Call`. Returns:
    /// - `Some((edge, call_location))` when the adapter returned a
    ///   target *and* caller/callee symbols were found in the
    ///   syntax results.
    /// - `None` when the reference should fall through to the
    ///   heuristic resolver (adapter returned no target, hit an
    ///   error, or caller/callee could not be matched to symbols).
    fn resolve_one(
        &self,
        reference: &UnresolvedReference,
        symbol_index: &SymbolIndex<'_>,
        seen_call_edges: &mut std::collections::HashSet<(String, String)>,
    ) -> Option<(Edge, Range)> {
        let call_site = match reference {
            UnresolvedReference::Call(cs) => cs,
            // Framework / declarative bindings belong to Apex's
            // resolver paths. The Rust adapter has no way to resolve
            // them, so leave them for the existing heuristic.
            UnresolvedReference::FrameworkBinding(_)
            | UnresolvedReference::DeclarativeBinding(_) => return None,
        };

        self.counters.call_refs_seen.fetch_add(1, Ordering::Relaxed);

        let (query_line, query_col) = caret_for_callee(call_site);
        let query = SemanticQueryInput {
            file: PathBuf::from(&call_site.location.file),
            line: query_line,
            column: query_col,
        };

        let target = {
            // Scope the lock tightly — never held across an `.await`.
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            match inner.resolve(&query) {
                Ok(Some(t)) => t,
                Ok(None) => {
                    self.counters
                        .no_target_misses
                        .fetch_add(1, Ordering::Relaxed);
                    return None;
                }
                Err(err) => {
                    self.counters.adapter_errors.fetch_add(1, Ordering::Relaxed);
                    // Named, non-silent failure per the measured-fallback
                    // discipline in T6 §3 Q5.
                    debug!(
                        target: "rust_layer2::resolve",
                        "adapter error at {}:{}:{} — {}",
                        call_site.location.file, query_line, query_col, err,
                    );
                    return None;
                }
            }
        };

        match target.confidence {
            AdapterConfidence::High => self
                .counters
                .high_resolutions
                .fetch_add(1, Ordering::Relaxed),
            AdapterConfidence::Medium => self
                .counters
                .medium_resolutions
                .fetch_add(1, Ordering::Relaxed),
        };

        let caller = match symbol_index.find_enclosing_function(&call_site.location) {
            Some(c) => c,
            None => {
                self.counters
                    .symbol_caller_misses
                    .fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };
        let callee = match symbol_index.find_callee_for(&index_target_from_resolved(&target)) {
            Some(c) => c,
            None => {
                if is_external_target(&target.target_file, &self.workspace_root) {
                    // Definition lives outside the scanned workspace
                    // (registry crate, `target/` build output). Not
                    // mappable to an in-repo node by definition; keep
                    // it out of the mapping-loss bucket and samples.
                    self.counters
                        .external_target_misses
                        .fetch_add(1, Ordering::Relaxed);
                } else {
                    self.counters
                        .symbol_callee_misses
                        .fetch_add(1, Ordering::Relaxed);
                    self.record_callee_miss(call_site, &target, symbol_index);
                }
                return None;
            }
        };

        if caller.id == callee.id {
            self.counters
                .self_loop_drops
                .fetch_add(1, Ordering::Relaxed);
            debug!(
                target: "rust_layer2::resolve",
                "self-recursion at {} → {}; dropping edge (validator rejects Call self-loops)",
                caller.fqn, callee.fqn
            );
            return None;
        }

        let edge_key = (caller.id.clone(), callee.id.clone());
        if !seen_call_edges.insert(edge_key) {
            self.counters
                .duplicate_edge_drops
                .fetch_add(1, Ordering::Relaxed);
            return None;
        }

        let confidence = match target.confidence {
            AdapterConfidence::High => Confidence::High,
            AdapterConfidence::Medium => Confidence::Medium,
        };
        let edge = Edge::new(
            caller.id.clone(),
            callee.id.clone(),
            EdgeKind::Call,
            Provenance::new(ProvenanceSource::Compiler, confidence),
        );
        self.counters.edges_emitted.fetch_add(1, Ordering::Relaxed);
        Some((edge, call_site.location.clone()))
    }

    fn record_callee_miss(
        &self,
        call_site: &CallSite,
        target: &ResolvedTarget,
        symbol_index: &SymbolIndex<'_>,
    ) {
        let _ = symbol_index;
        let mut pending = self
            .callee_miss_pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending.push(CalleeMissRecord {
            call_site: call_site.clone(),
            target_file: target.target_file.clone(),
            target_line: target.target_line,
            target_symbol_name: target.target_symbol_name.clone(),
        });
    }

    fn finalize_callee_miss_samples(&self, symbol_index: &SymbolIndex<'_>) {
        let pending = {
            let mut p = self
                .callee_miss_pending
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *p)
        };
        let samples = stratified_callee_miss_samples(
            pending,
            symbol_index,
            CALLEE_MISS_SAMPLE_TARGET,
            CALLEE_MISS_MIN_FILES,
        );
        let mut out = self
            .callee_miss_samples
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *out = samples;
    }
}

#[async_trait]
impl SemanticResolver for RustLayer2SemanticResolver {
    async fn resolve(&self, hints: &SyntaxResults) -> anyhow::Result<ResolvedEdges> {
        let mut out = ResolvedEdges::new();

        // Rust adapter only handles Rust source. If the pipeline has
        // been driven for a non-Rust language, short-circuit — the
        // factory should not have routed us here, but be defensive.
        let lang = hints.language.as_deref().unwrap_or("");
        if lang != "rust" {
            warn!(
                target: "rust_layer2::resolve",
                "RustLayer2SemanticResolver received non-rust scan (lang={lang:?}); returning empty edges"
            );
            return Ok(out);
        }

        {
            let mut samples = self
                .callee_miss_samples
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            samples.clear();
            let mut pending = self
                .callee_miss_pending
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            pending.clear();
        }

        let symbol_index = SymbolIndex::new(&hints.symbols);
        let mut seen_call_edges = std::collections::HashSet::new();

        for reference in &hints.references {
            if let Some((edge, call_location)) =
                self.resolve_one(reference, &symbol_index, &mut seen_call_edges)
            {
                out.mark_call_site_resolved(call_location);
                out.add_call_edge(edge);
                out.stats.compiler_edges += 1;
            }
        }

        self.finalize_callee_miss_samples(&symbol_index);

        // A1 follow-up: Layer-2 historically emitted only Call edges,
        // so shadow-mode scans of Rust repos came out with `0 import
        // edges` and analyzer fidelity dropped to Low-confidence for
        // dead-code and hotspots. The other languages get their
        // Import edges from `LspResolver::resolve_inner`
        // (see graphengine-parsing/src/infrastructure/lsp/resolver.rs)
        // which always runs both `ImportResolver::resolve_with_heuristics`
        // for symbol-level resolution and
        // `ModuleDependencyResolver::resolve_relative_module_imports`
        // for module→module relative-path resolution. We invoke the
        // same two helpers here so the Rust Layer-2 path produces
        // structurally identical Import edges.
        //
        // We deliberately skip the LSP arm — Layer-2's whole reason
        // for existing is that it doesn't need a subprocess LSP, and
        // `ra_ap_ide` isn't wired to answer go-to-definition for `use`
        // declarations the way `ImportResolver::resolve_with_lsp`
        // expects (separate work). The heuristic path is what other
        // languages also fall back to when their LSP is unavailable,
        // so this is the same correctness envelope.
        let symbol_import_edges = {
            let all_ranges: Vec<Range> = hints
                .import_specs
                .iter()
                .map(|spec| spec.range.clone())
                .collect();
            crate::infrastructure::lsp::resolvers::import_resolver::ImportResolver::resolve_with_heuristics(
                hints,
                &all_ranges,
            )
            .unwrap_or_default()
        };
        let symbol_import_count = symbol_import_edges.len();
        for edge in symbol_import_edges {
            out.add_import_edge(edge);
        }

        // Module-to-module Import edges (relative imports like
        // `use crate::util`). `.rs` is the only extension worth
        // matching for Rust today — Cargo's `mod foo;` declarations
        // resolve to `foo.rs` or `foo/mod.rs`. The helper consults
        // the filesystem only for js/python-style relative
        // specifiers; for absolute Rust paths it falls through to
        // `resolve_intra_project` which does FQN-based matching
        // against `file_module_index`.
        let module_dep_edges =
            crate::infrastructure::lsp::resolvers::module_dependency_resolver::ModuleDependencyResolver::resolve_relative_module_imports(
                hints,
                &[".rs".to_string()],
            );
        let module_dep_count = module_dep_edges.len();
        for edge in module_dep_edges {
            out.add_import_edge(edge);
        }

        let snap = self.snapshot();
        debug_assert!(
            snap.accounting_identity_holds(),
            "Layer-2 resolution accounting identity broken: buckets={} resolutions={}",
            snap.resolution_bucket_sum(),
            snap.adapter_resolved()
        );
        info!(
            target: "rust_layer2::resolve",
            "rust Layer-2 resolve done: call_refs={} high={} medium={} misses={} errors={} edges={} \
             conversion={:.3} mapping_conversion={:.3} identity_ok={} \
             caller_miss={} callee_miss={} external_target={} self_loop={} duplicate={} \
             import_edges_symbol={symbol_import_count} import_edges_module={module_dep_count}",
            snap.call_refs_seen,
            snap.high_resolutions,
            snap.medium_resolutions,
            snap.no_target_misses,
            snap.adapter_errors,
            snap.edges_emitted,
            snap.conversion_rate(),
            snap.mapping_conversion_rate(),
            snap.accounting_identity_holds(),
            snap.symbol_caller_misses,
            snap.symbol_callee_misses,
            snap.external_target_misses,
            snap.self_loop_drops,
            snap.duplicate_edge_drops,
        );
        Ok(out)
    }

    fn supported_language(&self) -> &str {
        "rust"
    }

    async fn is_available(&self) -> bool {
        // Adapter was constructed successfully by `new`, so the
        // `AnalysisHost` is live. A mid-scan adapter failure is not a
        // lifecycle event the port needs to surface — per-query
        // errors bump `adapter_errors` and fall through to heuristic.
        true
    }

    async fn session_metrics(&self) -> Option<SessionMetricsSnapshot> {
        // The Rust Layer-2 adapter is not LSP-subprocess-backed, so
        // it has no `session_metrics` in the LSP sense. Returning
        // `None` keeps the orchestrator's "did LSP actually come up"
        // telemetry cleanly segmented: `lsp_available=true` with
        // `session_metrics=None` is the Layer-2 signature.
        None
    }

    async fn resolution_disclosure(
        &self,
    ) -> Option<crate::application::resolution_disclosure::ResolutionDisclosure> {
        let snap = self.snapshot();
        Some(
            crate::application::resolution_disclosure::ResolutionDisclosure::layer2_active(
                "rust",
                snap.edges_emitted,
            ),
        )
    }

    async fn layer2_telemetry_json(&self) -> Option<String> {
        serde_json::to_string(&self.snapshot()).ok()
    }
}

/// Where to position the rust-analyzer cursor when asking
/// `goto_definition` about a call site.
///
/// - **Plain function call** `foo(x, y)`: use `call_range.start`.
///   `foo` begins at that offset, so `goto_definition` lands on the
///   callee identifier.
/// - **Qualified path call** `a::b::f(x)`: land on the **final path
///   segment** (`f`), not the path start. When the caret sits on a
///   module segment rust-analyzer resolves the module, producing
///   callee misses that look like proc-macro / external-crate loss
///   (P6 / adjudication addendum).
/// - **Method call** `obj.method(x)`: use `(receiver.end_line,
///   receiver.end_char + 1)` — skip the dot, land on `method`. Only
///   safe when `receiver.end_line == call.end_line` (the whole call
///   fits on one line). Multi-line method chains fall through to
///   heuristic; tracked as a known limitation alongside proc-macro
///   in T6 §6.3.
fn caret_for_callee(call_site: &CallSite) -> (u32, u32) {
    crate::infrastructure::semantic::caret::caret_for_callee(call_site)
}

fn callee_path_name(function_name: &str) -> &str {
    crate::infrastructure::semantic::caret::callee_path_name(function_name)
}

/// Final `::` segment of a path (trimmed; no argument list).
fn final_path_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path).trim()
}

fn index_target_from_resolved(target: &ResolvedTarget) -> IndexTarget {
    IndexTarget {
        file: target.target_file.clone(),
        line: target.target_line,
        col: target.target_column,
        symbol_moniker: target.target_symbol_name.clone(),
        confidence: match target.confidence {
            AdapterConfidence::High => Confidence::High,
            AdapterConfidence::Medium => Confidence::Medium,
        },
    }
}

/// Caret on the last path segment for `a::b::f` and multi-line
/// `a::b::\n    f` shapes. `path` is the callee text from tree-sitter
/// (`@func` capture), without the argument list.
#[allow(dead_code)]
fn caret_for_qualified_path(call_site: &CallSite, path: &str) -> (u32, u32) {
    crate::infrastructure::semantic::caret::caret_for_qualified_path(call_site, path)
}

fn format_range(r: &Range) -> String {
    format!("{}:{}:{}", r.file, r.start_line, r.start_char)
}

fn classify_callee_miss<'a>(
    call_site: &CallSite,
    target_file: &std::path::Path,
    target_symbol_name: &str,
    symbol_index: &SymbolIndex<'a>,
) -> &'static str {
    let path = callee_path_name(&call_site.function_name);
    let intended = final_path_segment(path);

    // P7: the call site names a callee that exists in the tree-sitter
    // index — mapping/caret loss, not proc-macro / external crate.
    if symbol_index.function_name_in_index(intended) {
        return "plan01_mappable";
    }

    // Qualified call where the adapter landed on a module / crate-root
    // segment instead of the final function (caret-on-module pattern).
    if path.contains("::") {
        let target_name = target_symbol_name;
        let module_segments: Vec<&str> = path
            .split("::")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if module_segments.len() >= 2 {
            let last = *module_segments.last().unwrap_or(&"");
            for seg in &module_segments[..module_segments.len() - 1] {
                if target_name == *seg || (target_name == "_" && *seg != last) {
                    return "plan01_mappable";
                }
            }
        }
    }

    if symbol_index.target_symbol_in_file(target_file, target_symbol_name) {
        "plan01_mappable"
    } else {
        "plan02_out_of_scope"
    }
}

/// Deterministic xorshift64* for reproducible P7 stratified sampling.
struct MissSampleRng(u64);

impl MissSampleRng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = (self.next_u64() as usize) % (i + 1);
            slice.swap(i, j);
        }
    }
}

fn stratified_callee_miss_samples(
    pending: Vec<CalleeMissRecord>,
    symbol_index: &SymbolIndex<'_>,
    target: usize,
    min_files: usize,
) -> Vec<CalleeMissSample> {
    if pending.is_empty() {
        return Vec::new();
    }
    if pending.len() <= target {
        return pending
            .into_iter()
            .map(|r| record_to_sample(&r, symbol_index))
            .collect();
    }

    let mut by_file: std::collections::HashMap<String, Vec<CalleeMissRecord>> =
        std::collections::HashMap::new();
    for record in pending {
        by_file
            .entry(record.call_site.location.file.clone())
            .or_default()
            .push(record);
    }

    let mut rng = MissSampleRng::new(0x5071_0107);
    let mut file_keys: Vec<String> = by_file.keys().cloned().collect();
    rng.shuffle(&mut file_keys);

    let files_available = file_keys.len();
    let files_to_cover = min_files.min(files_available).max(1);
    let mut selected_files: Vec<String> = file_keys.iter().take(files_to_cover).cloned().collect();
    for f in file_keys {
        if selected_files.len() >= files_to_cover {
            break;
        }
        if !selected_files.contains(&f) {
            selected_files.push(f);
        }
    }

    let base_per_file = target / selected_files.len();
    let mut remainder = target % selected_files.len();
    let mut picked: Vec<CalleeMissRecord> = Vec::with_capacity(target);

    for file in &selected_files {
        let Some(bucket) = by_file.get_mut(file) else {
            continue;
        };
        rng.shuffle(bucket);
        let mut take = base_per_file;
        if remainder > 0 {
            take += 1;
            remainder -= 1;
        }
        take = take.min(bucket.len());
        picked.extend(bucket.drain(..take));
    }

    if picked.len() < target {
        let mut leftovers: Vec<CalleeMissRecord> = by_file
            .into_values()
            .flat_map(std::vec::Vec::into_iter)
            .collect();
        rng.shuffle(&mut leftovers);
        for record in leftovers {
            if picked.len() >= target {
                break;
            }
            picked.push(record);
        }
    }

    picked.truncate(target);
    picked
        .into_iter()
        .map(|r| record_to_sample(&r, symbol_index))
        .collect()
}

fn record_to_sample(record: &CalleeMissRecord, symbol_index: &SymbolIndex<'_>) -> CalleeMissSample {
    let classification = classify_callee_miss(
        &record.call_site,
        &record.target_file,
        &record.target_symbol_name,
        symbol_index,
    );
    CalleeMissSample {
        call_site: format_range(&record.call_site.location),
        target: format!(
            "{}:{} {}",
            record.target_file.display(),
            record.target_line,
            record.target_symbol_name
        ),
        classification: classification.to_string(),
    }
}

/// Is the resolved definition outside the scanned workspace — a
/// cargo-registry crate, the sysroot, or generated code under the
/// workspace's own `target/` build directory? Such targets can never
/// map to an in-repo tree-sitter node, so their misses are counted as
/// `external_target_misses`, not mapping loss. Canonicalises both
/// sides when a plain prefix check fails (macOS `/private/var` vs
/// `/var` symlink skew).
fn is_external_target(target_file: &std::path::Path, workspace_root: &std::path::Path) -> bool {
    let relative = match target_file.strip_prefix(workspace_root) {
        Ok(rel) => Some(rel.to_path_buf()),
        Err(_) => match (target_file.canonicalize(), workspace_root.canonicalize()) {
            (Ok(t), Ok(w)) => t.strip_prefix(&w).ok().map(|rel| rel.to_path_buf()),
            _ => None,
        },
    };
    match relative {
        Some(rel) => rel
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == "target"),
        None => true,
    }
}

#[cfg(test)]
mod caret_tests {
    use super::*;
    use crate::domain::Range as DomainRange;

    fn cs(location: DomainRange, receiver: Option<DomainRange>) -> CallSite {
        CallSite {
            location,
            function_name: "foo".into(),
            receiver_range: receiver,
            receiver_text: None,
            arg_types: Vec::new(),
        }
    }

    #[test]
    fn plain_call_uses_call_start() {
        let loc = DomainRange::with_file(10, 4, 10, 12, "a.rs");
        let c = cs(loc, None);
        assert_eq!(caret_for_callee(&c), (10, 4));
    }

    #[test]
    fn method_call_single_line_steps_past_dot() {
        let call = DomainRange::with_file(7, 4, 7, 20, "a.rs");
        let recv = DomainRange::with_file(7, 4, 7, 10, "a.rs");
        let c = cs(call, Some(recv));
        assert_eq!(caret_for_callee(&c), (7, 11));
    }

    #[test]
    fn method_call_multi_line_falls_back_to_call_start() {
        let call = DomainRange::with_file(5, 4, 7, 20, "a.rs");
        let recv = DomainRange::with_file(5, 4, 6, 10, "a.rs");
        let c = cs(call, Some(recv));
        // Multi-line chain: fall back to call_start so we don't
        // produce a bogus caret in the middle of a newline.
        assert_eq!(caret_for_callee(&c), (5, 4));
    }

    #[test]
    fn qualified_single_line_targets_final_segment() {
        // `    a::b::f(x)` — call starts at `a` (col 4); the prefix
        // `a::b::` is 6 chars, so caret lands on `f` at col 4 + 6 = 10.
        let loc = DomainRange::with_file(10, 4, 10, 18, "a.rs");
        let mut c = cs(loc, None);
        c.function_name = "a::b::f".into();
        assert_eq!(caret_for_callee(&c), (10, 10));
    }

    #[test]
    fn qualified_multi_line_targets_final_segment_line() {
        // `    a::b::` on line 5; `        f(x)` on line 6.
        let loc = DomainRange::with_file(5, 4, 6, 14, "a.rs");
        let mut c = cs(loc, None);
        c.function_name = "a::b::\n        f".into();
        assert_eq!(caret_for_callee(&c), (6, 8));
    }
}

#[cfg(test)]
mod accounting_tests {
    use super::ResolveSnapshot;

    #[test]
    fn resolution_bucket_identity_closes() {
        let snap = ResolveSnapshot {
            high_resolutions: 14_660,
            medium_resolutions: 5,
            edges_emitted: 4_127,
            symbol_caller_misses: 10,
            symbol_callee_misses: 3_045,
            external_target_misses: 1_500,
            self_loop_drops: 4_797,
            duplicate_edge_drops: 1_186,
            ..Default::default()
        };
        assert_eq!(snap.adapter_resolved(), 14_665);
        assert_eq!(snap.resolution_bucket_sum(), 14_665);
        assert!(snap.accounting_identity_holds());
    }

    #[test]
    fn mapping_conversion_excludes_legitimate_drops() {
        let snap = ResolveSnapshot {
            high_resolutions: 100,
            medium_resolutions: 0,
            edges_emitted: 50,
            self_loop_drops: 20,
            duplicate_edge_drops: 5,
            symbol_caller_misses: 10,
            symbol_callee_misses: 15,
            ..Default::default()
        };
        assert!(snap.accounting_identity_holds());
        assert!((snap.mapping_conversion_rate() - 50.0 / 75.0).abs() < 1e-9);
    }

    #[test]
    fn external_targets_are_legitimate_drops_not_mapping_loss() {
        // P9: 100 resolutions, 40 emit, 50 resolve into registry crates,
        // 10 are genuine in-repo mapping loss. Conversion measures the
        // mappable universe only: 40 / (100 - 50) = 0.80.
        let snap = ResolveSnapshot {
            high_resolutions: 100,
            medium_resolutions: 0,
            edges_emitted: 40,
            external_target_misses: 50,
            symbol_callee_misses: 10,
            ..Default::default()
        };
        assert!(snap.accounting_identity_holds());
        assert_eq!(snap.legitimate_drops(), 50);
        assert!((snap.mapping_conversion_rate() - 40.0 / 50.0).abs() < 1e-9);
        // Raw conversion still reports against everything resolved.
        assert!((snap.conversion_rate() - 0.40).abs() < 1e-9);
    }
}

#[cfg(test)]
mod external_target_tests {
    use super::is_external_target;
    use std::path::Path;

    #[test]
    fn registry_path_is_external() {
        assert!(is_external_target(
            Path::new("/Users/x/.cargo/registry/src/index/serde-1.0.0/src/lib.rs"),
            Path::new("/Users/x/repo"),
        ));
    }

    #[test]
    fn workspace_source_is_internal() {
        assert!(!is_external_target(
            Path::new("/Users/x/repo/crate-a/src/lib.rs"),
            Path::new("/Users/x/repo"),
        ));
    }

    #[test]
    fn workspace_target_dir_is_external() {
        assert!(is_external_target(
            Path::new("/Users/x/repo/target/debug/build/gen/out.rs"),
            Path::new("/Users/x/repo"),
        ));
    }

    #[test]
    fn crate_named_target_prefix_is_internal() {
        // `target-utils/` is a source dir, not the build dir.
        assert!(!is_external_target(
            Path::new("/Users/x/repo/target-utils/src/lib.rs"),
            Path::new("/Users/x/repo"),
        ));
    }
}
