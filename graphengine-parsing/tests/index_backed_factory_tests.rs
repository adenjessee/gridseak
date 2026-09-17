//! Factory routing tests: TypeScript must not use subprocess LSP (plan-02 criterion 6).

#![cfg(feature = "scip")]

use std::path::PathBuf;

use graphengine_parsing::application::resolution_disclosure::{ResolutionTierKind, SkipReason};
use graphengine_parsing::application::use_cases::parse_repo::factory::UseCaseFactory;
use graphengine_parsing::domain::Confidence;
use tempfile::TempDir;

#[tokio::test]
async fn broken_typescript_project_surfaces_indexer_failed() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("package.json"), "{").unwrap();
    let db = tempfile::NamedTempFile::new().unwrap();
    let url = url::Url::from_directory_path(dir.path()).unwrap();

    let use_case = UseCaseFactory::with_real_components(
        "typescript".into(),
        Confidence::Low,
        db.path().to_str().unwrap(),
        Some(url),
    )
    .await
    .expect("factory");

    let resolver = use_case.semantic_resolver();
    let disclosure = resolver.resolution_disclosure().await.expect("disclosure");
    assert_eq!(disclosure.tier_used, ResolutionTierKind::Heuristic);
    assert_eq!(
        disclosure.skip_reason,
        Some(SkipReason::IndexerNotProvisioned)
    );
}

#[tokio::test]
async fn typescript_with_committed_index_uses_layer2_not_lsp() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_scip");
    let db = tempfile::NamedTempFile::new().unwrap();
    let url = url::Url::from_directory_path(&fixture).unwrap();

    let use_case = UseCaseFactory::with_real_components(
        "typescript".into(),
        Confidence::Low,
        db.path().to_str().unwrap(),
        Some(url),
    )
    .await
    .expect("factory");

    let resolver = use_case.semantic_resolver();
    let disclosure = resolver.resolution_disclosure().await;
    // IndexBackedSemanticResolver only sets disclosure after resolve(); before
    // resolve the inner resolver returns layer2_active with 0 edges.
    assert!(
        disclosure.is_some(),
        "TS chain must expose resolution disclosure"
    );
    assert_ne!(
        disclosure.unwrap().tier_used,
        ResolutionTierKind::SubprocessLsp,
        "TypeScript factory chain must not route through subprocess LSP"
    );
}
