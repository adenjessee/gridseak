//! Python index-backed resolver integration test (plan-02 T8).

#![cfg(feature = "scip")]

use std::path::PathBuf;

use graphengine_parsing::application::ports::{
    CallSite, SemanticResolver, SyntaxResults, UnresolvedReference,
};
use graphengine_parsing::domain::{Node, ProvenanceSource, Range};
use graphengine_parsing::infrastructure::{IndexBackedSemanticResolver, ScipSemanticIndex};

fn py_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/py_scip")
}

#[tokio::test]
async fn python_index_backed_emits_compiler_edge() {
    let root = py_fixture_root();
    let index =
        ScipSemanticIndex::from_file_at_workspace(root.join("index.scip"), root.clone(), "python")
            .expect("committed Python SCIP fixture must load");
    let resolver = IndexBackedSemanticResolver::new(index, root.clone(), "python");

    let file = root.join("app.py").to_string_lossy().to_string();
    let callee = Node::function(
        "app.callee".into(),
        Range::with_file(1, 0, 2, 1, file.clone()),
    );
    let caller = Node::function(
        "app.caller".into(),
        Range::with_file(5, 0, 6, 1, file.clone()),
    );
    let call_site = CallSite {
        location: Range::with_file(6, 11, 6, 17, file.clone()),
        function_name: "callee".into(),
        receiver_range: None,
        receiver_text: None,
        arg_types: Vec::new(),
    };

    let mut hints = SyntaxResults::new();
    hints.language = Some("python".into());
    hints.symbols = vec![caller.clone(), callee.clone()];
    hints.references.push(UnresolvedReference::Call(call_site));

    let edges = resolver.resolve(&hints).await.expect("resolve");
    assert_eq!(edges.call_edges.len(), 1);
    assert_eq!(
        edges.call_edges[0].provenance.source,
        ProvenanceSource::Compiler
    );
    assert_eq!(edges.stats.compiler_edges, 1);
}

#[tokio::test]
async fn broken_python_project_surfaces_indexer_failed_via_factory() {
    use graphengine_parsing::application::resolution_disclosure::{ResolutionTierKind, SkipReason};
    use graphengine_parsing::application::use_cases::parse_repo::factory::UseCaseFactory;
    use graphengine_parsing::domain::Confidence;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let db = tempfile::NamedTempFile::new().unwrap();
    let url = url::Url::from_directory_path(dir.path()).unwrap();
    let use_case = UseCaseFactory::with_real_components(
        "python".into(),
        Confidence::Low,
        db.path().to_str().unwrap(),
        Some(url),
    )
    .await
    .expect("factory");
    let disclosure = use_case
        .semantic_resolver()
        .resolution_disclosure()
        .await
        .expect("disclosure");
    assert_eq!(disclosure.tier_used, ResolutionTierKind::Heuristic);
    assert_eq!(
        disclosure.skip_reason,
        Some(SkipReason::IndexerNotProvisioned)
    );
}
