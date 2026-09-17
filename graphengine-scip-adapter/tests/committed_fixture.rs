//! Integration test against a committed `.scip` fixture (hermetic, no Node).

use std::path::Path;

use graphengine_scip_adapter::ScipIndex;

#[test]
fn committed_minimal_fixture_resolves_definition() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal/index.scip");
    let index = ScipIndex::from_file(&fixture).expect("parse committed fixture");
    let file = index.project_root().join("src/index.ts");
    let def = index
        .definition_at(&file, 4, 10)
        .expect("definition at callee position");
    assert!(def.symbol.contains("callee"));
}
