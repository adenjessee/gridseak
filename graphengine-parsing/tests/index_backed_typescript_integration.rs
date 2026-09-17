//! TypeScript index-backed resolver integration tests (plan-02 T7b).

#![cfg(feature = "scip")]

use std::path::PathBuf;

use graphengine_parsing::application::ports::{
    CallSite, SemanticResolver, SyntaxResults, UnresolvedReference,
};
use graphengine_parsing::application::resolution_disclosure::ResolutionTierKind;
use graphengine_parsing::domain::{Node, ProvenanceSource, Range};
use graphengine_parsing::infrastructure::{IndexBackedSemanticResolver, ScipSemanticIndex};

fn ts_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_scip")
}

#[tokio::test]
async fn typescript_index_backed_emits_compiler_edge() {
    let root = ts_fixture_root();
    let index = ScipSemanticIndex::from_file_at_workspace(
        root.join("index.scip"),
        root.clone(),
        "typescript",
    )
    .expect("committed SCIP fixture must load");
    let resolver = IndexBackedSemanticResolver::new(index, root.clone(), "typescript");

    let file = root.join("src/index.ts").to_string_lossy().to_string();
    let callee = Node::function(
        "index.callee".into(),
        Range::with_file(1, 0, 3, 1, file.clone()),
    );
    let caller = Node::function(
        "index.caller".into(),
        Range::with_file(5, 0, 7, 1, file.clone()),
    );
    let call_site = CallSite {
        location: Range::with_file(6, 9, 6, 15, file.clone()),
        function_name: "callee".into(),
        receiver_range: None,
        receiver_text: None,
        arg_types: Vec::new(),
    };

    let mut hints = SyntaxResults::new();
    hints.language = Some("typescript".into());
    hints.workspace_root = Some(root.to_string_lossy().to_string());
    hints.symbols = vec![caller.clone(), callee.clone()];
    hints.references.push(UnresolvedReference::Call(call_site));

    let edges = resolver.resolve(&hints).await.expect("resolve");
    assert_eq!(edges.call_edges.len(), 1);
    assert_eq!(edges.call_edges[0].from_id, caller.id);
    assert_eq!(edges.call_edges[0].to_id, callee.id);
    assert_eq!(
        edges.call_edges[0].provenance.source,
        ProvenanceSource::Compiler
    );
    assert_eq!(edges.stats.compiler_edges, 1);
    assert_eq!(edges.stats.lsp_edges, 0);

    let disclosure = resolver.resolution_disclosure().await.expect("disclosure");
    assert_eq!(disclosure.tier_used, ResolutionTierKind::Layer2);
}

#[test]
fn method_call_caret_hits_method_not_receiver() {
    use graphengine_parsing::infrastructure::semantic::caret::caret_for_callee;
    let file = ts_fixture_root()
        .join("src/index.ts")
        .to_string_lossy()
        .to_string();
    let call_site = CallSite {
        location: Range::with_file(16, 9, 16, 21, file.clone()),
        function_name: "method_call:parse".into(),
        receiver_range: Some(Range::with_file(16, 9, 16, 12, file)),
        receiver_text: Some("obj".into()),
        arg_types: Vec::new(),
    };
    let (line, col) = caret_for_callee(&call_site);
    assert_eq!(line, 16);
    assert_eq!(col, 13);
}
