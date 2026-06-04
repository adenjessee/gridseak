//! Semantic resolution with LSP and fallback strategies

use super::super::super::super::errors::ParsingError;
use super::super::super::super::ports::{
    GlobalSymbolTable, ResolvedEdges, SemanticResolver, SyntaxResults,
};
use super::super::resolution::fallback::FallbackEdgeBuilder;
use tracing::{debug, info, warn};

/// Semantic resolver service
pub struct SemanticResolverService;

impl SemanticResolverService {
    /// Resolve semantics with LSP-first strategy and fallback
    ///
    /// Strategy:
    /// 1) Run the primary resolver (LSP-first)
    /// 2) Add fallback call edges for unresolved call sites using the global table when there is a unique target
    ///
    /// # Arguments
    /// * `syntax_results` - Syntax extraction results
    /// * `global` - Global symbol table
    /// * `semantic_resolver` - LSP resolver implementation
    ///
    /// # Returns
    /// * `ResolvedEdges` - Resolved semantic relationships
    /// * `ParsingError` - If resolution fails
    pub async fn resolve_with_fallback(
        syntax_results: &SyntaxResults,
        global: &GlobalSymbolTable,
        semantic_resolver: &dyn SemanticResolver,
    ) -> Result<ResolvedEdges, ParsingError> {
        debug!("Starting resolve_semantics_with_global_context");

        // Step 1: Run primary resolver (LSP)
        let mut resolved = Self::resolve_with_lsp(syntax_results, semantic_resolver).await?;
        debug!("resolve_semantics completed, now doing global context processing");

        // Step 2: Add fallback edges for unresolved call sites
        resolved = FallbackEdgeBuilder::create_fallback_edges(syntax_results, global, resolved)?;

        Ok(resolved)
    }

    /// Resolve semantics using LSP resolver
    async fn resolve_with_lsp(
        syntax_results: &SyntaxResults,
        semantic_resolver: &dyn SemanticResolver,
    ) -> Result<ResolvedEdges, ParsingError> {
        debug!("Checking if semantic resolver is available...");
        // Check if resolver is available
        let is_available = semantic_resolver.is_available().await;
        info!("Semantic resolver is available: {}", is_available);
        debug!("Semantic resolver availability check completed");

        if !is_available {
            warn!("Semantic resolver not available, returning empty edges");
            return Ok(ResolvedEdges::new());
        }

        info!(
            "Starting semantic resolution with {} symbols",
            syntax_results.symbols.len()
        );
        let result = semantic_resolver.resolve(syntax_results).await;
        match &result {
            Ok(edges) => {
                info!(
                    "Semantic resolution succeeded with {} edges",
                    edges.all_edges().len()
                );
            }
            Err(e) => {
                warn!("Semantic resolution failed: {}", e);
            }
        }
        result.map_err(|e| ParsingError::resolution(format!("Semantic resolution failed: {}", e)))
    }
}
