//! Pipeline orchestrator for the complete parsing workflow

use super::super::super::super::errors::ParsingError;
use super::super::super::super::ports::{
    GraphRepository, ResolutionStatsSummary, SemanticResolver, SyntaxExtractor,
};
use super::super::use_case::ResolvedGraph;
use super::config::ConfigLoader;
use super::file_discovery::FileDiscovery;
use super::file_hashing;
use super::graph_building::GraphBuilder;
use super::incremental::{compute_plan, IncrementalPlan};
use super::per_file_slicer::{
    reconstitute_from_slices, slice_per_file, PerFileSlice, ScanMetadata, ORPHAN_FILE_KEY,
};
use super::persistence::GraphPersistence;
use super::semantic_resolution::SemanticResolverService;
use super::symbol_table::SymbolTableBuilder;
use super::syntax_extraction::SyntaxExtraction;
use crate::domain::Graph;
use crate::infrastructure::storage::FileCacheRow;
use crate::syntax::language::extractor::HookOutcome;
use graphengine_progress::{EngineEvent, EngineEventEmitter};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, info, warn};

/// Per-scan options the orchestrator consumes. Currently carries the
/// incremental-scan toggle; designed to grow further options without
/// breaking the `execute_with_progress` signature again.
///
/// `Default::default()` returns `incremental: true` — the orchestrator's
/// happy path. Callers that want a cold reparse (`gridseak scan
/// --no-incremental`) construct `ParseOptions { incremental: false }`.
#[derive(Debug, Clone, Copy)]
pub struct ParseOptions {
    /// When `true`, the orchestrator hashes discovered files and reuses
    /// cached extraction slices for files whose hash matches the prior
    /// scan. When `false`, every file is re-extracted and the cache is
    /// rewritten from scratch.
    pub incremental: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self { incremental: true }
    }
}

/// Parsing pipeline orchestrator
pub struct ParsingPipeline;

impl ParsingPipeline {
    /// Execute the complete parsing pipeline
    ///
    /// # Arguments
    /// * `root` - Root directory of the repository
    /// * `language` - Programming language to parse
    /// * `syntax_extractor` - Syntax extractor implementation
    /// * `semantic_resolver` - Semantic resolver implementation
    /// * `graph_repo` - Graph repository implementation
    /// * `min_confidence` - Minimum confidence level for validation
    ///
    /// # Returns
    /// * `ResolvedGraph` - Validated semantic graph
    /// * `ParsingError` - If parsing fails at any stage
    pub async fn execute(
        root: std::path::PathBuf,
        language: String,
        syntax_extractor: &dyn SyntaxExtractor,
        semantic_resolver: &dyn SemanticResolver,
        graph_repo: &dyn GraphRepository,
        min_confidence: crate::domain::Confidence,
    ) -> Result<ResolvedGraph, ParsingError> {
        Self::execute_with_progress(
            root,
            language,
            syntax_extractor,
            semantic_resolver,
            graph_repo,
            min_confidence,
            None,
            ParseOptions::default(),
        )
        .await
    }

    /// Execute the complete parsing pipeline with optional progress reporting.
    ///
    /// Eight parameters because the pipeline is the seam between several
    /// independent component crates (`SyntaxExtractor`, `SemanticResolver`,
    /// `GraphRepository`, the progress emitter) and the per-scan inputs
    /// (`root`, `language`, `min_confidence`, `options`). Bundling them
    /// into a config struct would invert the dependency direction —
    /// callers would need to construct that struct to pass to the
    /// orchestrator — and obscure that each port is a distinct trait
    /// object. Allow the clippy lint here rather than hide the shape.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_with_progress(
        root: std::path::PathBuf,
        language: String,
        syntax_extractor: &dyn SyntaxExtractor,
        semantic_resolver: &dyn SemanticResolver,
        graph_repo: &dyn GraphRepository,
        min_confidence: crate::domain::Confidence,
        emitter: Option<Arc<dyn EngineEventEmitter>>,
        options: ParseOptions,
    ) -> Result<ResolvedGraph, ParsingError> {
        info!("Starting parse for {} in {}", language, root.display());

        // Phase/status are now wire-format strings rather than typed
        // enums (R3). The previous `ProgressPhase`/`ProgressStatus`
        // enums serialized via `rename_all = "lowercase"`, so the
        // string literals below are byte-identical to what went over
        // the wire pre-migration — a consumer rebuilt against the new
        // graphengine-progress crate sees no schema break.
        let emit = |percent: u8, phase: &str, status: &str, message: &str| {
            if let Some(ref e) = emitter {
                let _ = e.emit(EngineEvent::progress(percent, phase, status, message));
            }
        };

        // Step 1: Load configuration
        Self::execute_step("load_config", || {
            ConfigLoader::load_language_config(&language)
        })?;

        // Step 2: Discover source files
        emit(16, "discovery", "start", "Discovering source files");
        let files = Self::execute_step("discover_files", || {
            FileDiscovery::discover_source_files(&root, &language, syntax_extractor)
        })?;
        info!("Discovered {} files", files.len());
        emit(
            20,
            "discovery",
            "done",
            &format!("Discovered {} source files", files.len()),
        );

        if files.is_empty() {
            warn!("No source files found for language: {}", language);
            return Ok(ResolvedGraph::new(
                Graph::new(),
                ResolutionStatsSummary::new(),
            ));
        }

        // S1: incremental-scan plan. Hash every discovered file, load
        // the prior file_cache, partition files into changed /
        // unchanged. The actual extraction below runs on the changed
        // set only; cached slices for unchanged files are reloaded
        // and merged before resolution. On `--no-incremental` (or any
        // failure in the hash / cache load path) the orchestrator
        // falls back to a full reparse and emits a disabled-cache
        // stats event so renderers can surface the bypass to the user.
        let (plan, cached_slices, plan_disabled) =
            Self::compute_incremental_plan(options.incremental, &files, &language, graph_repo)
                .await;
        let cache_hits = plan.unchanged.len();
        let cache_misses = plan.changed.len();
        let cache_removed = plan.removed_paths.len();
        if let Some(ref e) = emitter {
            let event = if plan_disabled {
                EngineEvent::cache_stats_disabled(files.len())
            } else {
                EngineEvent::cache_stats(cache_hits, cache_misses, cache_removed)
            };
            let _ = e.emit(event);
        }
        info!(
            "incremental plan: {} discovered, {} cached, {} changed, {} removed (disabled={})",
            files.len(),
            cache_hits,
            cache_misses,
            cache_removed,
            plan_disabled
        );

        // Step 2.5 (S1-ε): pre-extraction row pruning.
        //
        // The parser's UPSERT path is keyed on `node.id` (a SHA256 of
        // FQN + body), so editing a function body produces a DIFFERENT
        // node id from the one already in the DB — without an explicit
        // delete, the old row would survive the rescan. Same story for
        // edges whose endpoints moved. The right time to do the
        // deletion is here, BEFORE we re-extract the changed files,
        // because:
        //
        //   1. After this point the orchestrator persists fresh slices
        //      via `graph_repo.upsert(...)` near end-of-scan. UPSERT
        //      overwrites by primary key but cannot tell that an old
        //      id-X row is now "orphaned" because the file produces
        //      id-Y instead.
        //   2. `ON DELETE CASCADE` on `edges.from_id` / `edges.to_id`
        //      means deleting a node automatically removes incident
        //      edges. We do not need a separate edge-delete pass.
        //   3. `file_extraction_coverage` is keyed on `file_path` —
        //      the prune method drops those rows too so the analyzer
        //      doesn't see a coverage record without its underlying
        //      nodes.
        //
        // Files in `plan.removed_paths` are also pruned: they no
        // longer exist on disk, so any rows attributed to them are
        // dead weight.
        //
        // `plan_disabled` (the `--no-incremental` opt-out) still runs
        // the planner for `removed_paths` and forces every discovered
        // file into `changed`, so this block executes for both modes.
        if !plan.changed.is_empty() || !plan.removed_paths.is_empty() {
            let mut paths_to_prune: Vec<String> = plan
                .changed
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            paths_to_prune.extend(plan.removed_paths.iter().cloned());
            match graph_repo.prune_files_from_graph(&paths_to_prune).await {
                Ok(deleted) => {
                    info!(
                        "S1-\u{03b5} pre-extract prune: removed {} stale node row(s) for {} file path(s) ({} changed + {} removed)",
                        deleted,
                        paths_to_prune.len(),
                        plan.changed.len(),
                        plan.removed_paths.len()
                    );
                }
                Err(e) => {
                    // Pre-extract prune is a best-effort cleanup. A
                    // failure here means some stale rows MAY survive
                    // the rescan, but the analyzer's structural
                    // contracts already tolerate orphan nodes (the
                    // tier-signal layer reports the resolution gap),
                    // so we log and continue rather than fail the
                    // entire scan. A persistent failure would show up
                    // as a gradual fan-in inflation over many rescans;
                    // a future audit-mode could verify the prune
                    // happened.
                    warn!("S1-\u{03b5} pre-extract prune failed (continuing): {e}");
                }
            }
        }

        // Step 3: Extract syntax information (changed files only).
        let extract_targets: &[PathBuf] = if plan_disabled {
            // Cold reparse: pass every file to the extractor.
            &files
        } else {
            &plan.changed
        };
        emit(
            21,
            "syntax",
            "start",
            &format!(
                "Extracting syntax from {} files ({} cached)",
                extract_targets.len(),
                cache_hits
            ),
        );
        let t_extraction = std::time::Instant::now();
        let mut syntax_results = Self::execute_step_async("extract_syntax", || {
            SyntaxExtraction::extract_from_files(extract_targets, syntax_extractor)
        })
        .await?;
        syntax_results.set_workspace_root(root.to_string_lossy().to_string());
        syntax_results.set_language(language.clone());

        // S1: capture the slice keys of files we just extracted *before*
        // post-syntax hooks mutate anything. The cache stores the raw
        // extraction output (so hooks can re-run correctly on next
        // rescan against a possibly different merged set); the slice
        // keys here are reused at end-of-scan to know which file_cache
        // rows must be rewritten.
        let (fresh_slices, scan_meta) = if plan_disabled {
            // Cold reparse: every file is "changed".
            slice_per_file(&syntax_results)
        } else {
            slice_per_file(&syntax_results)
        };

        // Merge cached slices into the freshly-extracted aggregate.
        // The reconstitute step rebuilds a `SyntaxResults` containing
        // every file's items so resolution / graph-building runs
        // end-to-end on the full set, not just the changed files.
        if !cached_slices.is_empty() {
            let merged = Self::merge_cached_into_extraction(
                syntax_results,
                &fresh_slices,
                &cached_slices,
                &scan_meta,
            );
            syntax_results = merged;
            info!(
                "merged {} cached file slices with {} freshly extracted slices",
                cached_slices.len(),
                fresh_slices.len()
            );
        }
        info!(
            "[TIMING] Syntax extraction ({} files): {:?}",
            files.len(),
            t_extraction.elapsed()
        );
        info!(
            "Extracted {} symbols, {} unresolved references, {} imports, {} type refs",
            syntax_results.symbols.len(),
            syntax_results.references.len(),
            syntax_results.imports.len(),
            syntax_results.type_refs.len()
        );
        emit(
            55,
            "syntax",
            "done",
            &format!(
                "Extracted {} symbols, {} unresolved references",
                syntax_results.symbols.len(),
                syntax_results.references.len()
            ),
        );

        // Step 3.5: language-specific post-syntax hooks. Dispatched
        // through the `SyntaxExtractor` port so the orchestrator stays
        // language-agnostic — previously this site hardcoded Apex VF
        // extraction (TR-A.5) and Apex framework entry-point propagation
        // (Round 5 R11 fix) by name; after T5 each language declares
        // its own post-syntax stages behind the port and adding a new
        // language does not grow this file. See
        // `docs/workstreams/universal-fidelity/tasks/T5-orchestrator-collapse.md`.
        match syntax_extractor.post_syntax_hooks(&root, &mut syntax_results) {
            HookOutcome::NoOp => {}
            HookOutcome::Ok { summary: Some(s) } => info!("post-syntax hooks: {s}"),
            HookOutcome::Ok { summary: None } => {}
            HookOutcome::Warning { message } => {
                warn!("post-syntax hooks: {message}")
            }
        }

        // S2-γ parse fast path: when every file in this language pass is
        // cache-hit and nothing was removed, the graph rows for this
        // language are already in the persistent DB — skip resolution and
        // graph rebuild (still O(minutes) on large repos otherwise).
        if !plan_disabled
            && plan.is_fully_cached()
            && plan.removed_paths.is_empty()
            && extract_targets.is_empty()
        {
            info!(
                "S2-γ: fully cached language pass for {} ({} files) — skipping resolution and graph rebuild",
                language,
                cache_hits
            );
            emit(
                75,
                "resolution",
                "done",
                &format!("Skipped resolution ({} files cached)", cache_hits),
            );
            emit(
                82,
                "graph",
                "done",
                &format!("Skipped graph rebuild ({} files cached)", cache_hits),
            );
            Self::write_file_cache(
                graph_repo,
                &fresh_slices,
                &language,
                &files,
                plan_disabled,
                cache_hits,
            )
            .await?;
            let scan_stats =
                crate::infrastructure::storage::parse_meta_store::IncrementalScanStats {
                    cached: cache_hits,
                    reparsed: cache_misses,
                    removed: cache_removed,
                    plan_disabled,
                    changed_paths: Vec::new(),
                    removed_paths: plan.removed_paths.clone(),
                };
            if let Err(err) = graph_repo.write_incremental_scan_stats(&scan_stats).await {
                warn!(
                    "S2: incremental_scan_stats persistence failed ({}); analysis fast-path disabled this scan",
                    err
                );
            }
            return Ok(ResolvedGraph::new(
                Graph::new(),
                ResolutionStatsSummary::new(),
            ));
        }

        // Step 4: Build global symbol table for cross-file resolution
        emit(
            56,
            "resolution",
            "start",
            "Building symbol table and resolving semantics",
        );
        let global_symbol_table = Self::execute_step("build_global_symbol_table", || {
            SymbolTableBuilder::build_from_syntax_results(&syntax_results)
        })?;
        info!(
            "Built global symbol table with {} symbols across {} files",
            global_symbol_table.symbols_by_name.len(),
            global_symbol_table.symbols_by_file.len()
        );

        // Step 5: Resolve semantic relationships with global context
        let lsp_available = semantic_resolver.is_available().await;
        info!("LSP available for {}: {}", language, lsp_available);

        let t_resolution = std::time::Instant::now();
        let resolved_edges = Self::execute_step_async("resolve_semantics", || {
            SemanticResolverService::resolve_with_fallback(
                &syntax_results,
                &global_symbol_table,
                semantic_resolver,
            )
        })
        .await?;
        info!(
            "[TIMING] Semantic resolution (total): {:?}",
            t_resolution.elapsed()
        );
        info!(
            "Resolved {} call edges, {} import edges, {} type edges, {} containment edges",
            resolved_edges.call_edges.len(),
            resolved_edges.import_edges.len(),
            resolved_edges.type_edges.len(),
            resolved_edges.containment_edges.len()
        );
        emit(
            75,
            "resolution",
            "done",
            &format!(
                "Resolved {} edges total",
                resolved_edges.call_edges.len()
                    + resolved_edges.import_edges.len()
                    + resolved_edges.type_edges.len()
                    + resolved_edges.containment_edges.len()
            ),
        );

        // Step 6: Build and validate the graph
        emit(76, "graph", "start", "Building graph");
        let stats = resolved_edges.stats.clone();
        // TR-A.0: drain class-symbols off the results before they are
        // consumed by `GraphBuilder`. The payload is persisted via
        // a dedicated repository call after the graph is saved, so
        // the graph-building path sees no class-symbols-derived
        // nodes or edges — which keeps the rev-6.1 byte-identical
        // regression gate clean (the feature is invisible to the
        // analysis output until the resolver consumes the table in
        // PRs 2–5).
        let class_symbols_payload: Vec<(String, String)> =
            std::mem::take(&mut syntax_results.class_symbols);
        // T8: pull the per-file extraction-coverage records out of
        // `SyntaxResults` before the graph-builder drops it.
        // Persisted below, after the graph is written, so that a
        // failure in the coverage-table write cannot abort the
        // graph transaction.
        let extraction_coverage_payload: Vec<crate::application::ports::FileExtractionCoverage> =
            std::mem::take(&mut syntax_results.extraction_coverage);
        let t_graph_build = std::time::Instant::now();
        let mut graph = Self::execute_step("build_graph", || {
            GraphBuilder::build_from_results(syntax_results, resolved_edges, min_confidence)
        })?;
        graph
            .metadata
            .insert("lsp_available".into(), lsp_available.to_string());
        graph.metadata.insert("language".into(), language.clone());

        // Persist resolution telemetry so `ge-analyze` can emit the
        // `ResolutionDegraded` finding (Sprint D.2) without forcing
        // analysis to re-run the parser. Individual scalar keys instead
        // of a JSON blob so that:
        //   * the analysis-side `read_metadata` helper (which returns
        //     `Option<String>`) stays the lowest-common-denominator API,
        //   * a missing field defaults cleanly to 0 rather than blocking
        //     the whole finding on a schema mismatch,
        //   * sqlite pragma queries can expose individual counters if
        //     ops ever needs to debug without deserializing json.
        graph
            .metadata
            .insert("resolution_lsp_edges".into(), stats.lsp_edges.to_string());
        graph.metadata.insert(
            "resolution_heuristic_edges".into(),
            stats.heuristic_edges.to_string(),
        );
        graph.metadata.insert(
            "resolution_heuristic_call_fallbacks".into(),
            stats.heuristic_call_fallbacks.to_string(),
        );
        graph.metadata.insert(
            "resolution_heuristic_import_fallbacks".into(),
            stats.heuristic_import_fallbacks.to_string(),
        );
        graph.metadata.insert(
            "resolution_heuristic_type_fallbacks".into(),
            stats.heuristic_type_fallbacks.to_string(),
        );
        graph.metadata.insert(
            "resolution_heuristic_call_ambiguous_drops".into(),
            stats.heuristic_call_ambiguous_drops.to_string(),
        );

        // Sprint D.4 completion: also persist LSP session-lifecycle
        // metrics alongside the resolution stats. The CLI's
        // `--lsp-telemetry` JSON uses `resolved_graph.session_metrics()`
        // directly, but having the same keys in graph metadata gives
        // `ge-analyze` and any ad-hoc SQL consumer the same view
        // without a second channel to reconcile.
        let session_metrics_snapshot = semantic_resolver.session_metrics().await;
        if let Some(ref m) = session_metrics_snapshot {
            graph.metadata.insert(
                "session_start_attempts".into(),
                m.start_attempts.to_string(),
            );
            graph.metadata.insert(
                "session_successful_starts".into(),
                m.successful_starts.to_string(),
            );
            graph
                .metadata
                .insert("session_failed_starts".into(), m.failed_starts.to_string());
            if let Some(err) = m.last_error.as_ref() {
                // SQLite metadata values are short UTF-8 strings; a
                // multi-KB LSP stack trace here would be wasteful. Cap
                // at a generous 2 KiB so callers still get enough to
                // triage without bloating every scan's metadata table.
                let truncated = if err.len() > 2048 {
                    format!("{}…[truncated]", &err[..2048])
                } else {
                    err.clone()
                };
                graph
                    .metadata
                    .insert("session_last_error".into(), truncated);
            }
        }
        info!(
            "[TIMING] Graph build ({} nodes, {} edges): {:?}",
            graph.node_count(),
            graph.edge_count(),
            t_graph_build.elapsed()
        );
        emit(
            82,
            "graph",
            "done",
            &format!(
                "Built graph: {} nodes, {} edges",
                graph.node_count(),
                graph.edge_count()
            ),
        );

        // Step 7: Persist the graph
        emit(83, "db", "start", "Persisting graph to database");
        let t_persist = std::time::Instant::now();
        Self::execute_step_async("persist_graph", || {
            GraphPersistence::persist_to_repository(&graph, graph_repo)
        })
        .await?;
        info!(
            "[TIMING] Persistence ({} nodes, {} edges): {:?}",
            graph.node_count(),
            graph.edge_count(),
            t_persist.elapsed()
        );
        emit(89, "db", "done", "Graph persisted");

        // TR-A.0: persist the Apex class-symbols payload. Runs after
        // graph persist so a class-symbols write failure cannot
        // abort the main graph transaction (the graph is the source
        // of truth; the symbols table is a downstream consumer).
        // Non-SQLite repositories default to a no-op, so mocks and
        // in-memory dev backends aren't burdened.
        if !class_symbols_payload.is_empty() {
            let row_count = class_symbols_payload.len();
            Self::execute_step_async("persist_apex_class_symbols", || async {
                graph_repo
                    .upsert_apex_class_symbols(&class_symbols_payload)
                    .await
                    .map_err(|e| ParsingError::Repository(e.to_string()))
            })
            .await?;
            info!(
                "[TIMING] apex_class_symbols persistence ({} rows)",
                row_count
            );
        }

        // T8 (universal-fidelity sprint). Same deferred-persistence
        // discipline as `apex_class_symbols`: the graph write is the
        // canonical artefact; coverage is a downstream consumer
        // that can fail without aborting graph persistence. A
        // pre-T8 trait impl defaults to the no-op stub, so
        // non-SQLite backends are unaffected.
        if !extraction_coverage_payload.is_empty() {
            let row_count = extraction_coverage_payload.len();
            Self::execute_step_async("persist_file_extraction_coverage", || async {
                graph_repo
                    .upsert_file_extraction_coverage(&extraction_coverage_payload)
                    .await
                    .map_err(|e| ParsingError::Repository(e.to_string()))
            })
            .await?;
            info!(
                "[TIMING] file_extraction_coverage persistence ({} rows)",
                row_count
            );
        }

        // S1 cache write-back. Persist a `file_cache` row for every
        // file we actually extracted this scan (`fresh_slices.keys()`),
        // then prune cache rows whose files no longer exist on disk.
        // We do NOT rewrite rows for cache-hit files — their existing
        // rows are still byte-identical to the freshly-merged slice,
        // and skipping them avoids unnecessary SQLite writes on every
        // scan of a quiet repo. Mismatches happen only when the
        // content changed, which means the file is in
        // `fresh_slices.keys()` and will be written here.
        Self::write_file_cache(
            graph_repo,
            &fresh_slices,
            &language,
            &files,
            plan_disabled,
            cache_hits,
        )
        .await?;

        let scan_stats = crate::infrastructure::storage::parse_meta_store::IncrementalScanStats {
            cached: cache_hits,
            reparsed: cache_misses,
            removed: cache_removed,
            plan_disabled,
            changed_paths: plan
                .changed
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            removed_paths: plan.removed_paths.clone(),
        };
        if let Err(err) = graph_repo.write_incremental_scan_stats(&scan_stats).await {
            warn!(
                "S2: incremental_scan_stats persistence failed ({}); analysis fast-path disabled this scan",
                err
            );
        }

        Ok(ResolvedGraph::with_session_metrics(
            graph,
            stats,
            session_metrics_snapshot,
        ))
    }

    /// S1: compute the incremental plan + load cached slices, or
    /// produce an empty plan + disabled-cache flag when incremental
    /// scanning is off (CLI `--no-incremental`) or when any I/O step
    /// fails. The orchestrator never aborts on a cache failure — the
    /// cache is an optimisation, not a correctness contract — so this
    /// helper degrades to "treat every file as changed" on any
    /// problem and logs a `warn!`.
    async fn compute_incremental_plan(
        incremental: bool,
        files: &[PathBuf],
        language: &str,
        graph_repo: &dyn GraphRepository,
    ) -> (
        IncrementalPlan,
        BTreeMap<String, PerFileSlice>,
        bool, // disabled
    ) {
        if !incremental {
            // `--no-incremental` bypasses cache *reuse* but still needs
            // the planner's `removed_paths` (and per-file changed set)
            // so pre-extract graph pruning stays honest on a
            // persistent DB. Without this, a full reparse would UPSERT
            // new node ids alongside stale rows for the same files.
            let current_hashes = match file_hashing::hash_files(files) {
                Ok(h) => h,
                Err(err) => {
                    warn!(
                        "S1: file-hashing failed during --no-incremental ({}); \
                         pre-extract prune will be skipped",
                        err
                    );
                    return (IncrementalPlan::default(), BTreeMap::new(), true);
                }
            };
            let cache_rows = match graph_repo.read_file_cache().await {
                Ok(rows) => rows,
                Err(err) => {
                    warn!(
                        "S1: file_cache read failed during --no-incremental ({}); \
                         pre-extract prune will be skipped",
                        err
                    );
                    return (IncrementalPlan::default(), BTreeMap::new(), true);
                }
            };
            let plan = compute_plan(files, &current_hashes, &cache_rows, language);
            let forced = IncrementalPlan {
                unchanged: Vec::new(),
                changed: files.to_vec(),
                removed_paths: plan.removed_paths,
                total_count: files.len(),
            };
            return (forced, BTreeMap::new(), true);
        }

        let current_hashes = match file_hashing::hash_files(files) {
            Ok(h) => h,
            Err(err) => {
                warn!(
                    "S1: file-hashing failed ({}); falling back to full reparse",
                    err
                );
                return (IncrementalPlan::default(), BTreeMap::new(), true);
            }
        };

        let cache_rows = match graph_repo.read_file_cache().await {
            Ok(rows) => rows,
            Err(err) => {
                warn!(
                    "S1: file_cache read failed ({}); falling back to full reparse",
                    err
                );
                return (IncrementalPlan::default(), BTreeMap::new(), true);
            }
        };

        let plan = compute_plan(files, &current_hashes, &cache_rows, language);

        // Materialise the cached slices for unchanged files. Each
        // row's `payload_json` deserialises into a `PerFileSlice`.
        // A deserialise failure on any one row is treated as a cache
        // miss for that file (push it onto the changed set, drop the
        // slice). This keeps a corrupted cache row from blocking the
        // whole scan; the failed file just gets re-extracted.
        let mut cached_slices: BTreeMap<String, PerFileSlice> = BTreeMap::new();
        let mut soft_changed: Vec<PathBuf> = Vec::new();
        for path in &plan.unchanged {
            let Some(row) = cache_rows.get(path) else {
                continue;
            };
            match serde_json::from_str::<PerFileSlice>(&row.payload_json) {
                Ok(slice) => {
                    cached_slices.insert(path.clone(), slice);
                }
                Err(err) => {
                    warn!(
                        "S1: dropping corrupted cache row for {} ({}); re-extracting",
                        path, err
                    );
                    soft_changed.push(PathBuf::from(path));
                }
            }
        }

        // Promote any soft-changed (deserialise-failed) paths from
        // `unchanged` to `changed` so the extractor reparses them.
        let mut plan = plan;
        if !soft_changed.is_empty() {
            let soft_set: HashSet<String> = soft_changed
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            plan.unchanged.retain(|p| !soft_set.contains(p));
            plan.changed.extend(soft_changed);
            plan.total_count = plan.unchanged.len() + plan.changed.len();
        }

        (plan, cached_slices, false)
    }

    /// S1: merge cached per-file slices into the freshly-extracted
    /// aggregate. The returned `SyntaxResults` contains every file's
    /// items — changed (freshly extracted) and unchanged (loaded from
    /// the cache) — so downstream resolution + graph-building runs
    /// end-to-end on the full set.
    fn merge_cached_into_extraction(
        mut fresh_results: super::super::super::super::ports::SyntaxResults,
        fresh_slices: &BTreeMap<String, PerFileSlice>,
        cached_slices: &BTreeMap<String, PerFileSlice>,
        scan_meta: &ScanMetadata,
    ) -> super::super::super::super::ports::SyntaxResults {
        // Start from the fresh slices (everything we just extracted),
        // then layer the cached slices on top. Any key collision is a
        // bug — `unchanged` and `changed` sets are disjoint by
        // construction of the planner — but we resolve in favour of
        // the fresh slice as the defensive choice (the freshly
        // extracted slice is the canonical one).
        let mut merged: BTreeMap<String, PerFileSlice> = cached_slices.clone();
        for (key, slice) in fresh_slices {
            merged.insert(key.clone(), slice.clone());
        }
        let merged_results = reconstitute_from_slices(&merged, scan_meta);
        // Carry forward the scan-level fields the slicer cleared
        // (set on `fresh_results` after extraction). The reconstitute
        // step honours `scan_meta` for these fields, but we make sure
        // the merged result keeps the language / workspace_root the
        // orchestrator stamped onto `fresh_results` already.
        fresh_results.symbols = merged_results.symbols;
        fresh_results.identifier_uses = merged_results.identifier_uses;
        fresh_results.references = merged_results.references;
        fresh_results.imports = merged_results.imports;
        fresh_results.type_refs = merged_results.type_refs;
        fresh_results.type_references = merged_results.type_references;
        fresh_results.import_specs = merged_results.import_specs;
        fresh_results.mod_decls = merged_results.mod_decls;
        fresh_results.synthesized_edges = merged_results.synthesized_edges;
        fresh_results.class_symbols = merged_results.class_symbols;
        fresh_results.local_var_scopes = merged_results.local_var_scopes;
        fresh_results.extraction_coverage = merged_results.extraction_coverage;
        fresh_results
    }

    /// S1: write `file_cache` rows for the files actually extracted
    /// this scan, then prune cache rows for files that disappeared.
    /// Errors are logged but never fail the scan — the cache is an
    /// optimisation, not a correctness contract.
    async fn write_file_cache(
        graph_repo: &dyn GraphRepository,
        fresh_slices: &BTreeMap<String, PerFileSlice>,
        language: &str,
        discovered_files: &[PathBuf],
        plan_disabled: bool,
        cache_hits: usize,
    ) -> Result<(), ParsingError> {
        // The orphan bucket holds items not attributable to a
        // specific file (synthesised edges with unknown from_id,
        // class_symbols whose FQN doesn't match a symbol). These
        // are not cacheable per-file — drop them from the write set.
        // Their nodes are still in the persisted graph; only the
        // scratch-pad cache row is skipped.
        let cached_at = system_time_to_iso8601(SystemTime::now());
        let mut rows: Vec<FileCacheRow> = Vec::with_capacity(fresh_slices.len());
        for (file_path, slice) in fresh_slices {
            if file_path == ORPHAN_FILE_KEY {
                continue;
            }
            // Re-hash from disk so the cache row's hash matches the
            // exact content the extractor consumed. Hashing one file
            // is cheap; we accept the extra I/O so the cache write
            // doesn't depend on the bookkeeping the planner already
            // did (those hashes are dropped at this point).
            let bytes = match std::fs::read(file_path) {
                Ok(b) => b,
                Err(err) => {
                    warn!(
                        "S1: cache write skipped for {} ({}); row will not update",
                        file_path, err
                    );
                    continue;
                }
            };
            let content_hash = blake3::hash(&bytes).to_hex().to_string();
            let payload_json = match serde_json::to_string(slice) {
                Ok(s) => s,
                Err(err) => {
                    warn!(
                        "S1: cache write skipped for {} (serialise failed: {})",
                        file_path, err
                    );
                    continue;
                }
            };
            rows.push(FileCacheRow {
                file_path: file_path.clone(),
                content_hash,
                language: language.to_string(),
                payload_json,
                cached_at: cached_at.clone(),
            });
        }

        if !rows.is_empty() {
            let row_count = rows.len();
            if let Err(err) = graph_repo.upsert_file_cache(&rows).await {
                warn!(
                    "S1: file_cache upsert failed ({}); cache will not reflect this scan",
                    err
                );
            } else {
                info!("[TIMING] file_cache persistence ({} rows)", row_count);
            }
        } else if !plan_disabled && cache_hits == 0 {
            debug!("S1: no cache rows to write (cold scan with no extractable files)");
        }

        // Prune cache rows whose files are no longer discovered.
        // Scoped to THIS language — the file_cache table is shared
        // across passes in a persistent parse DB.
        {
            let current_paths: HashSet<String> = discovered_files
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            match graph_repo
                .prune_file_cache_missing(language, &current_paths)
                .await
            {
                Ok(0) => {}
                Ok(removed) => info!("S1: pruned {} stale file_cache rows", removed),
                Err(err) => warn!(
                    "S1: file_cache prune failed ({}); stale rows may linger",
                    err
                ),
            }
        }

        Ok(())
    }

    /// Execute a synchronous pipeline step
    fn execute_step<F, T>(step_name: &str, step: F) -> Result<T, ParsingError>
    where
        F: FnOnce() -> Result<T, ParsingError>,
    {
        debug!("Executing step: {}", step_name);
        step().map_err(|e| {
            warn!("Step {} failed: {}", step_name, e);
            e
        })
    }

    /// Execute an asynchronous pipeline step
    async fn execute_step_async<F, Fut, T>(step_name: &str, step: F) -> Result<T, ParsingError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, ParsingError>>,
    {
        debug!("Executing step: {}", step_name);
        step().await.map_err(|e| {
            warn!("Step {} failed: {}", step_name, e);
            e
        })
    }
}

/// Format a SystemTime as a UTC ISO-8601 string with second precision
/// (`YYYY-MM-DDTHH:MM:SSZ`). The S1 cache stores this in
/// `file_cache.cached_at` for human-debuggable rows. We avoid pulling
/// in chrono/time just for this — the cache field is metadata, not
/// parsed by downstream consumers.
fn system_time_to_iso8601(t: SystemTime) -> String {
    // Compute days since the Unix epoch (1970-01-01) using whole-day
    // arithmetic so leap years fall out for free, then derive
    // year/month/day with the civil-from-days algorithm
    // (https://howardhinnant.github.io/date_algorithms.html#civil_from_days).
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let hour = (secs_of_day / 3600) as u32;
    let minute = ((secs_of_day % 3600) / 60) as u32;
    let second = (secs_of_day % 60) as u32;

    // civil_from_days: input is days since 1970-01-01, output is
    // (year, month [1-12], day [1-31]).
    let z = days + 719_468; // shift epoch to 0000-03-01
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = y + if month <= 2 { 1 } else { 0 };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod orchestrator_tests {
    use super::*;

    #[test]
    fn iso8601_formats_unix_epoch_as_1970_jan_1() {
        let s = system_time_to_iso8601(SystemTime::UNIX_EPOCH);
        assert_eq!(s, "1970-01-01T00:00:00Z");
    }

    #[test]
    fn iso8601_formats_known_post_epoch_second() {
        // 2024-01-02T03:04:05Z is 1_704_164_645 unix seconds.
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_704_164_645);
        assert_eq!(system_time_to_iso8601(t), "2024-01-02T03:04:05Z");
    }

    #[test]
    fn iso8601_handles_leap_year_day() {
        // 2024-02-29T12:00:00Z is 1_709_208_000 unix seconds.
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_709_208_000);
        assert_eq!(system_time_to_iso8601(t), "2024-02-29T12:00:00Z");
    }
}
