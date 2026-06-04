//! Real LSP Resolver Implementation
//!
//! Integrates the real LSP client, timing coordinator, and synchronization
//! manager to provide a robust, race-condition-free LSP-based semantic resolver.

use crate::application::ports::{
    CallSite, ResolutionStatsSummary, ResolvedEdges, SemanticResolver, SyntaxResults,
    UnresolvedReference,
};
use crate::domain::{Confidence, Edge, EdgeKind, NodeKind, Provenance, ProvenanceSource};
use crate::infrastructure::config::LanguageConfig;
use crate::infrastructure::lsp::{
    LspClient, LspErrorType as LspError, ParsingPhase, SynchronizationManager, TimingConfig,
    TimingCoordinator,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, error, info, instrument, warn};
use url::Url;

/// Real LSP-based semantic resolver with proper timing and synchronization
#[derive(Debug)]
pub struct RealLspResolver {
    /// LSP client
    client: Arc<LspClient>,
    /// Timing coordinator
    timing: Arc<TimingCoordinator>,
    /// Synchronization manager
    sync: Arc<SynchronizationManager>,
    /// Language configuration
    config: Arc<LanguageConfig>,
    /// Initialization state
    initialized: Arc<std::sync::atomic::AtomicBool>,
}

impl RealLspResolver {
    /// Create a new real LSP resolver
    pub async fn new(
        config: LanguageConfig,
        workspace_root: Option<Url>,
        timing_config: Option<TimingConfig>,
    ) -> Result<Self, LspError> {
        let timing_config = timing_config.unwrap_or_default();
        let timing = Arc::new(TimingCoordinator::new(timing_config, 10));
        let sync = Arc::new(SynchronizationManager::new(10));

        // Create LSP client
        let client = Arc::new(LspClient::new(config.clone(), workspace_root.clone(), 10));

        let resolver = Self {
            client,
            timing,
            sync,
            config: Arc::new(config),
            initialized: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        // Initialize the resolver
        resolver.initialize().await?;

        Ok(resolver)
    }

    /// Initialize the LSP resolver
    #[instrument(skip(self))]
    async fn initialize(&self) -> Result<(), LspError> {
        if self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }

        // Start pipeline timing
        self.timing.start_pipeline().await.map_err(|e| {
            LspError::initialization_failed(format!("Failed to start timing pipeline: {}", e))
        })?;

        // Initialize LSP client with proper timing
        let init_result = self
            .timing
            .execute_phase(ParsingPhase::LspInitialization, || {
                let client = Arc::clone(&self.client);
                async move {
                    client
                        .initialize()
                        .await
                        .map_err(|e| format!("LSP client initialization failed: {}", e))
                }
            })
            .await;

        match init_result {
            Ok(_) => {
                self.initialized
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                info!("Real LSP resolver initialized successfully");
                Ok(())
            }
            Err(e) => {
                error!("Failed to initialize LSP resolver: {}", e);
                Err(LspError::initialization_failed(e))
            }
        }
    }

    /// Resolve function calls using real LSP
    #[instrument(skip(self, syntax_results))]
    async fn resolve_calls(&self, syntax_results: &SyntaxResults) -> Result<Vec<Edge>, LspError> {
        info!(
            "Resolving {} unresolved references using real LSP",
            syntax_results.references.len()
        );

        // Use synchronization to prevent race conditions
        let result = self
            .sync
            .execute_critical_section(|| {
                let client = Arc::clone(&self.client);
                let references = syntax_results.references.clone();
                async move {
                    let mut resolved_edges = Vec::new();

                    for reference in references {
                        // UF-FU-006: compute the LSP document URI from
                        // the call-site's actual file path. Short-circuit
                        // when the path is missing, unknown, or a
                        // placeholder — querying the LSP server with
                        // `file://<unknown>` can block on a "file does
                        // not exist" round-trip or succeed with irrelevant
                        // symbols from whatever file happens to share
                        // that fake URI in the server's cache.
                        let call_site = reference.call_site();
                        let file_uri = match lsp_document_uri_for(call_site) {
                            Some(uri) => uri,
                            None => {
                                warn!(
                                    "Skipping LSP resolution for call `{}`: \
                                     call-site has no usable file path \
                                     (got {:?}); falling through to heuristic.",
                                    call_site.function_name, call_site.location.file
                                );
                                continue;
                            }
                        };

                        match client.get_document_symbols(file_uri).await {
                            Ok(symbols) => {
                                // Parse symbols and find matching function
                                if let Some(edge) = self
                                    .resolve_single_call(&reference, &symbols, syntax_results)
                                    .await
                                {
                                    resolved_edges.push(edge);
                                }
                            }
                            Err(e) => {
                                warn!("Failed to get document symbols: {}", e);
                            }
                        }
                    }

                    Ok::<Vec<Edge>, String>(resolved_edges)
                }
            })
            .await;

        match result {
            Ok(edges) => Ok(edges),
            Err(e) => {
                error!("Failed to resolve calls: {}", e);
                Err(LspError::protocol_error(format!(
                    "Call resolution failed: {}",
                    e
                )))
            }
        }
    }

    /// Resolve a single function call
    async fn resolve_single_call(
        &self,
        reference: &UnresolvedReference,
        symbols: &Value,
        syntax_results: &SyntaxResults,
    ) -> Option<Edge> {
        let call_site = reference.call_site();
        // Find the function that contains this call site
        let containing_function =
            self.find_containing_function(&call_site.location, syntax_results);

        // Find the called function in the symbols
        let called_function = self.find_called_function(&call_site.function_name, symbols);

        if let (Some(caller_id), Some(callee_id)) = (containing_function, called_function) {
            build_resolved_call_edge(caller_id, callee_id, reference, &call_site.function_name)
        } else {
            debug!("Could not resolve call to: {}", call_site.function_name);
            None
        }
    }

    /// Find the function containing a call site
    fn find_containing_function(
        &self,
        location: &crate::domain::Range,
        syntax_results: &SyntaxResults,
    ) -> Option<String> {
        for symbol in &syntax_results.symbols {
            if symbol.kind == NodeKind::Function {
                // Check if the call site is within the function's range
                if self.is_within_range(location, &symbol.location) {
                    return Some(symbol.id.clone());
                }
            }
        }
        None
    }

    /// Find a called function in LSP symbols
    fn find_called_function(&self, function_name: &str, symbols: &Value) -> Option<String> {
        // Parse LSP symbols response and find matching function
        // This is a simplified implementation - in reality, we'd need to parse
        // the LSP response structure properly
        if let Some(symbols_array) = symbols.as_array() {
            for symbol in symbols_array {
                if let Some(name) = symbol.get("name").and_then(|n| n.as_str()) {
                    if name == function_name {
                        // Generate a consistent ID for the symbol
                        return Some(format!("lsp_{}", function_name));
                    }
                }
            }
        }
        None
    }

    /// Check if one range is within another
    fn is_within_range(&self, inner: &crate::domain::Range, outer: &crate::domain::Range) -> bool {
        // Simple range containment check
        inner.start_line >= outer.start_line && inner.end_line <= outer.end_line
    }

    /// Resolve imports using real LSP
    #[instrument(skip(self, syntax_results))]
    async fn resolve_imports(&self, syntax_results: &SyntaxResults) -> Result<Vec<Edge>, LspError> {
        info!(
            "Resolving {} imports using real LSP",
            syntax_results.imports.len()
        );

        // For now, create heuristic-based import edges
        // In a full implementation, we'd use LSP to resolve actual import relationships
        let mut edges = Vec::new();

        for import_range in &syntax_results.imports {
            // Find the module that contains this import
            if let Some(module_id) = self.find_containing_module(import_range, syntax_results) {
                // Create edges from modules to imported symbols
                for symbol in &syntax_results.symbols {
                    if symbol.kind == NodeKind::Module {
                        let edge = Edge::new(
                            module_id.clone(),
                            symbol.id.clone(),
                            EdgeKind::Import,
                            Provenance::new(ProvenanceSource::Lsp, Confidence::Medium),
                        );
                        edges.push(edge);
                    }
                }
            }
        }

        Ok(edges)
    }

    /// Resolve type references using real LSP
    #[instrument(skip(self, syntax_results))]
    async fn resolve_types(&self, syntax_results: &SyntaxResults) -> Result<Vec<Edge>, LspError> {
        info!(
            "Resolving {} type references using real LSP",
            syntax_results.type_refs.len()
        );

        // For now, create heuristic-based type edges
        // In a full implementation, we'd use LSP to resolve actual type relationships
        let mut edges = Vec::new();

        for type_ref in &syntax_results.type_refs {
            // Find the function that contains this type reference
            if let Some(function_id) = self.find_containing_function(type_ref, syntax_results) {
                // Create edges from functions to type symbols
                for symbol in &syntax_results.symbols {
                    if matches!(symbol.kind, NodeKind::Struct | NodeKind::Enum) {
                        let edge = Edge::new(
                            function_id.clone(),
                            symbol.id.clone(),
                            EdgeKind::Type,
                            Provenance::new(ProvenanceSource::Lsp, Confidence::Medium),
                        );
                        edges.push(edge);
                    }
                }
            }
        }

        Ok(edges)
    }

    /// Resolve containment relationships using real LSP
    #[instrument(skip(self, syntax_results))]
    async fn resolve_containment(
        &self,
        syntax_results: &SyntaxResults,
    ) -> Result<Vec<Edge>, LspError> {
        info!("Resolving containment relationships using real LSP");

        // Create containment edges between modules and their contained symbols
        let mut edges = Vec::new();

        for symbol in &syntax_results.symbols {
            if symbol.kind == NodeKind::Module {
                // Find symbols that are contained within this module
                for other_symbol in &syntax_results.symbols {
                    if other_symbol.id != symbol.id
                        && self.is_contained_in(&other_symbol.location, &symbol.location)
                    {
                        let edge = Edge::new(
                            symbol.id.clone(),
                            other_symbol.id.clone(),
                            EdgeKind::Contains,
                            Provenance::new(ProvenanceSource::Lsp, Confidence::High),
                        );
                        edges.push(edge);
                    }
                }
            }
        }

        Ok(edges)
    }

    /// Find the module containing a range
    fn find_containing_module(
        &self,
        range: &crate::domain::Range,
        syntax_results: &SyntaxResults,
    ) -> Option<String> {
        for symbol in &syntax_results.symbols {
            if symbol.kind == NodeKind::Module && self.is_within_range(range, &symbol.location) {
                return Some(symbol.id.clone());
            }
        }
        None
    }

    /// Check if one range is contained within another
    fn is_contained_in(&self, inner: &crate::domain::Range, outer: &crate::domain::Range) -> bool {
        self.is_within_range(inner, outer)
    }

    /// Check if the LSP client is available
    pub fn is_available(&self) -> bool {
        self.initialized.load(std::sync::atomic::Ordering::SeqCst) && self.client.is_available()
    }

    /// Get timing statistics
    pub fn get_timing_stats(&self) -> crate::infrastructure::lsp::TimingStats {
        self.timing.get_timing_stats()
    }

    /// Get synchronization statistics
    pub async fn get_sync_stats(&self) -> crate::infrastructure::lsp::SyncStats {
        self.sync.get_sync_stats().await
    }
}

/// Compute the LSP document URI for a call-site's source file.
///
/// Returns `None` when the file path is missing or a placeholder —
/// these values cannot produce a meaningful `file://` URI and the
/// caller should short-circuit to the heuristic fallback rather than
/// pay an LSP round-trip.
///
/// # Guarded inputs
///
/// * Empty string. Never a valid file path; indicates the extractor
///   did not populate `Range.file`.
/// * `"<unknown>"`. The `Range::new` default placeholder; emitted
///   when a range is constructed without a known source file.
///
/// # Not guarded here
///
/// Relative paths pass through unchanged. Per RFC 8089 a strict
/// `file://` URI requires an absolute path, but `Url::from_file_path`
/// cannot be used here without widening the call-site contract to
/// guarantee absolute paths. The LSP servers we target today (`rust-
/// analyzer`, `pyright`, `jdtls`) accept relative `file://` URIs when
/// the server's workspace root is set. If this ever breaks, the fix
/// is to canonicalise `call_site.location.file` against
/// `SyntaxResults::workspace_root` at this seam.
fn lsp_document_uri_for(call_site: &CallSite) -> Option<String> {
    let file = call_site.location.file.as_str();
    if file.is_empty() || file == "<unknown>" {
        return None;
    }
    Some(format!("file://{file}"))
}

/// Pure edge-construction helper for LSP call resolution.
///
/// Extracted from [`RealLspResolver::resolve_single_call`] so the
/// edge-kind dispatch is unit-testable without a live LSP client.
///
/// # Contract (post-UF-FU-005)
///
/// The emitted [`Edge::kind`] is determined by
/// [`UnresolvedReference::edge_kind`] — i.e., the typed variant of the
/// incoming reference, never a hardcoded constant. This is the
/// UF-FU-005 fix: the pre-fix `resolve_single_call` hardcoded
/// `EdgeKind::Call`, which silently mislabelled any non-Call variant
/// that reached the LSP path as a plain Call edge with
/// `Provenance::Lsp / Confidence::High`. Routing through this helper
/// ensures every consumer of LSP-resolved edges honours the typed
/// dispatch established in P1.d.
///
/// # Self-loop policy
///
/// Returns `None` when `caller_id == callee_id`. Matches the pre-fix
/// behaviour (self-loops suppressed to avoid spurious recursion edges
/// in call graphs; recursion is tracked by a separate analysis pass).
///
/// # Provenance
///
/// `Provenance::new(ProvenanceSource::Lsp, Confidence::High)` —
/// unchanged from the pre-fix code. LSP is authoritative for targets
/// it returns; confidence downgrade on ambiguous LSP responses is a
/// separate concern (tracked under the `graphengine-ra-ide-adapter`
/// design at T6).
pub fn build_resolved_call_edge(
    caller_id: String,
    callee_id: String,
    reference: &UnresolvedReference,
    function_name_for_log: &str,
) -> Option<Edge> {
    if caller_id == callee_id {
        debug!("Skipping self-loop: {} -> {}", caller_id, callee_id);
        return None;
    }
    let edge_kind = reference.edge_kind();
    debug!(
        "Resolved call: {} -> {} ({}) as {:?}",
        caller_id, callee_id, function_name_for_log, edge_kind
    );
    Some(Edge::new(
        caller_id,
        callee_id,
        edge_kind,
        Provenance::new(ProvenanceSource::Lsp, Confidence::High),
    ))
}

#[async_trait]
impl SemanticResolver for RealLspResolver {
    #[instrument(skip(self, syntax_results))]
    async fn resolve(
        &self,
        syntax_results: &SyntaxResults,
    ) -> Result<ResolvedEdges, anyhow::Error> {
        // Check if LSP is available
        if !self.is_available() {
            warn!("Real LSP server not available, returning empty edges");
            return Ok(ResolvedEdges::new());
        }

        // Execute LSP resolution with proper timing
        let result = self.timing.execute_phase(ParsingPhase::LspResolution, || {
            let resolver = self;
            async move {
                let syntax_results = syntax_results;
                // Resolve different types of relationships in parallel with synchronization
                let (call_edges, import_edges, type_edges, containment_edges) = tokio::try_join!(
                    async {
                        resolver
                            .resolve_calls(syntax_results)
                            .await
                            .map_err(|e| format!("Call resolution failed: {}", e))
                    },
                    async {
                        resolver
                            .resolve_imports(syntax_results)
                            .await
                            .map_err(|e| format!("Import resolution failed: {}", e))
                    },
                    async {
                        resolver
                            .resolve_types(syntax_results)
                            .await
                            .map_err(|e| format!("Type resolution failed: {}", e))
                    },
                    async {
                        resolver
                            .resolve_containment(syntax_results)
                            .await
                            .map_err(|e| format!("Containment resolution failed: {}", e))
                    },
                )?;

                Ok::<ResolvedEdges, String>(ResolvedEdges {
                    call_edges,
                    import_edges,
                    type_edges,
                    containment_edges,
                    stats: ResolutionStatsSummary::new(),
                    resolved_call_sites: std::collections::HashSet::new(),
                })
            }
        }).await;

        match result {
            Ok(resolved_edges) => {
                let total_edges = resolved_edges.call_edges.len()
                    + resolved_edges.import_edges.len()
                    + resolved_edges.type_edges.len()
                    + resolved_edges.containment_edges.len();

                info!("Real LSP resolution complete: {} total edges", total_edges);
                Ok(resolved_edges)
            }
            Err(e) => {
                error!("Real LSP resolution failed: {}", e);
                Err(anyhow::anyhow!("LSP resolution failed: {}", e))
            }
        }
    }

    async fn is_available(&self) -> bool {
        self.is_available()
    }

    fn supported_language(&self) -> &str {
        &self.config.language
    }
}

impl Drop for RealLspResolver {
    fn drop(&mut self) {
        // Ensure proper cleanup
        if self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            // Shutdown LSP client
            let client = Arc::clone(&self.client);
            tokio::spawn(async move {
                if let Err(e) = client.shutdown().await {
                    warn!("Failed to shutdown LSP client: {}", e);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Range;

    fn synthetic_call_site(file: &str) -> CallSite {
        CallSite {
            location: Range::with_file(1, 0, 1, 10, file.to_string()),
            function_name: "f".to_string(),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        }
    }

    #[test]
    fn real_resolver_empty_path_short_circuits() {
        // UF-FU-006 regression gate. The LSP document URI helper must
        // return `None` for the empty-path case so the caller skips
        // `get_document_symbols` entirely — avoiding an LSP round-trip
        // with a malformed URI and preventing the pre-fix "file://"
        // placeholder from landing on the wire.
        let call_site = synthetic_call_site("");
        assert_eq!(
            lsp_document_uri_for(&call_site),
            None,
            "empty file path must short-circuit the LSP query"
        );
    }

    #[test]
    fn real_resolver_unknown_placeholder_short_circuits() {
        // `Range::new` (no `with_file`) populates `<unknown>` as the
        // file placeholder. That value must not reach the LSP server
        // either — same reasoning as the empty-path case.
        let call_site = synthetic_call_site("<unknown>");
        assert_eq!(
            lsp_document_uri_for(&call_site),
            None,
            "`<unknown>` placeholder must short-circuit the LSP query"
        );
    }

    #[test]
    fn real_resolver_real_path_returns_file_uri() {
        let call_site = synthetic_call_site("/abs/path/to/Controller.cls");
        assert_eq!(
            lsp_document_uri_for(&call_site),
            Some("file:///abs/path/to/Controller.cls".to_string()),
            "real path must produce a `file://` URI; the pre-fix \
             hardcoded `dummy_path` produced `file://dummy_path`, \
             which UF-FU-006 replaced."
        );
    }

    #[test]
    fn real_resolver_relative_path_returns_file_uri_unchanged() {
        // Relative paths currently pass through as-is; canonicalisation
        // against the workspace root is out of scope for UF-FU-006.
        // Locking present behaviour so a future canonicalisation change
        // surfaces as an intentional test edit.
        let call_site = synthetic_call_site("force-app/main/default/classes/Account.cls");
        assert_eq!(
            lsp_document_uri_for(&call_site),
            Some("file://force-app/main/default/classes/Account.cls".to_string())
        );
    }

    #[tokio::test]
    async fn test_real_lsp_resolver_creation() {
        let config = LanguageConfig::new(
            "rust".to_string(),
            vec![".rs".to_string()],
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        );

        // This test will fail because we don't have a real LSP server running
        // but it demonstrates the API
        let result = RealLspResolver::new(config, None, None).await;
        assert!(result.is_err()); // Expected to fail without real LSP server
    }

    #[tokio::test]
    async fn test_range_containment() {
        let resolver = RealLspResolver {
            client: Arc::new(LspClient::new(
                LanguageConfig::new(
                    "rust".to_string(),
                    vec![".rs".to_string()],
                    std::collections::HashMap::new(),
                    std::collections::HashMap::new(),
                ),
                None,
                10,
            )),
            timing: Arc::new(TimingCoordinator::new(TimingConfig::default(), 10)),
            sync: Arc::new(SynchronizationManager::new(10)),
            config: Arc::new(LanguageConfig::new(
                "rust".to_string(),
                vec![".rs".to_string()],
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
            )),
            initialized: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        let inner = crate::domain::Range {
            start_line: 10,
            end_line: 15,
            start_char: 0,
            end_char: 10,
            file: "test.rs".to_string(),
        };

        let outer = crate::domain::Range {
            start_line: 5,
            end_line: 20,
            start_char: 0,
            end_char: 20,
            file: "test.rs".to_string(),
        };

        assert!(resolver.is_within_range(&inner, &outer));
    }
}
