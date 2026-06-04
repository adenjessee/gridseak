//! Rust Layer-2 semantic resolver.
//!
//! Wraps [`graphengine_ra_ide_adapter::RustAnalyzerSemanticResolver`]
//! and implements the engine's [`SemanticResolver`] port so the
//! existing pipeline (`ParsingPipeline::execute` →
//! `SemanticResolverService::resolve_with_fallback`) can consume it
//! transparently. Every successfully-resolved call site produces a
//! single `Edge { kind: Call, provenance: Lsp / High|Medium }`
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
//! see the effect.
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
    CallSite, ResolvedEdges, SemanticResolver, SessionMetricsSnapshot, SyntaxResults,
    UnresolvedReference,
};
use crate::domain::{
    Confidence, Edge, EdgeKind, Node, NodeKind, Provenance, ProvenanceSource, Range,
};

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
        }
    }
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
}

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
        self.counters.as_snapshot()
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

        let caller = symbol_index.find_enclosing_function(&call_site.location)?;
        let callee = symbol_index.find_callee_for(&target)?;

        if caller.id == callee.id {
            // ra_ap_ide resolves a recursive call inside a function
            // back to the function's own definition. The domain-level
            // `Edge::new` validator rejects self-loops for
            // `EdgeKind::Call` (panics in debug, warns-and-drops in
            // release), and the project-wide graph convention is that
            // recursion is a per-node attribute, not a call edge (see
            // `test_e2e_rust_parsing`'s self-loop commentary and the
            // `validate_edge` contract in `domain::edge`). Drop the
            // edge at the adapter; the fallback will still skip this
            // call site because its `function_name` will no longer
            // have a qualifying candidate other than itself.
            debug!(
                target: "rust_layer2::resolve",
                "self-recursion at {} → {}; dropping edge (validator rejects Call self-loops)",
                caller.fqn, callee.fqn
            );
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
            Provenance::new(ProvenanceSource::Lsp, confidence),
        );
        self.counters.edges_emitted.fetch_add(1, Ordering::Relaxed);
        Some((edge, call_site.location.clone()))
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

        let symbol_index = SymbolIndex::new(&hints.symbols);

        for reference in &hints.references {
            if let Some((edge, call_location)) = self.resolve_one(reference, &symbol_index) {
                out.mark_call_site_resolved(call_location);
                out.add_call_edge(edge);
                out.stats.lsp_edges += 1;
            }
        }

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

        let snap = self.counters.as_snapshot();
        info!(
            target: "rust_layer2::resolve",
            "rust Layer-2 resolve done: call_refs={} high={} medium={} misses={} errors={} edges={} \
             import_edges_symbol={symbol_import_count} import_edges_module={module_dep_count}",
            snap.call_refs_seen,
            snap.high_resolutions,
            snap.medium_resolutions,
            snap.no_target_misses,
            snap.adapter_errors,
            snap.edges_emitted,
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
}

/// Where to position the rust-analyzer cursor when asking
/// `goto_definition` about a call site.
///
/// - **Plain function call** `foo(x, y)`: use `call_range.start`.
///   `foo` begins at that offset, so `goto_definition` lands on the
///   callee identifier.
/// - **Method call** `obj.method(x)`: use `(receiver.end_line,
///   receiver.end_char + 1)` — skip the dot, land on `method`. Only
///   safe when `receiver.end_line == call.end_line` (the whole call
///   fits on one line). Multi-line method chains fall through to
///   heuristic; tracked as a known limitation alongside proc-macro
///   in T6 §6.3.
fn caret_for_callee(call_site: &CallSite) -> (u32, u32) {
    if let Some(recv) = &call_site.receiver_range {
        if recv.end_line == call_site.location.end_line && recv.end_line == recv.start_line {
            return (recv.end_line, recv.end_char.saturating_add(1));
        }
    }
    (call_site.location.start_line, call_site.location.start_char)
}

/// Caller / callee index over the scan's flat `Vec<Node>`. Rebuilt
/// once per `resolve()` call because the symbol list is already in
/// memory and the indexing cost is dwarfed by the per-call
/// adapter-dispatch cost.
struct SymbolIndex<'a> {
    /// `file path → [symbols in that file, sorted by start_line]`.
    by_file: std::collections::HashMap<&'a str, Vec<&'a Node>>,
}

impl<'a> SymbolIndex<'a> {
    fn new(symbols: &'a [Node]) -> Self {
        let mut by_file: std::collections::HashMap<&'a str, Vec<&'a Node>> =
            std::collections::HashMap::new();
        for node in symbols {
            by_file
                .entry(node.location.file.as_str())
                .or_default()
                .push(node);
        }
        for v in by_file.values_mut() {
            v.sort_by_key(|n| (n.location.start_line, n.location.start_char));
        }
        Self { by_file }
    }

    /// Tightest `Function`-kind node whose range contains `location`.
    /// "Tightest" = smallest containing range, i.e. the innermost
    /// function when nested closures / fn-in-fn appear.
    fn find_enclosing_function(&self, location: &Range) -> Option<&'a Node> {
        let candidates = self.by_file.get(location.file.as_str())?;
        candidates
            .iter()
            .copied()
            .filter(|n| n.kind == NodeKind::Function && contains(&n.location, location))
            .min_by_key(|n| range_span(&n.location))
    }

    /// Node whose `(file, kind, range)` best matches a
    /// [`ResolvedTarget`]. Match shape:
    /// 1. Same file as `target.target_file` (absolute path equality
    ///    after filesystem-neutral normalisation — fall back to
    ///    string equality if canonicalisation fails).
    /// 2. `target.target_line` falls within the node's range.
    /// 3. Among nodes matching 1+2, prefer exact-name matches against
    ///    `target.target_symbol_name` (suffix-of-fqn match).
    /// 4. Kind is `Function` — ra_ap_ide resolves call targets to
    ///    function-kind items by definition.
    fn find_callee_for(&self, target: &ResolvedTarget) -> Option<&'a Node> {
        let target_path = target.target_file.to_string_lossy();
        // Try the stored file path first; if the extractor stored a
        // relative path we still succeed because `ra_ap_ide` also
        // returns absolute-style VFS paths that begin with the
        // workspace root — a suffix match catches both shapes.
        let (_file_key, file_symbols) = self
            .by_file
            .iter()
            .find(|(k, _)| path_matches(k, target_path.as_ref()))
            .or_else(|| {
                self.by_file
                    .iter()
                    .find(|(k, _)| target_path.ends_with(**k))
            })?;

        // Step 1: narrow by range containment on target_line.
        let line_hits: Vec<&Node> = file_symbols
            .iter()
            .copied()
            .filter(|n| n.kind == NodeKind::Function)
            .filter(|n| {
                n.location.start_line <= target.target_line
                    && target.target_line <= n.location.end_line
            })
            .collect();

        if line_hits.is_empty() {
            return None;
        }

        // Step 2: among range-matches, prefer exact suffix match on fqn.
        let name = target.target_symbol_name.as_str();
        if let Some(exact) = line_hits.iter().find(|n| fqn_ends_with(&n.fqn, name)) {
            return Some(*exact);
        }

        // Step 3: fall back to the single line-containing function
        // (there is often exactly one — the function whose header
        // line is `target_line`). If multiple, refuse to guess —
        // the heuristic resolver will handle the ambiguity with its
        // own trait-candidate filter.
        if line_hits.len() == 1 {
            return Some(line_hits[0]);
        }
        None
    }
}

/// Does `outer` wrap `inner` on a byte-range basis, using
/// (line, char) tuples for comparison? Lines are 1-based and
/// char-offsets are 0-based per the `Range` contract in
/// `graphengine-parsing/src/domain/node.rs`.
fn contains(outer: &Range, inner: &Range) -> bool {
    if outer.file != inner.file {
        return false;
    }
    let outer_start = (outer.start_line, outer.start_char);
    let outer_end = (outer.end_line, outer.end_char);
    let inner_start = (inner.start_line, inner.start_char);
    let inner_end = (inner.end_line, inner.end_char);
    outer_start <= inner_start && inner_end <= outer_end
}

/// Rough "how many bytes / lines does this range span" metric used
/// only to pick the tightest enclosing function.
fn range_span(range: &Range) -> (u32, u32) {
    let line_span = range.end_line.saturating_sub(range.start_line);
    let col_span = if range.end_line == range.start_line {
        range.end_char.saturating_sub(range.start_char)
    } else {
        0
    };
    (line_span, col_span)
}

/// Loose path equality. Callers pass target paths coming from
/// `ra_ap_ide` (absolute, canonical) and symbol paths coming from
/// the tree-sitter extractor (often workspace-relative). We accept
/// an exact match, a suffix match in either direction, or a
/// canonicalised equality. Anything ambiguous falls through to the
/// heuristic, so false-positives here just waste work; they do not
/// poison edges.
fn path_matches(symbol_path: &str, target_path: &str) -> bool {
    if symbol_path == target_path {
        return true;
    }
    if target_path.ends_with(symbol_path) || symbol_path.ends_with(target_path) {
        return true;
    }
    match (
        std::path::Path::new(symbol_path).canonicalize().ok(),
        std::path::Path::new(target_path).canonicalize().ok(),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Does `fqn` end with `name` as a whole FQN segment? FQNs use
/// language-specific separators (`::`, `.`, `#`); we accept any
/// non-alphanumeric boundary so a match on `module::helper` against
/// symbol name `helper` succeeds.
fn fqn_ends_with(fqn: &str, name: &str) -> bool {
    if fqn == name {
        return true;
    }
    if !fqn.ends_with(name) {
        return false;
    }
    // Boundary check: the char immediately before `name` must not be
    // alphanumeric (to avoid matching `my_helper` when searching for
    // `helper`).
    let Some(prefix_end) = fqn.len().checked_sub(name.len()) else {
        return false;
    };
    !matches!(
        fqn[..prefix_end].chars().last(),
        Some(c) if c.is_alphanumeric() || c == '_'
    )
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
}

#[cfg(test)]
mod fqn_tests {
    use super::*;

    #[test]
    fn suffix_name_with_colon_boundary_matches() {
        assert!(fqn_ends_with("crate::module::helper", "helper"));
    }

    #[test]
    fn partial_name_match_rejected() {
        assert!(!fqn_ends_with("crate::module::my_helper", "helper"));
    }

    #[test]
    fn exact_match() {
        assert!(fqn_ends_with("helper", "helper"));
    }

    #[test]
    fn empty_name_rejected_for_non_equal() {
        // fqn != name, fqn ends_with "" (always true), so boundary check kicks in
        // but name "" has zero length -> prefix_end == fqn.len(), prefix.chars().last() is 'r'
        // which is alphanumeric -> rejected.
        assert!(!fqn_ends_with("helper", ""));
    }
}

#[cfg(test)]
mod symbol_index_tests {
    use super::*;
    use crate::domain::{Node, NodeKind, Provenance, Range as DomainRange};

    fn fn_node(fqn: &str, file: &str, range: DomainRange) -> Node {
        Node::new(
            NodeKind::Function,
            fqn.to_string(),
            range.with_file_path(file),
            Provenance::tree_sitter(),
        )
    }

    #[test]
    fn enclosing_function_picks_tightest() {
        let outer = fn_node(
            "crate::outer",
            "a.rs",
            DomainRange::with_file(1, 0, 30, 0, "a.rs"),
        );
        let inner = fn_node(
            "crate::outer::inner",
            "a.rs",
            DomainRange::with_file(10, 4, 20, 4, "a.rs"),
        );
        let symbols = vec![outer.clone(), inner.clone()];
        let idx = SymbolIndex::new(&symbols);
        let site = DomainRange::with_file(15, 8, 15, 12, "a.rs");
        let got = idx.find_enclosing_function(&site).expect("enclosing");
        assert_eq!(got.fqn, inner.fqn);
    }

    #[test]
    fn callee_lookup_matches_by_line_and_name() {
        let sym = fn_node(
            "crate::target::callee",
            "b.rs",
            DomainRange::with_file(12, 0, 18, 0, "b.rs"),
        );
        let symbols = vec![sym.clone()];
        let idx = SymbolIndex::new(&symbols);
        let target = ResolvedTarget {
            target_file: std::path::PathBuf::from("b.rs"),
            target_line: 12,
            target_column: 3,
            target_symbol_name: "callee".into(),
            confidence: graphengine_ra_ide_adapter::Confidence::High,
        };
        let got = idx.find_callee_for(&target).expect("callee");
        assert_eq!(got.id, sym.id);
    }

    #[test]
    fn callee_lookup_prefers_exact_name_when_multiple_match_line() {
        // Two functions on overlapping lines — pick the one whose fqn
        // matches `target_symbol_name`.
        let a = fn_node(
            "crate::helpers::callee",
            "c.rs",
            DomainRange::with_file(20, 0, 30, 0, "c.rs"),
        );
        let b = fn_node(
            "crate::helpers::other",
            "c.rs",
            DomainRange::with_file(20, 40, 30, 50, "c.rs"),
        );
        let symbols = vec![a.clone(), b.clone()];
        let idx = SymbolIndex::new(&symbols);
        let target = ResolvedTarget {
            target_file: std::path::PathBuf::from("c.rs"),
            target_line: 25,
            target_column: 0,
            target_symbol_name: "callee".into(),
            confidence: graphengine_ra_ide_adapter::Confidence::High,
        };
        let got = idx.find_callee_for(&target).expect("callee");
        assert_eq!(got.id, a.id);
    }
}

// Shim: `Range::with_file_path` doesn't exist on the domain type; we
// provide a local helper so the test-only `fn_node` above stays
// readable without touching the domain crate. Promoting this into
// `graphengine-parsing::domain::node` is a cosmetic cleanup that
// would churn unrelated call sites; scoped to tests it stays inline.
#[cfg(test)]
trait WithFilePath: Sized {
    fn with_file_path(self, file: &str) -> Self;
}
#[cfg(test)]
impl WithFilePath for crate::domain::Range {
    fn with_file_path(mut self, file: &str) -> Self {
        self.file = file.to_string();
        self
    }
}
