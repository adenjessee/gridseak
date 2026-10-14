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

/// Build the Rust scan's semantic resolver.
///
/// First choice is the compiler tier shared with Go / TypeScript / Python:
/// a `rust-analyzer scip` index (cached under `.gridseak/scip/scip-rust`
/// with the v2 fingerprint) swept occurrence by occurrence by
/// `IndexBackedSemanticResolver`. When no usable index exists, the scan
/// warns on stderr and falls back to the in-process `ra_ap_ide` resolver
/// (`rust-layer2`), whose disclosure keeps the index skip reason so the
/// report carries `compiler_tier_unavailable_v1`. If that also fails (no
/// `Cargo.toml`, sysroot discovery error, …) the subprocess-LSP path runs.
///
/// `GRIDSEAK_RUST_RESOLVER=layer2` skips the SCIP tier (A/B comparisons).
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

    let index_skip = rust_scip_tier(workspace_root);
    let index_skip = match index_skip {
        Ok(resolver) => return resolver,
        Err(skip) => skip,
    };

    match RustLayer2SemanticResolver::new(&ws_path) {
        Ok(resolver) => {
            info!(
                "Rust Layer-2 adapter initialised against workspace at {}",
                ws_path.display()
            );
            match index_skip {
                Some(reason) => Box::new(DisclosedSemanticResolver::keeping_inner_edges(
                    Box::new(resolver),
                    ResolutionDisclosure::layer2_fallback_to_ra_ide("rust", reason),
                )),
                None => Box::new(resolver),
            }
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

/// The SCIP tier for Rust. `Err(Some(reason))` when it was attempted and is
/// unusable (already warned on stderr); `Err(None)` when it was not
/// attempted (opted out, or built without the `scip` feature).
#[cfg(all(feature = "rust-layer2", feature = "scip"))]
fn rust_scip_tier(
    workspace_root: Option<&url::Url>,
) -> Result<Box<dyn SemanticResolver>, Option<crate::application::resolution_disclosure::SkipReason>>
{
    if std::env::var("GRIDSEAK_RUST_RESOLVER").as_deref() == Ok("layer2") {
        info!("GRIDSEAK_RUST_RESOLVER=layer2: skipping the Rust SCIP tier");
        return Err(None);
    }
    load_index_backed("rust", workspace_root).map_err(|(reason, detail)| {
        let detail = detail.unwrap_or_default();
        let first_line = detail.lines().next().unwrap_or(&detail);
        eprintln!(
            "warning: rust: compiler-backed (SCIP) call resolution unavailable; using the in-process rust-analyzer resolver, which resolves call sites one at a time and falls back to name matching ({first_line})"
        );
        Some(reason)
    })
}

#[cfg(all(feature = "rust-layer2", not(feature = "scip")))]
fn rust_scip_tier(
    _workspace_root: Option<&url::Url>,
) -> Result<Box<dyn SemanticResolver>, Option<crate::application::resolution_disclosure::SkipReason>>
{
    Err(None)
}

#[cfg(not(feature = "rust-layer2"))]
fn build_rust_semantic_resolver(
    config: crate::infrastructure::LanguageConfig,
    workspace_root: Option<&url::Url>,
) -> Box<dyn SemanticResolver> {
    #[cfg(feature = "scip")]
    if let Ok(resolver) = load_index_backed("rust", workspace_root) {
        return resolver;
    }
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
    use crate::application::resolution_disclosure::ResolutionDisclosure;
    use crate::infrastructure::DisclosedSemanticResolver;

    match load_index_backed(language, workspace_root) {
        Ok(resolver) => resolver,
        Err((reason, detail)) => {
            if let Some(detail) = detail {
                warn_compiler_tier_unavailable(language, &detail);
            }
            Box::new(DisclosedSemanticResolver::new(
                Box::new(HeuristicOnlySemanticResolver::new(language)),
                ResolutionDisclosure::layer2_fallback_to_heuristic(language, reason),
            ))
        }
    }
}

/// Provision (or reuse) the language's SCIP index and wrap it in the
/// reference-occurrence resolver. `Err` carries the skip reason and, when
/// the compiler tier was attempted, a detail line for the stderr warning.
#[cfg(feature = "scip")]
fn load_index_backed(
    language: &str,
    workspace_root: Option<&url::Url>,
) -> Result<
    Box<dyn SemanticResolver>,
    (
        crate::application::resolution_disclosure::SkipReason,
        Option<String>,
    ),
> {
    use crate::application::resolution_disclosure::SkipReason;
    use crate::infrastructure::{IndexBackedSemanticResolver, ScipSemanticIndex};
    use graphengine_scip_adapter::{provision_index, IndexerError, IndexerLanguage};
    use tracing::warn;

    let indexer_lang = match language {
        "typescript" | "javascript" => IndexerLanguage::TypeScript,
        "python" => IndexerLanguage::Python,
        "go" => IndexerLanguage::Go,
        "rust" => IndexerLanguage::Rust,
        other => {
            warn!("no SCIP registry mapping for {other}; heuristic only");
            return Err((SkipReason::LanguageNotRouted, None));
        }
    };

    let ws_path = workspace_root
        .and_then(|u| u.to_file_path().ok())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let skip_from_err = |err: &IndexerError| -> SkipReason {
        if err.stale() {
            SkipReason::IndexStale
        } else if err.not_provisioned() {
            SkipReason::IndexerNotProvisioned
        } else {
            SkipReason::IndexerFailed
        }
    };

    // `provision_index` returns the cached index only when its fingerprint
    // sidecar matches this workspace, and (re)writes the sidecar only when
    // an indexer actually ran. Never write the sidecar here: doing so for a
    // reused cache would bless an index copied from another checkout.
    let index_path = match provision_index(&ws_path, indexer_lang) {
        Ok(path) => path,
        Err(err) => {
            warn!(
                "{language} SCIP indexer skipped (exit {}): {}",
                err.exit_code, err.message
            );
            return Err((skip_from_err(&err), Some(err.message)));
        }
    };

    match ScipSemanticIndex::from_file_at_workspace(&index_path, ws_path.clone(), language) {
        Ok(index) => {
            let coverage = index.workspace_coverage(&ws_path);
            if coverage.looks_foreign() {
                let detail = format!(
                    "IndexStale(foreign_documents): only {}/{} sampled documents of {} exist under {}",
                    coverage.present,
                    coverage.checked,
                    index_path.display(),
                    ws_path.display()
                );
                warn!("{language} SCIP index rejected: {detail}");
                return Err((SkipReason::IndexStale, Some(detail)));
            }
            let root = index.project_root().to_path_buf();
            info!(
                "{language} index-backed resolver loaded SCIP at {} ({}/{} sampled documents present)",
                root.display(),
                coverage.present,
                coverage.checked
            );
            Ok(Box::new(IndexBackedSemanticResolver::new(
                index, root, language,
            )))
        }
        Err(err) => {
            warn!("Failed to load SCIP index for {language}: {err}");
            Err((
                SkipReason::IndexerFailed,
                Some(format!("failed to load {}: {err}", index_path.display())),
            ))
        }
    }
}

/// One stderr line whenever the compiler tier is skipped, so a scan cannot
/// silently fall back to tree-sitter edges.
#[cfg(feature = "scip")]
fn warn_compiler_tier_unavailable(language: &str, detail: &str) {
    let first_line = detail.lines().next().unwrap_or(detail);
    eprintln!(
        "warning: {language}: compiler-backed call resolution unavailable; using tree-sitter heuristic edges ({first_line})"
    );
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

#[cfg(all(test, feature = "scip"))]
mod scip_cache_tests {
    use super::*;
    use crate::application::resolution_disclosure::{ResolutionTierKind, SkipReason};
    use graphengine_scip_adapter::fingerprint::{
        fingerprint_workspace, read_sidecar, sidecar_path, write_sidecar,
    };
    use graphengine_scip_adapter::provisioning::{auto_provision_enabled, cache_dir, entry_for};
    use graphengine_scip_adapter::IndexerLanguage;
    use std::path::{Path, PathBuf};

    fn go_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/go_scip")
    }

    /// Workspace with the given sources and the fixture's index in the cache dir.
    fn workspace_with_cached_index(sources: &[(&str, &str)]) -> tempfile::TempDir {
        let ws = tempfile::TempDir::new().unwrap();
        for (name, body) in sources {
            std::fs::write(ws.path().join(name), body).unwrap();
        }
        let dir = cache_dir(ws.path(), entry_for(IndexerLanguage::Go).unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(go_fixture().join("index.scip"), dir.join("index.scip")).unwrap();
        ws
    }

    fn fixture_sources() -> Vec<(&'static str, String)> {
        ["go.mod", "hello.go"]
            .iter()
            .map(|n| (*n, std::fs::read_to_string(go_fixture().join(n)).unwrap()))
            .collect()
    }

    fn build(ws: &Path) -> Box<dyn SemanticResolver> {
        let url = url::Url::from_directory_path(ws).unwrap();
        let config = crate::infrastructure::load_config("go").unwrap();
        build_index_backed_or_heuristic("go", Some(&url), config)
    }

    #[tokio::test]
    async fn fresh_cache_is_reused_without_rewriting_sidecar() {
        let src = fixture_sources();
        let refs: Vec<(&str, &str)> = src.iter().map(|(n, b)| (*n, b.as_str())).collect();
        let ws = workspace_with_cached_index(&refs);
        let mut fp = fingerprint_workspace(ws.path(), IndexerLanguage::Go).unwrap();
        fp.generated_unix_secs = 12_345;
        write_sidecar(ws.path(), IndexerLanguage::Go, &fp);
        let sidecar = sidecar_path(ws.path(), IndexerLanguage::Go).unwrap();
        let before = std::fs::read_to_string(&sidecar).unwrap();

        let resolver = build(ws.path());
        let d = resolver.resolution_disclosure().await.unwrap();
        assert_eq!(d.tier_used, ResolutionTierKind::Layer2, "{d:?}");
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), before);
        assert_eq!(
            read_sidecar(ws.path(), IndexerLanguage::Go)
                .unwrap()
                .generated_unix_secs,
            12_345
        );
    }

    #[tokio::test]
    async fn copied_foreign_cache_falls_back_and_is_not_blessed() {
        if auto_provision_enabled() {
            return; // would try to run scip-go
        }
        // Fresh cache in workspace A, then A's .gridseak/ copied into B.
        let src = fixture_sources();
        let refs: Vec<(&str, &str)> = src.iter().map(|(n, b)| (*n, b.as_str())).collect();
        let a = workspace_with_cached_index(&refs);
        let fp = fingerprint_workspace(a.path(), IndexerLanguage::Go).unwrap();
        write_sidecar(a.path(), IndexerLanguage::Go, &fp);

        let b = tempfile::TempDir::new().unwrap();
        std::fs::write(b.path().join("other.go"), "package other\n").unwrap();
        let dst = cache_dir(b.path(), entry_for(IndexerLanguage::Go).unwrap());
        std::fs::create_dir_all(&dst).unwrap();
        let src_dir = cache_dir(a.path(), entry_for(IndexerLanguage::Go).unwrap());
        for f in ["index.scip", "fingerprint.txt"] {
            std::fs::copy(src_dir.join(f), dst.join(f)).unwrap();
        }
        let before = std::fs::read_to_string(dst.join("fingerprint.txt")).unwrap();

        let d = build(b.path()).resolution_disclosure().await.unwrap();
        assert_eq!(d.tier_used, ResolutionTierKind::Heuristic);
        assert_eq!(d.skip_reason, Some(SkipReason::IndexStale));
        assert_eq!(
            std::fs::read_to_string(dst.join("fingerprint.txt")).unwrap(),
            before,
            "a refused cache must never get a fresh sidecar"
        );
    }

    #[tokio::test]
    async fn index_whose_documents_are_absent_is_rejected() {
        // Sidecar matches this workspace, but the index describes files that
        // do not exist here.
        let ws = workspace_with_cached_index(&[("other.go", "package other\n")]);
        let fp = fingerprint_workspace(ws.path(), IndexerLanguage::Go).unwrap();
        write_sidecar(ws.path(), IndexerLanguage::Go, &fp);

        let d = build(ws.path()).resolution_disclosure().await.unwrap();
        assert_eq!(d.tier_used, ResolutionTierKind::Heuristic);
        assert_eq!(d.skip_reason, Some(SkipReason::IndexStale));
    }

    #[tokio::test]
    async fn missing_cache_discloses_not_provisioned() {
        if auto_provision_enabled() {
            return;
        }
        let ws = tempfile::TempDir::new().unwrap();
        std::fs::write(ws.path().join("main.go"), "package main\n").unwrap();
        let d = build(ws.path()).resolution_disclosure().await.unwrap();
        assert_eq!(d.tier_used, ResolutionTierKind::Heuristic);
        assert_eq!(d.skip_reason, Some(SkipReason::IndexerNotProvisioned));
    }
}
