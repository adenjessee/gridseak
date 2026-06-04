//! Mock LSP-based semantic resolver for testing the parsing pipeline
//! without a live language-server process.
//!
//! Relocated from `graphengine-parsing/src/infrastructure/lsp/mock_resolver.rs`
//! by R2 (v0.1.0-rc1 follow-up). Behaviour identical to the original;
//! `use crate::…` paths were rewritten to consume the parsing crate's
//! public API.

use crate::mock_lsp_session::{MockSessionState, MockSessionSupervisor};
use async_trait::async_trait;
use graphengine_parsing::application::ports::{
    ResolutionStatsSummary, ResolvedEdges, SemanticResolver, SyntaxResults,
};
use graphengine_parsing::domain::{
    Confidence, Edge, EdgeKind, NodeKind, Provenance, ProvenanceSource,
};
use graphengine_parsing::infrastructure::config::LanguageConfig;
use graphengine_parsing::infrastructure::lsp::errors::LspError;
use std::sync::Arc;
use tracing::{info, instrument, warn};
use url::Url;

/// Mock LSP-based semantic resolver
pub struct MockLspResolver {
    /// Language configuration
    config: Arc<LanguageConfig>,
    /// Mock session supervisor for LSP server management
    session_supervisor: Arc<MockSessionSupervisor>,
}

impl MockLspResolver {
    /// Create a new mock LSP resolver
    pub fn new(config: LanguageConfig, workspace_root: Option<Url>) -> Self {
        let session_supervisor =
            Arc::new(MockSessionSupervisor::new(config.clone(), workspace_root));

        Self {
            config: Arc::new(config),
            session_supervisor,
        }
    }

    /// Check if the LSP resolver is available
    pub async fn is_available(&self) -> bool {
        self.session_supervisor.get_state().is_functional()
    }

    /// Set the session state (for testing)
    pub fn set_session_state(&self, state: MockSessionState) {
        self.session_supervisor.set_state(state);
    }

    #[instrument(skip(self, syntax_results))]
    async fn resolve_calls(&self, syntax_results: &SyntaxResults) -> Result<Vec<Edge>, LspError> {
        let mut edges = Vec::new();
        let functions: Vec<_> = syntax_results
            .symbols
            .iter()
            .filter(|node| node.kind == NodeKind::Function)
            .collect();

        for (i, from_func) in functions.iter().enumerate() {
            if let Some(to_func) = functions.get(i + 1) {
                let edge = Edge {
                    from_id: from_func.id.clone(),
                    to_id: to_func.id.clone(),
                    kind: EdgeKind::Call,
                    provenance: Provenance::new(ProvenanceSource::Lsp, Confidence::High),
                };
                edges.push(edge);
            }
        }

        info!("Mock resolved {} call edges", edges.len());
        Ok(edges)
    }

    #[instrument(skip(self, syntax_results))]
    async fn resolve_imports(&self, syntax_results: &SyntaxResults) -> Result<Vec<Edge>, LspError> {
        let mut edges = Vec::new();
        let modules: Vec<_> = syntax_results
            .symbols
            .iter()
            .filter(|node| node.kind == NodeKind::Module)
            .collect();

        for (i, from_module) in modules.iter().enumerate() {
            if let Some(to_module) = modules.get(i + 1) {
                let edge = Edge {
                    from_id: from_module.id.clone(),
                    to_id: to_module.id.clone(),
                    kind: EdgeKind::Import,
                    provenance: Provenance::new(ProvenanceSource::Lsp, Confidence::High),
                };
                edges.push(edge);
            }
        }

        info!("Mock resolved {} import edges", edges.len());
        Ok(edges)
    }

    #[instrument(skip(self, syntax_results))]
    async fn resolve_types(&self, syntax_results: &SyntaxResults) -> Result<Vec<Edge>, LspError> {
        let mut edges = Vec::new();
        let structs: Vec<_> = syntax_results
            .symbols
            .iter()
            .filter(|node| node.kind == NodeKind::Struct)
            .collect();

        for (i, from_struct) in structs.iter().enumerate() {
            if let Some(to_struct) = structs.get(i + 1) {
                let edge = Edge {
                    from_id: from_struct.id.clone(),
                    to_id: to_struct.id.clone(),
                    kind: EdgeKind::Type,
                    provenance: Provenance::new(ProvenanceSource::Lsp, Confidence::High),
                };
                edges.push(edge);
            }
        }

        info!("Mock resolved {} type edges", edges.len());
        Ok(edges)
    }

    #[instrument(skip(self, syntax_results))]
    async fn resolve_containment(
        &self,
        syntax_results: &SyntaxResults,
    ) -> Result<Vec<Edge>, LspError> {
        let mut edges = Vec::new();
        let modules: Vec<_> = syntax_results
            .symbols
            .iter()
            .filter(|node| node.kind == NodeKind::Module)
            .collect();
        let functions: Vec<_> = syntax_results
            .symbols
            .iter()
            .filter(|node| node.kind == NodeKind::Function)
            .collect();

        for (i, module) in modules.iter().enumerate() {
            if let Some(function) = functions.get(i) {
                let edge = Edge {
                    from_id: module.id.clone(),
                    to_id: function.id.clone(),
                    kind: EdgeKind::Contains,
                    provenance: Provenance::new(ProvenanceSource::Lsp, Confidence::High),
                };
                edges.push(edge);
            }
        }

        info!("Mock resolved {} containment edges", edges.len());
        Ok(edges)
    }
}

#[async_trait]
impl SemanticResolver for MockLspResolver {
    #[instrument(skip(self, syntax_results))]
    async fn resolve(
        &self,
        syntax_results: &SyntaxResults,
    ) -> Result<ResolvedEdges, anyhow::Error> {
        info!(
            "Starting mock LSP semantic resolution for {} symbols",
            syntax_results.symbols.len()
        );

        if !self.is_available().await {
            warn!("Mock LSP server not available, returning empty edges");
            return Ok(ResolvedEdges::new());
        }

        let (call_edges, import_edges, type_edges, containment_edges) = tokio::try_join!(
            async {
                self.resolve_calls(syntax_results)
                    .await
                    .map_err(|e| anyhow::anyhow!("Call resolution failed: {}", e))
            },
            async {
                self.resolve_imports(syntax_results)
                    .await
                    .map_err(|e| anyhow::anyhow!("Import resolution failed: {}", e))
            },
            async {
                self.resolve_types(syntax_results)
                    .await
                    .map_err(|e| anyhow::anyhow!("Type resolution failed: {}", e))
            },
            async {
                self.resolve_containment(syntax_results)
                    .await
                    .map_err(|e| anyhow::anyhow!("Containment resolution failed: {}", e))
            },
        )?;

        let resolved_edges = ResolvedEdges {
            call_edges,
            import_edges,
            type_edges,
            containment_edges,
            stats: ResolutionStatsSummary::new(),
            resolved_call_sites: std::collections::HashSet::new(),
        };

        info!(
            "Mock LSP resolution complete: {} call edges, {} import edges, {} type edges, {} containment edges",
            resolved_edges.call_edges.len(),
            resolved_edges.import_edges.len(),
            resolved_edges.type_edges.len(),
            resolved_edges.containment_edges.len()
        );

        Ok(resolved_edges)
    }

    async fn is_available(&self) -> bool {
        self.session_supervisor.get_state().is_functional()
    }

    fn supported_language(&self) -> &str {
        &self.config.language
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphengine_parsing::domain::{Node, NodeKind, Range};
    use graphengine_parsing::infrastructure::config::create_default_rust_config;

    #[test]
    fn test_mock_lsp_resolver_creation() {
        let config = create_default_rust_config();
        let _resolver = MockLspResolver::new(config, None);
    }

    #[test]
    fn test_mock_lsp_resolver_availability() {
        let config = create_default_rust_config();
        let _resolver = MockLspResolver::new(config, None);
    }

    #[tokio::test]
    async fn test_resolve_empty_syntax_results() {
        let config = create_default_rust_config();
        let resolver = MockLspResolver::new(config, None);
        let syntax_results = SyntaxResults::new();
        let result = resolver.resolve(&syntax_results).await;
        assert!(result.is_ok());
        let resolved_edges = result.unwrap();
        assert!(resolved_edges.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_with_syntax_results() {
        let config = create_default_rust_config();
        let resolver = MockLspResolver::new(config, None);
        let mut syntax_results = SyntaxResults::new();

        let node = Node::new(
            NodeKind::Function,
            "test::function".to_string(),
            Range::with_file(1, 0, 5, 10, "mock.rs".to_string()),
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
        );
        syntax_results.add_symbol(node);

        syntax_results.add_call_site(
            Range::with_file(2, 0, 2, 10, "mock.rs".to_string()),
            "mock_call".to_string(),
        );

        let result = resolver.resolve(&syntax_results).await;
        assert!(result.is_ok());
        let resolved_edges = result.unwrap();
        assert!(resolved_edges.is_empty());
    }
}
