//! Import resolution using LSP and heuristics
//!
//! Resolves import relationships between modules and imported symbols,
//! with LSP-based resolution and heuristic fallback. Processes imports
//! in concurrent chunks via `join_all` for throughput.

use crate::application::ports::{CallSite, SyntaxResults};
use crate::domain::{Confidence, Edge, EdgeKind, Provenance, ProvenanceSource, Range};
use crate::infrastructure::lsp::definition_provider::DefinitionProvider;
use crate::infrastructure::lsp::errors::LspError;
use crate::infrastructure::lsp::stats::LspMiss;
use crate::infrastructure::lsp::utils::document_sync::DocumentSyncManager;
use crate::infrastructure::lsp::utils::symbol_lookup::{
    find_containing_module, find_symbol_at_location,
};
use crate::module_resolution::ModuleResolver;
use crate::symbol_index::SymbolIndex;
use futures::future::join_all;
use std::collections::HashSet;
use tokio::time::Duration;
use tracing::{info, warn};

const DEFAULT_CHUNK_SIZE: usize = 32;

/// Resolves import relationships
pub struct ImportResolver;

impl ImportResolver {
    /// Resolve imports using LSP with concurrent chunks.
    pub async fn resolve_with_lsp(
        definition_provider: &dyn DefinitionProvider,
        syntax_results: &SyntaxResults,
        misses: &mut Vec<LspMiss>,
        total_work: usize,
        completed_work: usize,
    ) -> Result<(Vec<Edge>, Vec<Range>), LspError> {
        let mut edges = Vec::new();
        let mut unresolved_imports = Vec::new();

        match definition_provider.ensure_ready().await {
            Ok(_) => {
                let mut files_to_sync: HashSet<String> = HashSet::new();
                for spec in &syntax_results.import_specs {
                    files_to_sync.insert(spec.source_file.clone());
                }

                let opened_documents = DocumentSyncManager::sync_documents_simple(
                    definition_provider,
                    files_to_sync,
                    true,
                    Duration::from_secs(5),
                )
                .await;

                let total_imports = syntax_results.import_specs.len();
                let mut processed = 0usize;

                for chunk in syntax_results.import_specs.chunks(DEFAULT_CHUNK_SIZE) {
                    let futs: Vec<_> = chunk
                        .iter()
                        .map(|spec| {
                            let import_name = spec
                                .alias
                                .clone()
                                .or_else(|| spec.path.segments.last().cloned())
                                .unwrap_or_default();
                            let call_site = CallSite {
                                location: spec.range.clone(),
                                function_name: import_name.clone(),
                                receiver_range: None,
                                receiver_text: None,
                                arg_types: Vec::new(),
                            };
                            async move {
                                let result = definition_provider.find_definition(&call_site).await;
                                (import_name, result)
                            }
                        })
                        .collect();

                    let chunk_results = join_all(futs).await;

                    for (spec, (import_name, result)) in chunk.iter().zip(chunk_results) {
                        processed += 1;

                        if total_imports > 0
                            && (processed.is_multiple_of((total_imports / 4).max(1))
                                || processed == total_imports)
                        {
                            let phase_progress =
                                (processed as f64 / total_imports as f64 * 100.0) as u32;
                            let overall_progress = if total_work > 0 {
                                ((completed_work + processed) as f64 / total_work as f64 * 100.0)
                                    as u32
                            } else {
                                0
                            };
                            info!(
                                "[2/5] Imports: {}/{} ({}% phase, {}% overall)",
                                processed, total_imports, phase_progress, overall_progress
                            );
                        }

                        match result {
                            Ok(Some(def_range)) => {
                                if let Some(module_id) =
                                    find_containing_module(&spec.range, syntax_results)
                                {
                                    if let Some(imported_symbol) =
                                        find_symbol_at_location(&def_range, syntax_results)
                                    {
                                        if module_id == imported_symbol.id {
                                            warn!(
                                                "Skipping invalid Import self-loop for module_id={} (symbol={})",
                                                module_id, imported_symbol.fqn
                                            );
                                            unresolved_imports.push(spec.range.clone());
                                            continue;
                                        }
                                        let edge = Edge::new(
                                            module_id,
                                            imported_symbol.id.clone(),
                                            EdgeKind::Import,
                                            Provenance::new(
                                                ProvenanceSource::Lsp,
                                                Confidence::High,
                                            ),
                                        );
                                        edges.push(edge);
                                    } else {
                                        unresolved_imports.push(spec.range.clone());
                                    }
                                } else {
                                    unresolved_imports.push(spec.range.clone());
                                }
                            }
                            Ok(None) => {
                                unresolved_imports.push(spec.range.clone());
                                misses.push(LspMiss {
                                    file: spec.range.file.clone(),
                                    symbol: import_name,
                                    line: spec.range.start_line,
                                    char: spec.range.start_char,
                                });
                            }
                            Err(e) => {
                                warn!("Failed to resolve import with LSP: {}", e);
                                unresolved_imports.push(spec.range.clone());
                            }
                        }
                    }
                }

                DocumentSyncManager::close_documents(definition_provider, &opened_documents).await;
            }
            Err(err) => {
                warn!("Unable to prepare definition provider for imports: {}", err);
                for spec in &syntax_results.import_specs {
                    unresolved_imports.push(spec.range.clone());
                }
            }
        }

        Ok((edges, unresolved_imports))
    }

    /// Resolve imports using heuristics (fallback when LSP fails)
    pub fn resolve_with_heuristics(
        syntax_results: &SyntaxResults,
        unresolved_imports: &[Range],
    ) -> Result<Vec<Edge>, LspError> {
        let module_resolver = ModuleResolver::from_syntax(syntax_results);
        let symbol_index = SymbolIndex::from_syntax(syntax_results);
        let mut edges = Vec::new();

        for import_range in unresolved_imports {
            if let Some(module_id) = find_containing_module(import_range, syntax_results) {
                if let Some(import_spec) = syntax_results
                    .import_specs
                    .iter()
                    .find(|spec| spec.range == *import_range)
                {
                    let mut candidate_names: Vec<String> = Vec::new();
                    if let Some(alias) = &import_spec.alias {
                        candidate_names.push(alias.clone());
                    }
                    if let Some(last_segment) = import_spec.path.segments.last() {
                        if !candidate_names
                            .iter()
                            .any(|candidate| candidate == last_segment)
                        {
                            candidate_names.push(last_segment.clone());
                        }
                    }

                    for candidate in candidate_names {
                        if let Some(resolved) = symbol_index.resolve_function(
                            &candidate,
                            &import_range.file,
                            &module_resolver,
                        ) {
                            if module_id == resolved.record.id {
                                warn!(
                                    "Skipping invalid Import self-loop for module_id={} (candidate={})",
                                    module_id, candidate
                                );
                                continue;
                            }
                            let edge = Edge::new(
                                module_id.clone(),
                                resolved.record.id.clone(),
                                EdgeKind::Import,
                                Provenance::new(ProvenanceSource::Heuristic, Confidence::Medium),
                            );
                            edges.push(edge);
                            break;
                        }
                    }
                }
            }
        }

        Ok(edges)
    }
}
