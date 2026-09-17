//! Rust engine unification cross-check (plan-02 T9).
//!
//! Compares `rust_layer2` (in-process ra-ide) vs SCIP ingested from
//! `rust-analyzer scip` on the same two-file fixture.

#![cfg(all(feature = "rust-layer2", feature = "scip"))]

use std::path::PathBuf;

use graphengine_parsing::application::ports::{
    CallSite, SemanticResolver, SyntaxResults, UnresolvedReference,
};
use graphengine_parsing::domain::{Node, Range};
use graphengine_parsing::infrastructure::{
    IndexBackedSemanticResolver, RustLayer2SemanticResolver, ScipSemanticIndex,
};

fn two_file_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("graphengine-ra-ide-adapter/tests/fixtures/two_file_call")
}

#[tokio::test]
async fn rust_unification_crosscheck_records_engine_parity_on_fixture() {
    let root = two_file_fixture_root();
    let index_path = root.join("index.scip");
    assert!(
        index_path.is_file(),
        "run `rust-analyzer scip .` in {} before this test",
        root.display()
    );

    let main_file = root.join("src/main.rs").to_string_lossy().to_string();
    let lib_file = root.join("src/lib.rs").to_string_lossy().to_string();
    let caller = Node::function(
        "fixture_main::main".into(),
        Range::with_file(3, 0, 5, 1, main_file.clone()),
    );
    let callee = Node::function(
        "fixture_lib::callee".into(),
        Range::with_file(6, 0, 8, 1, lib_file.clone()),
    );
    let call_site = CallSite {
        location: Range::with_file(4, 4, 4, 10, main_file.clone()),
        function_name: "callee".into(),
        receiver_range: None,
        receiver_text: None,
        arg_types: Vec::new(),
    };

    let mut hints = SyntaxResults::new();
    hints.language = Some("rust".into());
    hints.workspace_root = Some(root.to_string_lossy().to_string());
    hints.symbols = vec![caller.clone(), callee.clone()];
    hints.references.push(UnresolvedReference::Call(call_site));

    let layer2 = RustLayer2SemanticResolver::new(&root).expect("layer2 loads fixture");
    let layer2_edges = layer2.resolve(&hints).await.expect("layer2 resolve");
    let layer2_emitted = layer2.snapshot().edges_emitted;

    let scip_index = ScipSemanticIndex::from_file_at_workspace(&index_path, root.clone(), "rust")
        .expect("scip index loads");
    let scip_resolver = IndexBackedSemanticResolver::new(scip_index, root.clone(), "rust");
    let scip_edges = scip_resolver.resolve(&hints).await.expect("scip resolve");

    eprintln!(
        "T9 cross-check two_file_call: layer2_emitted={layer2_emitted} scip_call_edges={} layer2_compiler_edges={} scip_compiler_edges={}",
        scip_edges.call_edges.len(),
        layer2_edges.stats.compiler_edges,
        scip_edges.stats.compiler_edges,
    );

    assert_eq!(layer2_edges.call_edges.len(), 1, "layer2 baseline");
    assert_eq!(
        scip_edges.call_edges.len(),
        layer2_edges.call_edges.len(),
        "SCIP path should match layer2 on the canonical two-file fixture"
    );
    assert_eq!(layer2_edges.call_edges[0].to_id, callee.id);
    assert_eq!(scip_edges.call_edges[0].to_id, callee.id);
}
