//! Analysis segment runner (S2-γ).

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use anyhow::Result;
use rusqlite::Connection;

use super::super::super::config::AnalysisConfig;
use super::super::super::graph::AnalysisGraph;
use super::super::super::progress;
use super::super::super::report;
use super::super::super::report::*;
use super::super::super::{empty_report, is_parse_db_stale, node_annotation_for};
use super::super::super::{graph, structural_classification};
use super::super::session::AnalysisRunContext;

pub fn run(ctx: &mut AnalysisRunContext<'_>) -> Result<Option<HealthReport>> {
    ctx.start = Instant::now();

    // Stage 1 shadow-mode progress: emit a structured event at each
    // detector boundary. The eprintln lines stay for humans reading
    // the analyzer log directly; the JSONL events are what
    // `gridseak-engine-runner` parses to drive percentage UIs in the
    // CLI and desktop. Both are routed through stderr; the consumer's
    // `try_parse_line` decides which path each line takes.
    progress::emit_progress("loading", 2, "reading graph from database");
    eprintln!("[ge-analyze] Reading graph from database...");
    ctx.conn = Connection::open_with_flags(
        &ctx.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    graph::validate_schema(&ctx.conn)?;

    // TR-A.0 / R32: detect stale parse DBs so downstream consumers
    // know the Apex type oracle was never populated for this DB.
    // Any value < CURRENT_SCHEMA_VERSION (or missing parse_meta
    // table entirely) flags the DB as stale. See `build_integrity_status`
    // for the caveat semantics.
    ctx.stale_parse_db = is_parse_db_stale(&ctx.conn);

    // Detect ecosystem from DB before loading ctx.graph (need raw ctx.conn for Project node query)
    let detected_eco = graph::detect_ecosystem(&ctx.conn);
    ctx.config = match ctx.bootstrap_config.take() {
        Some(mut c) => {
            if c.ecosystem.is_none() {
                c.ecosystem = Some(detected_eco);
            }
            c
        }
        None => AnalysisConfig::for_ecosystem(detected_eco),
    };
    eprintln!(
        "[ge-analyze] Ecosystem: {}",
        ctx.config.resolved_ecosystem()
    );

    // Apply module depth override before loading ctx.graph (changes the depth)
    if let Some(ov) = ctx.overrides {
        crate::validation::overrides::apply_module_overrides(ov, &mut ctx.config);
    }

    ctx.graph = AnalysisGraph::load_with_module_config(
        &ctx.conn,
        ctx.config.modules.analysis_depth,
        ctx.config.modules.strip_build_convention_dirs,
    )?;
    eprintln!(
        "[ge-analyze] {} nodes, {} edges loaded.",
        ctx.graph.total_nodes(),
        ctx.graph.total_edges()
    );

    // Apply file classification ctx.overrides early so production edge filtering benefits
    ctx.override_entry_point_ids = HashSet::new();
    if let Some(ov) = ctx.overrides {
        let summary =
            crate::validation::overrides::apply_file_overrides(&mut ctx.graph, &ov.file_overrides);
        if summary > 0 {
            eprintln!("[ge-analyze] Applied {summary} file classification override(s).");
        }
        let (exempt_ids, ep_count) = crate::validation::overrides::apply_entry_point_overrides(
            ov,
            &mut ctx.config.dead_code,
        );
        ctx.override_entry_point_ids = exempt_ids;
        if ep_count > 0 {
            eprintln!("[ge-analyze] Applied {ep_count} entry point override(s).");
        }
    }

    if ctx.graph.total_nodes() == 0 {
        return Ok(Some(empty_report(
            &ctx.db_path,
            ctx.start.elapsed().as_millis() as u64,
            &ctx.config.score_weights,
        )));
    }

    let import_edge_count = ctx.graph.import_edge_count();
    let stored_import_edges: Option<usize> =
        graph::read_metadata(&ctx.conn, "import_edges").and_then(|v| v.parse().ok());
    let effective_import_count = stored_import_edges.unwrap_or(import_edge_count);

    let lsp_available: bool = graph::read_metadata(&ctx.conn, "lsp_available")
        .map(|v| v == "true")
        .unwrap_or(false);

    ctx.resolution_quality = {
        let tier = if effective_import_count == 0 && ctx.graph.total_structural_edges() > 0 {
            ResolutionTier::None
        } else if effective_import_count > 0 && lsp_available {
            ResolutionTier::Full
        } else if effective_import_count > 0 {
            ResolutionTier::HeuristicOnly
        } else {
            ResolutionTier::None
        };
        let recommendation = match tier {
            ResolutionTier::None => Some(
                "No import edges detected. Install the appropriate LSP server for higher accuracy cross-file analysis.".into()
            ),
            ResolutionTier::HeuristicOnly => Some(
                "LSP was not available during parsing. Install the language server for higher accuracy resolution.".into()
            ),
            ResolutionTier::Full => None,
        };
        let call_edges = ctx.graph.call_edges_by_confidence();
        let all_edges = ctx.graph.all_edges_by_confidence();
        let measured_tier = report::MeasuredFidelityTier::from_call_edges(&call_edges);
        let high_ratio_on_calls = if call_edges.total() == 0 {
            None
        } else {
            Some(call_edges.high_ratio())
        };
        ResolutionQuality {
            import_edges_total: effective_import_count,
            resolution_tier: tier,
            measured_fidelity: report::MeasuredFidelity {
                tier: measured_tier,
                high_ratio_on_calls,
                call_edges_by_confidence: call_edges,
                all_edges_by_confidence: all_edges,
            },
            recommendation,
        }
    };

    let graph_lacks_imports = ctx.resolution_quality.resolution_tier == ResolutionTier::None
        && ctx.graph.total_structural_edges() > 0;
    if graph_lacks_imports {
        eprintln!(
            "[ge-analyze] Warning: No Import edges found in ctx.graph ({} structural edges, all intra-file). \
             Cross-file metrics (dead code, coupling, fan-in, blast radius) may ctx.report false positives. \
             Ensure the parser config has import sub-captures (@source, @imported_name) for this language.",
            ctx.graph.total_structural_edges(),
        );
    }

    ctx.findings = Vec::new();
    ctx.analysis_errors = Vec::new();
    ctx.node_annotations = BTreeMap::new();
    ctx.module_annotations = BTreeMap::new();

    // Initialize annotations for all function nodes with identity info
    for id in &ctx.graph.function_node_ids {
        let ann = match ctx.graph.nodes.get(id) {
            Some(node) => node_annotation_for(node),
            None => continue,
        };
        ctx.node_annotations.insert(id.clone(), ann);
    }

    // --- 0. Prepare ctx.graph (MUST run before any metric that consults
    //        production_structural_edge_indices). Ordering bug fix:
    //        previously cycle detection ran here and saw an empty
    //        production edge set because finalize_production_edges()
    //        was deferred until after coupling. See docs/workstreams/proof-foundation-gap/FINDINGS.md.
    progress::emit_progress(
        "test_classification",
        15,
        "running structural test classification",
    );
    eprintln!("[ge-analyze] Running structural test classification...");
    let language_for_classification = ctx.config.resolved_ecosystem().to_string();
    let structural_test_files =
        structural_classification::classify_files(&ctx.graph, &language_for_classification);
    eprintln!(
        "[ge-analyze] Structural classification: {} test/test-support files identified",
        structural_test_files.len()
    );

    // Propagate structural classification to File nodes so existing is_test checks benefit.
    {
        let test_file_paths: HashSet<String> = structural_test_files
            .iter()
            .filter(|(_, c)| {
                matches!(
                    c.role,
                    structural_classification::FileRole::Test
                        | structural_classification::FileRole::TestSupport
                )
            })
            .filter_map(|(id, _)| ctx.graph.nodes.get(id).and_then(|n| n.file_path.clone()))
            .collect();

        for node in ctx.graph.nodes.values_mut() {
            if node.kind == graph::NodeKind::File {
                let path = node.file_path.as_deref().or(node.path_repo_rel.as_deref());
                if let Some(p) = path {
                    if test_file_paths.contains(p) {
                        node.is_test = true;
                    }
                }
            }
        }
    }

    // Build production-only edge index now that classification flags are finalized.
    ctx.graph.finalize_production_edges();
    eprintln!(
        "[ge-analyze] Production edges: {} of {} structural edges.",
        ctx.graph.production_structural_edge_indices.len(),
        ctx.graph.structural_edge_indices.len(),
    );

    // Pipeline invariants: guard against silent regressions of the ordering
    // bug and similar empty-collection hazards. Debug builds panic; release
    // builds surface violations as AnalysisError entries so the downstream
    // report can flag the data as suspect.
    if let Err(violations) = ctx.graph.validate_invariants() {
        for v in &violations {
            eprintln!("[ge-analyze] INVARIANT VIOLATION: {}", v);
            ctx.analysis_errors.push(AnalysisError {
                algorithm: "graph_invariants".into(),
                error: v.clone(),
                nodes_affected: None,
            });
        }
        debug_assert!(
            violations.is_empty(),
            "AnalysisGraph invariant violations: {:?}",
            violations
        );
    }

    Ok(None)
}
