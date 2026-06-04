//! Type resolution using LSP and heuristics
//!
//! Resolves type relationships between symbols and their type references,
//! with LSP-based resolution and heuristic fallback. Processes type refs
//! in concurrent chunks via `join_all` for throughput.

use crate::application::ports::{CallSite, SyntaxResults};
use crate::domain::{Confidence, Edge, EdgeKind, NodeKind, Provenance, ProvenanceSource, Range};
use crate::infrastructure::lsp::definition_provider::DefinitionProvider;
use crate::infrastructure::lsp::errors::LspError;
use crate::infrastructure::lsp::stats::LspMiss;
use crate::infrastructure::lsp::utils::document_sync::DocumentSyncManager;
use crate::infrastructure::lsp::utils::{
    range_utils::is_within_range, source_code_utils::extract_identifier_at_range,
    symbol_lookup::find_containing_function,
};
use futures::future::join_all;
use std::collections::HashSet;
use tokio::time::Duration;
use tracing::warn;

const DEFAULT_CHUNK_SIZE: usize = 32;

/// Resolves type relationships
pub struct TypeResolver;

impl TypeResolver {
    /// Resolve types using LSP with concurrent chunks.
    pub async fn resolve_with_lsp(
        definition_provider: &dyn DefinitionProvider,
        syntax_results: &SyntaxResults,
        misses: &mut Vec<LspMiss>,
    ) -> Result<(Vec<Edge>, Vec<Range>), LspError> {
        let mut edges = Vec::new();
        let mut unresolved_types = Vec::new();

        match definition_provider.ensure_ready().await {
            Ok(_) => {
                let mut files_to_sync: HashSet<String> = HashSet::new();
                for type_ref in &syntax_results.type_refs {
                    files_to_sync.insert(type_ref.file.clone());
                }

                let (opened_documents, file_cache) = DocumentSyncManager::sync_documents(
                    definition_provider,
                    files_to_sync,
                    true,
                    Duration::from_secs(5),
                )
                .await;

                for chunk in syntax_results.type_refs.chunks(DEFAULT_CHUNK_SIZE) {
                    let futs: Vec<_> = chunk
                        .iter()
                        .map(|type_ref| {
                            let type_name = extract_identifier_at_range(type_ref, &file_cache);
                            async move {
                                let name = match type_name {
                                    Some(ref n) if !n.is_empty() => n.clone(),
                                    _ => return (type_ref, None::<String>, None),
                                };
                                let call_site = CallSite {
                                    location: type_ref.clone(),
                                    function_name: name.clone(),
                                    receiver_range: None,
                                    receiver_text: None,
                                    arg_types: Vec::new(),
                                };
                                let result = definition_provider.find_definition(&call_site).await;
                                (type_ref, Some(name), Some(result))
                            }
                        })
                        .collect();

                    let chunk_results = join_all(futs).await;

                    for (type_ref, type_name_opt, result_opt) in chunk_results {
                        let type_name = match type_name_opt {
                            Some(name) => name,
                            None => {
                                unresolved_types.push(type_ref.clone());
                                continue;
                            }
                        };

                        let result = match result_opt {
                            Some(r) => r,
                            None => {
                                unresolved_types.push(type_ref.clone());
                                continue;
                            }
                        };

                        match result {
                            Ok(Some(def_range)) => {
                                if let Some(container_id) =
                                    find_containing_function(type_ref, syntax_results)
                                {
                                    if let Some(type_symbol) =
                                        syntax_results.symbols.iter().find(|symbol| {
                                            symbol.location.file == def_range.file
                                                && is_within_range(&def_range, &symbol.location)
                                        })
                                    {
                                        if matches!(
                                            type_symbol.kind,
                                            NodeKind::Struct | NodeKind::Enum | NodeKind::Type
                                        ) {
                                            let edge = Edge::new(
                                                container_id,
                                                type_symbol.id.clone(),
                                                EdgeKind::Type,
                                                Provenance::new(
                                                    ProvenanceSource::Lsp,
                                                    Confidence::High,
                                                ),
                                            );
                                            edges.push(edge);
                                        } else {
                                            unresolved_types.push(type_ref.clone());
                                        }
                                    } else {
                                        unresolved_types.push(type_ref.clone());
                                    }
                                } else {
                                    unresolved_types.push(type_ref.clone());
                                }
                            }
                            Ok(None) => {
                                unresolved_types.push(type_ref.clone());
                                misses.push(LspMiss {
                                    file: type_ref.file.clone(),
                                    symbol: type_name,
                                    line: type_ref.start_line,
                                    char: type_ref.start_char,
                                });
                            }
                            Err(e) => {
                                warn!("Failed to resolve type reference with LSP: {}", e);
                                unresolved_types.push(type_ref.clone());
                            }
                        }
                    }
                }

                DocumentSyncManager::close_documents(definition_provider, &opened_documents).await;
            }
            Err(err) => {
                warn!("Unable to prepare definition provider for types: {}", err);
                for type_ref in &syntax_results.type_refs {
                    unresolved_types.push(type_ref.clone());
                }
            }
        }

        Ok((edges, unresolved_types))
    }

    /// Resolve types using heuristics (fallback when LSP fails)
    pub fn resolve_with_heuristics(
        syntax_results: &SyntaxResults,
        unresolved_types: &[Range],
    ) -> Result<Vec<Edge>, LspError> {
        let mut edges = Vec::new();

        for type_ref in unresolved_types {
            if let Some(container_id) = find_containing_function(type_ref, syntax_results) {
                for symbol in &syntax_results.symbols {
                    if matches!(
                        symbol.kind,
                        NodeKind::Struct | NodeKind::Enum | NodeKind::Type
                    ) && is_within_range(type_ref, &symbol.location)
                    {
                        let edge = Edge::new(
                            container_id.clone(),
                            symbol.id.clone(),
                            EdgeKind::Type,
                            Provenance::new(ProvenanceSource::Heuristic, Confidence::Medium),
                        );
                        edges.push(edge);
                        break;
                    }
                }
            }
        }

        Ok(edges)
    }
}
