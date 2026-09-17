//! Factory methods for creating ParseRepositoryUseCase instances
//!
//! Provides various factory methods to create use cases with different
//! component configurations (mock, real, SQLite, in-memory, etc.)

use super::super::super::errors::ParsingError;
use super::use_case::ParseRepositoryUseCase;
use crate::application::ports::SemanticResolver;
use crate::domain::Confidence;
use crate::infrastructure::lsp::build_session_options;
use crate::infrastructure::{load_config, LspResolver, SqliteRepository};
use crate::syntax::language::apex::{
    build_apex_session_options, ApexHeuristicResolver, ApexResolverDispatcher,
};
use crate::syntax::TreeSitterExtractor;
use graphengine_progress::EngineEventEmitter;
use std::sync::Arc;
use tracing::info;

/// Factory for creating ParseRepositoryUseCase instances
pub struct UseCaseFactory;

impl UseCaseFactory {
    // `with_infrastructure` (and the `use_case::with_infrastructure`
    // re-export) used to live here, instantiating
    // `MockSyntaxExtractor` + `MockLspResolver` +
    // `MockGraphRepository`. R2 (v0.1.0-rc1 follow-up) deleted both
    // entry points because their only callers were two tests and one
    // bench, and the mocks themselves were relocated to the
    // `graphengine-parsing-test-support` dev-dependency crate. Test
    // and bench callers now construct the use case directly via
    // `ParseRepositoryUseCase::new(...)` with mocks imported from
    // that crate. See `tests/parse_repo_use_case_smoke.rs`,
    // `tests/infrastructure_tests.rs`, and
    // `tests/infrastructure/benches/infrastructure_bench.rs`.

    /// Create a use case with SQLite storage for persistent graph data
    ///
    /// # Arguments
    /// * `language` - Programming language to parse
    /// * `min_confidence` - Minimum confidence level for validation
    /// * `db_path` - Path to the SQLite database file
    ///
    /// # Returns
    /// * `Result<ParseRepositoryUseCase, ParsingError>` - The configured use case or an error
    pub async fn with_sqlite_storage(
        language: String,
        min_confidence: Confidence,
        db_path: &str,
    ) -> Result<ParseRepositoryUseCase, ParsingError> {
        Self::with_real_components(language, min_confidence, db_path, None).await
    }

    /// Create a use case with real Tree-sitter + mock LSP + SQLite
    ///
    /// # Arguments
    /// * `language` - Programming language to parse
    /// * `min_confidence` - Minimum confidence level for validation
    /// * `db_path` - Path to the SQLite database file
    ///
    /// # Returns
    /// * `Result<ParseRepositoryUseCase, ParsingError>` - The configured use case or an error
    pub async fn with_real_syntax_extraction(
        language: String,
        min_confidence: Confidence,
        db_path: &str,
    ) -> Result<ParseRepositoryUseCase, ParsingError> {
        Self::with_real_components(language, min_confidence, db_path, None).await
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
    /// * `Result<ParseRepositoryUseCase, ParsingError>` - The configured use case or an error
    pub async fn with_real_components(
        language: String,
        min_confidence: Confidence,
        db_path: &str,
        workspace_root: Option<url::Url>,
    ) -> Result<ParseRepositoryUseCase, ParsingError> {
        Self::with_real_components_progress(language, min_confidence, db_path, workspace_root, None)
            .await
    }

    /// Create a use case with real components and an optional progress emitter
    /// for per-file progress reporting during syntax extraction.
    pub async fn with_real_components_progress(
        language: String,
        min_confidence: Confidence,
        db_path: &str,
        workspace_root: Option<url::Url>,
        progress_emitter: Option<Arc<dyn EngineEventEmitter>>,
    ) -> Result<ParseRepositoryUseCase, ParsingError> {
        info!(
            "Creating use case with REAL Tree-sitter + REAL LSP + SQLite for language: {}",
            language
        );

        // Load configuration for the language
        let config = load_config(&language).map_err(|e| {
            ParsingError::config(format!("Failed to load config for {}: {}", language, e))
        })?;

        info!("Configuration loaded successfully");
        info!("About to create TreeSitterExtractor");

        // Create REAL Tree-sitter syntax extractor
        let mut extractor = TreeSitterExtractor::new(config.clone()).map_err(|e| {
            ParsingError::config(format!("Failed to create TreeSitterExtractor: {}", e))
        })?;

        // Set workspace root for FQN construction (files outside src/ use relative paths)
        if let Some(ref ws_root) = workspace_root {
            if let Ok(path) = ws_root.to_file_path() {
                extractor.set_workspace_root(path.to_string_lossy().to_string());
            }
        }

        // Attach progress emitter if provided (clone Arc for tree-sitter; keep original for pipeline)
        if let Some(ref emitter) = progress_emitter {
            extractor.set_progress_emitter(Arc::clone(emitter));
        }

        let syntax_extractor = Box::new(extractor);

        info!("Created TreeSitterExtractor for language: {}", language);
        info!("About to create LspResolver");

        // Language-keyed semantic-resolver selection. Four branches:
        //
        // - **apex** — wrap the jorje LSP in the Apex dispatcher so we
        //   fall back to the pure-Rust heuristic when Java / jorje is
        //   missing. Keeps Apex first-class on machines without a JDK.
        // - **rust** (feature `rust-layer2`, default-on post-Gate 1.2)
        //   — instantiate `RustLayer2SemanticResolver`, which links
        //   `ra_ap_ide` directly. Falls through to the default LSP
        //   path if the workspace-root lookup fails (no `Cargo.toml`
        //   visible, sysroot discovery blew up, etc.) so the scan
        //   still produces heuristic edges rather than aborting.
        // - **rust** (feature disabled) or **any other language** —
        //   the subprocess-LSP path via `LspResolver::new`. Behaviour
        //   unchanged from pre-T6.
        //
        // The `rust-layer2` feature gate sits at `Cargo.toml` level,
        // not runtime, so a `cargo build --no-default-features`
        // deliberately yields the old subprocess-LSP path.
        let semantic_resolver: Box<dyn SemanticResolver> = if language == "apex" {
            // F.2: detect the SFDX layout now so jorje receives every
            // `packageDirectories` entry as a `workspaceFolders`
            // member, and hold on to the canary class we'll use to
            // wait for indexing before issuing `textDocument/definition`.
            let session_options = build_apex_session_options(workspace_root.as_ref());
            let lsp = Arc::new(LspResolver::with_options(
                config,
                workspace_root,
                session_options,
            ));
            // Construction uses the preload-only registry; user-declared
            // class symbols are layered onto it inside every
            // `SemanticResolver::resolve` call via
            // `seed_registry_from_hints` (see
            // `apex::heuristic_resolver`). Pre-seeding here would
            // require a second parse pass — the runtime seeding step
            // keeps the factory free of parse dependencies and is the
            // resolution to FOLLOWUP_RISKS.md R37.
            let heuristic = Arc::new(ApexHeuristicResolver::with_standard_preload_only());
            Box::new(ApexResolverDispatcher::new(lsp, heuristic))
        } else if language == "rust" {
            build_rust_semantic_resolver(config, workspace_root.as_ref())
        } else if language == "typescript" || language == "javascript" {
            build_index_backed_or_heuristic("typescript", workspace_root.as_ref(), config)
        } else if language == "python" {
            build_index_backed_or_heuristic("python", workspace_root.as_ref(), config)
        } else if language == "go" {
            build_index_backed_or_heuristic("go", workspace_root.as_ref(), config)
        } else {
            let session_options = build_session_options(&language, workspace_root.as_ref());
            Box::new(LspResolver::with_options(
                config,
                workspace_root,
                session_options,
            ))
        };

        info!(
            "Created semantic resolver (apex-dispatcher={})",
            language == "apex"
        );

        // Use real SQLite repository
        let graph_repo = Box::new(SqliteRepository::new(db_path).map_err(|e| {
            ParsingError::config(format!("Failed to create SQLite repository: {}", e))
        })?);

        info!("REAL Tree-sitter + REAL LSP + SQLite components created successfully");

        let mut use_case = ParseRepositoryUseCase::new(
            syntax_extractor,
            semantic_resolver,
            graph_repo,
            min_confidence,
        );

        // Also pass the emitter to the use case so the pipeline orchestrator
        // can emit phase-level progress (discovery, resolution, graph build, persist).
        if let Some(emitter) = progress_emitter {
            use_case.set_progress_emitter(emitter);
        }

        Ok(use_case)
    }

    // `with_in_memory_storage` was deleted alongside
    // `with_infrastructure` in R2. Its sole caller (one integration
    // test, `tests/application/cross_file_calls.rs`) was rewritten to
    // construct the use case directly with mocks imported from
    // `graphengine-parsing-test-support`.
}

/// Build the Rust scan's semantic resolver. When the `rust-layer2`
/// feature is enabled (default post-Gate 1.2), we try to construct
/// the `ra_ap_ide`-backed `RustLayer2SemanticResolver` against the
/// workspace root. If that fails (no `Cargo.toml` visible, sysroot
/// discovery error, etc.) or the feature is disabled, we fall back
/// to the subprocess-LSP path so the scan still produces heuristic
/// edges and does not abort.
#[cfg(feature = "rust-layer2")]
fn build_rust_semantic_resolver(
    config: crate::infrastructure::LanguageConfig,
    workspace_root: Option<&url::Url>,
) -> Box<dyn SemanticResolver> {
    use crate::application::resolution_disclosure::{ResolutionDisclosure, SkipReason};
    use crate::infrastructure::{DisclosedSemanticResolver, RustLayer2SemanticResolver};

    let ws_path = workspace_root
        .and_then(|u| u.to_file_path().ok())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    match RustLayer2SemanticResolver::new(&ws_path) {
        Ok(resolver) => {
            info!(
                "Rust Layer-2 adapter initialised against workspace at {}",
                ws_path.display()
            );
            Box::new(resolver)
        }
        Err(err) => {
            info!(
                "Rust Layer-2 adapter unavailable ({}); falling back to subprocess-LSP path",
                err
            );
            let ws_clone = workspace_root.cloned();
            let session_options = build_session_options("rust", workspace_root);
            let lsp = Box::new(LspResolver::with_options(config, ws_clone, session_options));
            Box::new(DisclosedSemanticResolver::new(
                lsp,
                ResolutionDisclosure::layer2_fallback_to_lsp("rust", SkipReason::AdapterInitFailed),
            ))
        }
    }
}

#[cfg(not(feature = "rust-layer2"))]
fn build_rust_semantic_resolver(
    config: crate::infrastructure::LanguageConfig,
    workspace_root: Option<&url::Url>,
) -> Box<dyn SemanticResolver> {
    Box::new(LspResolver::with_options(
        config,
        workspace_root.cloned(),
        build_session_options("rust", workspace_root),
    ))
}

/// Resolver that emits no semantic edges — used when the batch indexer fails
/// and the pipeline falls through to heuristic fallback only.
#[cfg(feature = "scip")]
struct HeuristicOnlySemanticResolver {
    language: String,
}

#[cfg(feature = "scip")]
impl HeuristicOnlySemanticResolver {
    fn new(language: &str) -> Self {
        Self {
            language: language.to_string(),
        }
    }
}

#[cfg(feature = "scip")]
#[async_trait::async_trait]
impl SemanticResolver for HeuristicOnlySemanticResolver {
    async fn resolve(
        &self,
        _hints: &crate::application::ports::SyntaxResults,
    ) -> anyhow::Result<crate::application::ports::ResolvedEdges> {
        Ok(crate::application::ports::ResolvedEdges::new())
    }

    fn supported_language(&self) -> &str {
        &self.language
    }

    async fn is_available(&self) -> bool {
        true
    }
}

#[cfg(feature = "scip")]
fn build_index_backed_or_heuristic(
    language: &str,
    workspace_root: Option<&url::Url>,
    _config: crate::infrastructure::LanguageConfig,
) -> Box<dyn SemanticResolver> {
    use crate::application::resolution_disclosure::{ResolutionDisclosure, SkipReason};
    use crate::infrastructure::{
        DisclosedSemanticResolver, IndexBackedSemanticResolver, ScipSemanticIndex,
    };
    use graphengine_scip_adapter::fingerprint::{fingerprint_workspace, is_fresh, write_sidecar};
    use graphengine_scip_adapter::{provision_index, IndexerError, IndexerLanguage};
    use tracing::warn;

    let indexer_lang = match language {
        "typescript" | "javascript" => IndexerLanguage::TypeScript,
        "python" => IndexerLanguage::Python,
        "go" => IndexerLanguage::Go,
        other => {
            warn!("no SCIP registry mapping for {other}; heuristic only");
            return Box::new(DisclosedSemanticResolver::new(
                Box::new(HeuristicOnlySemanticResolver::new(language)),
                ResolutionDisclosure::layer2_fallback_to_heuristic(
                    language,
                    SkipReason::LanguageNotRouted,
                ),
            ));
        }
    };

    let ws_path = workspace_root
        .and_then(|u| u.to_file_path().ok())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let skip_from_err = |err: &IndexerError| -> SkipReason {
        if err.not_provisioned() {
            SkipReason::IndexerNotProvisioned
        } else {
            SkipReason::IndexerFailed
        }
    };

    let heuristic = |reason: SkipReason| -> Box<dyn SemanticResolver> {
        Box::new(DisclosedSemanticResolver::new(
            Box::new(HeuristicOnlySemanticResolver::new(language)),
            ResolutionDisclosure::layer2_fallback_to_heuristic(language, reason),
        ))
    };

    let index_path = ws_path.join("index.scip");
    let stale = index_path.is_file() && !is_fresh(&ws_path, indexer_lang);
    if stale {
        warn!("{language} SCIP index fingerprint mismatch (IndexStale)");
        // Still load the file if present; disclose staleness after load fails
        // only when we cannot reuse. Prefer cache from provision_index.
    }

    let load_result = if index_path.is_file() && !stale {
        ScipSemanticIndex::from_file_at_workspace(&index_path, ws_path.clone(), language)
    } else {
        match provision_index(&ws_path, indexer_lang) {
            Ok(generated) => {
                if let Some(fp) = fingerprint_workspace(&ws_path, indexer_lang) {
                    write_sidecar(&ws_path, indexer_lang, &fp);
                }
                ScipSemanticIndex::from_file_at_workspace(generated, ws_path.clone(), language)
            }
            Err(err) => {
                warn!(
                    "{language} SCIP indexer skipped (exit {}): {}",
                    err.exit_code, err.message
                );
                let reason = if stale {
                    SkipReason::IndexStale
                } else {
                    skip_from_err(&err)
                };
                return heuristic(reason);
            }
        }
    };

    match load_result {
        Ok(index) => {
            let root = index.project_root().to_path_buf();
            info!(
                "{language} index-backed resolver loaded SCIP at {}",
                root.display()
            );
            Box::new(IndexBackedSemanticResolver::new(index, root, language))
        }
        Err(err) => {
            warn!("Failed to load SCIP index for {language}: {err}");
            heuristic(SkipReason::IndexerFailed)
        }
    }
}

#[cfg(not(feature = "scip"))]
fn build_index_backed_or_heuristic(
    language: &str,
    workspace_root: Option<&url::Url>,
    config: crate::infrastructure::LanguageConfig,
) -> Box<dyn SemanticResolver> {
    Box::new(LspResolver::with_options(
        config,
        workspace_root.cloned(),
        build_session_options(language, workspace_root),
    ))
}
