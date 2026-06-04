//! LSP-based call resolution
//!
//! Resolves function calls using LSP semantic analysis, with support for
//! trait object detection and symbol candidate filtering. Processes call
//! sites in concurrent chunks via `join_all` for throughput.

use crate::application::ports::{CallSite, SyntaxResults, UnresolvedReference};
use crate::domain::{Confidence, Edge, Provenance, ProvenanceSource, Range};
use crate::infrastructure::lsp::definition_provider::DefinitionProvider;
use crate::infrastructure::lsp::errors::LspError;
use crate::infrastructure::lsp::receiver_detector::ReceiverTypeDetector;
use crate::infrastructure::lsp::stats::LspMiss;
use crate::infrastructure::lsp::stats::LspResolutionOutcome;
use crate::infrastructure::lsp::utils::{
    call_site_utils::extract_function_name, document_sync::DocumentSyncManager,
    symbol_lookup::find_containing_function,
};
use crate::symbol_index::SymbolIndex;
use futures::future::join_all;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use tokio::time::Duration;
use tracing::{debug, warn};

const DEFAULT_CHUNK_SIZE: usize = 32;

/// Resolves function calls using LSP
pub struct LspCallResolver;

impl LspCallResolver {
    /// Resolve calls using LSP semantic analysis with concurrent chunks.
    pub async fn resolve_with_lsp(
        definition_provider: &dyn DefinitionProvider,
        receiver_detector: Option<&Arc<ReceiverTypeDetector>>,
        syntax_results: &SyntaxResults,
        misses: &mut Vec<LspMiss>,
    ) -> Result<LspResolutionOutcome, LspError> {
        let mut outcome = LspResolutionOutcome::default();
        let symbol_index = SymbolIndex::from_syntax(syntax_results);

        let mut files_to_sync: HashSet<String> = HashSet::new();
        for reference in &syntax_results.references {
            let cs = reference.call_site();
            if !cs.location.file.is_empty() {
                files_to_sync.insert(cs.location.file.clone());
            }
        }

        let (opened_documents, _) = DocumentSyncManager::sync_documents(
            definition_provider,
            files_to_sync,
            true,
            Duration::from_secs(5),
        )
        .await;

        let definition_cache: Mutex<HashMap<(String, String), Option<Range>>> =
            Mutex::new(HashMap::new());

        for chunk in syntax_results.references.chunks(DEFAULT_CHUNK_SIZE) {
            let futs: Vec<_> = chunk
                .iter()
                .map(|reference| {
                    Self::resolve_single_reference(
                        definition_provider,
                        receiver_detector,
                        reference,
                        syntax_results,
                        &symbol_index,
                        &definition_cache,
                    )
                })
                .collect();

            let chunk_results = join_all(futs).await;

            for (reference, result) in chunk.iter().zip(chunk_results) {
                let call_site = reference.call_site();
                match result {
                    Ok(Some(edge)) => {
                        outcome.edges.push(edge);
                    }
                    Ok(None) => {
                        outcome.unresolved_calls.push(reference.clone());
                        misses.push(LspMiss {
                            file: call_site.location.file.clone(),
                            symbol: call_site.function_name.clone(),
                            line: call_site.location.start_line,
                            char: call_site.location.start_char,
                        });
                    }
                    Err(e) => {
                        warn!("Failed to resolve call site with LSP: {}", e);
                        outcome.unresolved_calls.push(reference.clone());
                    }
                }
            }
        }

        DocumentSyncManager::close_documents(definition_provider, &opened_documents).await;

        Ok(outcome)
    }

    async fn resolve_single_reference(
        definition_provider: &dyn DefinitionProvider,
        receiver_detector: Option<&Arc<ReceiverTypeDetector>>,
        reference: &UnresolvedReference,
        syntax_results: &SyntaxResults,
        symbol_index: &SymbolIndex,
        definition_cache: &Mutex<HashMap<(String, String), Option<Range>>>,
    ) -> Result<Option<Edge>, LspError> {
        let call_site = reference.call_site();
        let edge_kind = reference.edge_kind();
        let containing_function = find_containing_function(&call_site.location, syntax_results);

        if let Some(caller_id) = containing_function {
            let actual_function_name = extract_function_name(&call_site.function_name);

            if actual_function_name.is_empty() {
                debug!(
                    "Skipping LSP lookup for empty/invalid function name: '{}'",
                    call_site.function_name
                );
                return Ok(None);
            }

            let is_trait_object_call = if let Some(detector) = receiver_detector {
                if let Some(receiver_range) = &call_site.receiver_range {
                    match detector
                        .is_trait_object_call(call_site, Some(receiver_range))
                        .await
                    {
                        Ok(Some(true)) => {
                            debug!("Detected trait object call: {}", call_site.function_name);
                            true
                        }
                        Ok(Some(false)) => false,
                        Ok(None) => false,
                        Err(e) => {
                            warn!("Receiver type detection failed: {}", e);
                            false
                        }
                    }
                } else {
                    false
                }
            } else {
                false
            };

            let lsp_call_site = CallSite {
                location: call_site.location.clone(),
                function_name: actual_function_name.clone(),
                receiver_range: call_site.receiver_range.clone(),
                receiver_text: call_site.receiver_text.clone(),
                arg_types: call_site.arg_types.clone(),
            };

            // Qualified names (containing `::`) are position-independent within a file
            // and safe to cache. Bare method names are receiver-dependent and must
            // always go through LSP to avoid incorrect dedup.
            let is_qualified = actual_function_name.contains("::");

            let definition_result = if is_qualified {
                let cache_key = (
                    call_site.location.file.clone(),
                    actual_function_name.clone(),
                );

                let cached = {
                    let cache = definition_cache.lock().unwrap();
                    cache.get(&cache_key).cloned()
                };

                if let Some(cached_result) = cached {
                    Ok(cached_result)
                } else {
                    let result = definition_provider.find_definition(&lsp_call_site).await;
                    if let Ok(ref def) = result {
                        let mut cache = definition_cache.lock().unwrap();
                        cache.insert(cache_key, def.clone());
                    }
                    result
                }
            } else {
                definition_provider.find_definition(&lsp_call_site).await
            };

            match definition_result {
                Ok(Some(location)) => {
                    let candidates =
                        symbol_index.resolve_by_location_all(&location.file, &location);

                    let resolved_symbol = if is_trait_object_call {
                        candidates
                            .iter()
                            .find(|s| {
                                if let Some(ref trait_meta) = s.record.trait_metadata {
                                    !trait_meta.is_trait_default
                                        && trait_meta.implementing_type.is_some()
                                } else {
                                    false
                                }
                            })
                            .or_else(|| candidates.first())
                    } else {
                        candidates
                            .iter()
                            .find(|s| s.record.trait_metadata.is_none())
                            .or_else(|| candidates.first())
                    };

                    if let Some(resolved_symbol) = resolved_symbol {
                        let callee_id = resolved_symbol.record.id.clone();
                        if caller_id != callee_id {
                            // `edge_kind` is determined by the
                            // `UnresolvedReference` variant via
                            // `reference.edge_kind()` — the typed
                            // channel that replaced the old
                            // silently-broken-by-default hint field
                            // in P1.d.
                            let edge = Edge::new(
                                caller_id,
                                callee_id,
                                edge_kind,
                                Provenance::new(ProvenanceSource::Lsp, Confidence::High),
                            );
                            return Ok(Some(edge));
                        } else {
                            debug!("Skipping self-loop: {} -> {}", caller_id, callee_id);
                        }
                    } else {
                        debug!("No symbol found for LSP definition at {}", location.file);
                    }
                }
                Ok(None) => {
                    debug!(
                        "LSP could not find definition for '{}' (original: '{}') - will fall back to heuristics",
                        actual_function_name, call_site.function_name
                    );
                }
                Err(e) => {
                    warn!(
                        "LSP definition lookup failed for '{}': {}",
                        actual_function_name, e
                    );
                }
            }
        }

        Ok(None)
    }
}
