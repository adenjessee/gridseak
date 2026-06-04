//! Main use case for parsing a repository into a semantic graph
//!
//! Orchestrates the complete pipeline from workspace discovery
//! to validated graph storage.

use super::super::super::errors::ParsingError;
use super::super::super::ports::{
    GraphRepository, ResolutionStatsSummary, SemanticResolver, SessionMetricsSnapshot,
    SyntaxExtractor,
};
use super::factory::UseCaseFactory;
use crate::domain::{Confidence, Graph};
use graphengine_progress::EngineEventEmitter;
use std::sync::Arc;
use tracing::instrument;

/// Type-safe wrapper around a validated graph
/// Ensures the graph has passed validation with the required confidence level
#[derive(Debug, Clone)]
pub struct ResolvedGraph {
    graph: Graph,
    stats: ResolutionStatsSummary,
    /// LSP session-lifecycle metrics captured at end-of-scan. `None`
    /// for resolvers without an underlying LSP (mocks, pure-heuristic
    /// dispatchers in LSP-disabled configurations). Used by the CLI
    /// to answer "did LSP actually come up?" in `--lsp-telemetry`
    /// output.
    session_metrics: Option<SessionMetricsSnapshot>,
}

impl ResolvedGraph {
    pub fn new(graph: Graph, stats: ResolutionStatsSummary) -> Self {
        Self {
            graph,
            stats,
            session_metrics: None,
        }
    }

    /// Construct with session metrics attached. Preferred by the
    /// orchestrator over mutating after construction because it makes
    /// the "this field may be present" contract explicit at the call
    /// site.
    pub fn with_session_metrics(
        graph: Graph,
        stats: ResolutionStatsSummary,
        session_metrics: Option<SessionMetricsSnapshot>,
    ) -> Self {
        Self {
            graph,
            stats,
            session_metrics,
        }
    }

    /// Get the underlying graph
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Get the number of nodes in the resolved graph
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Get the number of edges in the resolved graph
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Check if the graph is empty
    pub fn is_empty(&self) -> bool {
        self.graph.is_empty()
    }

    pub fn stats(&self) -> &ResolutionStatsSummary {
        &self.stats
    }

    /// LSP session-lifecycle metrics, if the resolver reported them.
    /// `None` means either the resolver isn't LSP-backed, or the LSP
    /// tier never produced a session snapshot for this scan.
    pub fn session_metrics(&self) -> Option<&SessionMetricsSnapshot> {
        self.session_metrics.as_ref()
    }
}

/// Main use case for parsing a repository into a semantic graph
///
/// This orchestrates the complete pipeline:
/// 1. Load configuration (stub for now)
/// 2. Discover source files
/// 3. Extract syntax information
/// 4. Resolve semantic relationships
/// 5. Build and validate the graph
/// 6. Persist the results
pub struct ParseRepositoryUseCase {
    syntax_extractor: Box<dyn SyntaxExtractor>,
    semantic_resolver: Box<dyn SemanticResolver>,
    graph_repo: Box<dyn GraphRepository>,
    min_confidence: Confidence,
    progress_emitter: Option<Arc<dyn EngineEventEmitter>>,
}

impl ParseRepositoryUseCase {
    /// Create a new parse repository use case
    ///
    /// # Arguments
    /// * `syntax_extractor` - Extractor for syntax analysis
    /// * `semantic_resolver` - Resolver for semantic analysis
    /// * `graph_repo` - Repository for graph persistence
    /// * `min_confidence` - Minimum confidence level for validation
    pub fn new(
        syntax_extractor: Box<dyn SyntaxExtractor>,
        semantic_resolver: Box<dyn SemanticResolver>,
        graph_repo: Box<dyn GraphRepository>,
        min_confidence: Confidence,
    ) -> Self {
        Self {
            syntax_extractor,
            semantic_resolver,
            graph_repo,
            min_confidence,
            progress_emitter: None,
        }
    }

    /// Attach a progress emitter for pipeline-level phase reporting.
    pub fn set_progress_emitter(&mut self, emitter: Arc<dyn EngineEventEmitter>) {
        self.progress_emitter = Some(emitter);
    }

    // Convenience factory methods that delegate to UseCaseFactory.

    /// Create a use case with SQLite storage for persistent graph data
    ///
    /// # Arguments
    /// * `language` - Programming language to parse
    /// * `min_confidence` - Minimum confidence level for validation
    /// * `db_path` - Path to the SQLite database file
    ///
    /// # Returns
    /// * `Result<Self, ParsingError>` - The configured use case or an error
    pub async fn with_sqlite_storage(
        language: String,
        min_confidence: Confidence,
        db_path: &str,
    ) -> Result<Self, ParsingError> {
        UseCaseFactory::with_sqlite_storage(language, min_confidence, db_path).await
    }

    /// Create a use case with real Tree-sitter + mock LSP + SQLite
    ///
    /// # Arguments
    /// * `language` - Programming language to parse
    /// * `min_confidence` - Minimum confidence level for validation
    /// * `db_path` - Path to the SQLite database file
    ///
    /// # Returns
    /// * `Result<Self, ParsingError>` - The configured use case or an error
    pub async fn with_real_syntax_extraction(
        language: String,
        min_confidence: Confidence,
        db_path: &str,
    ) -> Result<Self, ParsingError> {
        UseCaseFactory::with_real_syntax_extraction(language, min_confidence, db_path).await
    }

    /// Create a use case with real Tree-sitter + real LSP + SQLite
    ///
    /// # Arguments
    /// * `language` - Programming language to parse
    /// * `min_confidence` - Minimum confidence level for validation
    /// * `db_path` - Path to the SQLite database file
    /// * `workspace_root` - Optional workspace root for LSP
    ///
    /// # Returns
    /// * `Result<Self, ParsingError>` - The configured use case or an error
    pub async fn with_real_components(
        language: String,
        min_confidence: Confidence,
        db_path: &str,
        workspace_root: Option<url::Url>,
    ) -> Result<Self, ParsingError> {
        UseCaseFactory::with_real_components(language, min_confidence, db_path, workspace_root)
            .await
    }

    /// Create a use case with real components and per-file progress reporting.
    pub async fn with_real_components_progress(
        language: String,
        min_confidence: Confidence,
        db_path: &str,
        workspace_root: Option<url::Url>,
        progress_emitter: Arc<dyn EngineEventEmitter>,
    ) -> Result<Self, ParsingError> {
        UseCaseFactory::with_real_components_progress(
            language,
            min_confidence,
            db_path,
            workspace_root,
            Some(progress_emitter),
        )
        .await
    }

    // `with_in_memory_storage` and `with_infrastructure` were deleted
    // by R2 (v0.1.0-rc1 follow-up). Both constructors instantiated
    // mock components that now live in
    // `graphengine-parsing-test-support`. Their callers — two
    // integration tests + one bench — construct the use case directly
    // via `ParseRepositoryUseCase::new(...)` with mocks imported from
    // that crate. See `tests/parse_repo_use_case_smoke.rs`,
    // `tests/infrastructure_tests.rs`, and
    // `tests/infrastructure/benches/infrastructure_bench.rs`.

    /// Parse a repository into a semantic graph using default options
    /// (incremental scanning enabled).
    ///
    /// # Arguments
    /// * `root` - Root directory of the repository
    /// * `language` - Programming language to parse
    ///
    /// # Returns
    /// * `ResolvedGraph` - Validated semantic graph
    /// * `ParsingError` - If parsing fails at any stage
    #[instrument(skip(self), fields(language = %language, root = %root.display()))]
    pub async fn parse(
        &self,
        root: std::path::PathBuf,
        language: String,
    ) -> Result<ResolvedGraph, ParsingError> {
        self.parse_with_options(
            root,
            language,
            super::pipeline::orchestrator::ParseOptions::default(),
        )
        .await
    }

    /// Parse a repository into a semantic graph with explicit per-scan
    /// options. The CLI's `--no-incremental` flag flows through here
    /// as `ParseOptions { incremental: false }`. New per-scan toggles
    /// extend `ParseOptions` rather than adding parameters to this
    /// method.
    #[instrument(skip(self, options), fields(language = %language, root = %root.display(), incremental = options.incremental))]
    pub async fn parse_with_options(
        &self,
        root: std::path::PathBuf,
        language: String,
        options: super::pipeline::orchestrator::ParseOptions,
    ) -> Result<ResolvedGraph, ParsingError> {
        super::pipeline::orchestrator::ParsingPipeline::execute_with_progress(
            root,
            language,
            self.syntax_extractor.as_ref(),
            self.semantic_resolver.as_ref(),
            self.graph_repo.as_ref(),
            self.min_confidence,
            self.progress_emitter.clone(),
            options,
        )
        .await
    }
}

// `#[cfg(test)] mod tests` block was relocated to
// `tests/parse_repo_use_case_smoke.rs` by R2 (v0.1.0-rc1 follow-up)
// so the mocks it depended on could be deleted from `src/`. The tests
// themselves are unchanged.
