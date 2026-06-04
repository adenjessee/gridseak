//! T8 integration test — R41 (Apex map-literal field initializer)
//! coverage gap surfaces through the full extractor pipeline.
//!
//! Locks acceptance criterion #3 from
//! [`docs/workstreams/universal-fidelity/tasks/T8-coverage-awareness.md`]:
//! a `.cls` file containing one `new Map<_, _>{ … }` initializer
//! produces a `FileExtractionCoverage` whose `coverage_gaps` vector
//! holds exactly `CoverageGap::ApexMapLiteralInitializer { count: 1 }`.

use graphengine_parsing::application::ports::{CoverageGap, SyntaxExtractor, SyntaxResults};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::PathBuf;

async fn extract(paths: &[PathBuf]) -> SyntaxResults {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    extractor.extract(paths).await.expect("parse apex fixtures")
}

#[tokio::test]
async fn r41_map_literal_emits_gap() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("R41Sample.cls");
    let src = r#"public class R41Sample {
    public Map<String, Object> m = new Map<String, Object>{ 'key' => helper() };
    private Object helper() { return null; }
}
"#;
    std::fs::write(&path, src).expect("write fixture");

    let results = extract(std::slice::from_ref(&path)).await;

    assert_eq!(results.extraction_coverage.len(), 1);
    let cov = &results.extraction_coverage[0];
    assert_eq!(cov.language, "apex");
    assert_eq!(cov.file_path, path);
    assert!(
        cov.coverage_gaps
            .iter()
            .any(|g| matches!(g, CoverageGap::ApexMapLiteralInitializer { count: 1 })),
        "expected exactly one ApexMapLiteralInitializer gap with count=1, got {:?}",
        cov.coverage_gaps,
    );
    assert!(cov.has_invalidating_no_callers_gap());
}

#[tokio::test]
async fn r41_no_map_literal_emits_no_gap() {
    // Regular field initializer (plain value, no map literal) must not
    // fire R41.
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("PlainField.cls");
    std::fs::write(&path, "public class PlainField { public Integer n = 42; }")
        .expect("write fixture");

    let results = extract(&[path]).await;
    assert_eq!(results.extraction_coverage.len(), 1);
    assert!(
        results.extraction_coverage[0].coverage_gaps.is_empty(),
        "plain field initializer must not fire R41",
    );
}
