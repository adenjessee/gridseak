//! T8 integration test — R39 (Apex property accessor) coverage gap
//! surfaces through the full extractor pipeline.
//!
//! Locks acceptance criterion #2 from
//! [`docs/workstreams/universal-fidelity/tasks/T8-coverage-awareness.md`]:
//! a `.cls` file containing one `get { … }` body produces a
//! `FileExtractionCoverage` whose `coverage_gaps` vector holds
//! exactly `CoverageGap::ApexPropertyAccessor { count: 1 }`.
//!
//! The test drives the real `TreeSitterExtractor::extract` path so
//! this is the end-to-end contract, not a unit-level stub. Unit-level
//! coverage of the counter lives at
//! `graphengine-parsing/src/syntax/language/apex/coverage.rs::tests`.

use graphengine_parsing::application::ports::{CoverageGap, SyntaxExtractor, SyntaxResults};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::PathBuf;

async fn extract(paths: &[PathBuf]) -> SyntaxResults {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    extractor.extract(paths).await.expect("parse apex fixtures")
}

/// Write a minimal `.cls` with one property accessor containing a
/// method call. Writing the fixture at test time rather than as a
/// checked-in file keeps the test hermetic and avoids polluting the
/// fixtures tree with single-purpose artefacts; the T8 NPSP canary
/// covers the "real corpus" axis separately.
fn write_r39_fixture(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("R39Sample.cls");
    let src = r#"public class R39Sample {
    public Integer x {
        get { return helper(); }
    }
    private Integer helper() { return 1; }
}
"#;
    std::fs::write(&path, src).expect("write fixture");
    path
}

#[tokio::test]
async fn r39_property_accessor_emits_gap() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_r39_fixture(tmp.path());

    let results = extract(std::slice::from_ref(&path)).await;

    assert_eq!(
        results.extraction_coverage.len(),
        1,
        "expected one coverage record, got {}",
        results.extraction_coverage.len(),
    );
    let cov = &results.extraction_coverage[0];
    assert_eq!(cov.language, "apex");
    assert_eq!(cov.file_path, path);
    assert!(
        cov.coverage_gaps
            .iter()
            .any(|g| matches!(g, CoverageGap::ApexPropertyAccessor { count: 1 })),
        "expected exactly one ApexPropertyAccessor gap with count=1, got {:?}",
        cov.coverage_gaps,
    );
    assert!(cov.has_invalidating_no_callers_gap());
}

#[tokio::test]
async fn r39_auto_property_emits_no_gap() {
    // Companion negative case: `public String s { get; set; }` has no
    // accessor body and therefore no unwalked region. A T8 regression
    // that starts counting auto-properties as R39 would blow the NPSP
    // false-positive budget wide open. Locking this behaviour here
    // prevents that drift.
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("AutoProp.cls");
    std::fs::write(
        &path,
        "public class AutoProp { public String s { get; set; } }",
    )
    .expect("write fixture");

    let results = extract(&[path]).await;

    assert_eq!(results.extraction_coverage.len(), 1);
    let cov = &results.extraction_coverage[0];
    assert!(
        cov.coverage_gaps.is_empty(),
        "auto-properties must not trigger R39 — got {:?}",
        cov.coverage_gaps,
    );
    assert!(!cov.has_invalidating_no_callers_gap());
}
