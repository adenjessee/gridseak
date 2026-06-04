//! End-to-end-ish regression: TS import -> Import edge persisted in SQLite DB.

use graphengine_parsing::application::ports::{GraphRepository, SemanticResolver, SyntaxExtractor};
use graphengine_parsing::application::use_cases::parse_repo::pipeline::graph_building::GraphBuilder;
use graphengine_parsing::domain::{
    Confidence, EdgeKind, Node, NodeKind, Provenance, ProvenanceSource, Range,
};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::infrastructure::lsp::definition_provider::DefinitionProvider;
use graphengine_parsing::infrastructure::lsp::LspResolver;
use graphengine_parsing::infrastructure::SqliteRepository;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

#[derive(Default)]
struct MockDefinitionProvider {
    defs: HashMap<String, Range>,
}

#[async_trait::async_trait]
impl DefinitionProvider for MockDefinitionProvider {
    async fn is_available(&self) -> bool {
        true
    }
    async fn ensure_ready(
        &self,
    ) -> Result<(), graphengine_parsing::infrastructure::lsp::errors::LspError> {
        Ok(())
    }
    async fn find_definition(
        &self,
        call_site: &graphengine_parsing::application::ports::CallSite,
    ) -> Result<Option<Range>, graphengine_parsing::infrastructure::lsp::errors::LspError> {
        Ok(self.defs.get(&call_site.function_name).cloned())
    }
    async fn hover(
        &self,
        _location: &Range,
    ) -> Result<Option<String>, graphengine_parsing::infrastructure::lsp::errors::LspError> {
        Ok(None)
    }
}

#[tokio::test]
async fn typescript_import_edge_is_persisted_in_db() {
    let temp = TempDir::new().expect("temp dir");
    let root = temp.path();
    let main_path = root.join("main.ts");
    let util_path = root.join("util.ts");
    let db_path = root.join("out.db");

    std::fs::write(
        &main_path,
        r#"
import { foo } from "./util";
foo();
"#,
    )
    .unwrap();
    std::fs::write(
        &util_path,
        r#"
export function foo() { return 1; }
"#,
    )
    .unwrap();

    // Extract syntax (real TS extractor).
    let ts_config = load_config("typescript").expect("load typescript config");
    let extractor = TreeSitterExtractor::new(ts_config).expect("typescript extractor");
    let mut syntax = extractor
        .extract(&[main_path.clone(), util_path.clone()])
        .await
        .expect("extract");

    // Ensure the callee symbol exists (so symbol_lookup can map def range -> Node).
    // The TypeScript config's function query should have extracted it, but keep this deterministic.
    let util_abs = util_path
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    if !syntax
        .symbols
        .iter()
        .any(|n| n.kind == NodeKind::Function && n.location.file == util_abs)
    {
        syntax.symbols.push(Node {
            id: "foo_id".into(),
            kind: NodeKind::Function,
            fqn: "util::foo".into(),
            location: Range::with_file(2, 0, 2, 40, util_abs.clone()),
            provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        });
    }

    // Resolve semantics (mocked definition provider).
    let foo_def_range = Range::with_file(2, 16, 2, 19, util_abs.clone());
    let provider = Arc::new(MockDefinitionProvider {
        defs: HashMap::from([("foo".to_string(), foo_def_range)]),
    });
    let resolver =
        LspResolver::with_provider(Arc::new(load_config("typescript").unwrap()), provider);
    let resolved = resolver.resolve(&syntax).await.expect("resolve");
    assert!(
        resolved
            .import_edges
            .iter()
            .any(|e| matches!(e.kind, EdgeKind::Import)),
        "expected at least one Import edge pre-persistence"
    );

    // Build final graph + persist.
    let graph = GraphBuilder::build_from_results(syntax, resolved, Confidence::Low).expect("graph");
    let repo = SqliteRepository::new(&db_path.to_string_lossy()).expect("repo");
    repo.upsert(&graph).await.expect("upsert");

    // Verify Import edges exist in DB.
    let conn = Connection::open(&db_path).expect("open db");
    // Post-P1.b, the `edges.kind` column stores serde-tagged JSON
    // (`{"kind":"Import"}`) rather than the pre-rework plain string
    // (`"Import"`). The test asserts on the exact wire form so a
    // future format drift fails here rather than silently returning
    // zero imports.
    let import_count: i64 = conn
        .query_row(
            r#"SELECT COUNT(*) FROM edges WHERE kind='{"kind":"Import"}'"#,
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert!(import_count > 0, "expected Import edges in DB");

    // Also require at least one stable Module->Module Import edge (dependency view).
    let module_import_count: i64 = conn
        .query_row(
            r#"SELECT COUNT(*)
             FROM edges e
             JOIN nodes n1 ON n1.id = e.from_id
             JOIN nodes n2 ON n2.id = e.to_id
             WHERE e.kind='{"kind":"Import"}' AND n1.kind='Module' AND n2.kind='Module'"#,
            [],
            |row| row.get(0),
        )
        .expect("count module->module");
    assert!(
        module_import_count > 0,
        "expected at least one Module->Module Import edge in DB"
    );
}
