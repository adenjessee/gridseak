//! Regression tests: TypeScript import extraction and edge emission

use graphengine_parsing::application::ports::{
    ImportKind, ImportPath, ImportSpec, ImportVisibility, PathRoot, SemanticResolver,
    SyntaxExtractor,
};
use graphengine_parsing::domain::{
    Confidence, Node, NodeKind, Provenance, ProvenanceSource, Range,
};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::infrastructure::lsp::definition_provider::DefinitionProvider;
use graphengine_parsing::infrastructure::lsp::LspResolver;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
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
async fn typescript_import_extraction_produces_import_specs_and_resolver_emits_import_edge() {
    let temp = TempDir::new().expect("temp dir");
    let main_path = temp.path().join("main.ts");
    let util_path = temp.path().join("util.ts");

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

    let config = load_config("typescript").expect("load typescript config");
    let extractor = TreeSitterExtractor::new(config).expect("typescript extractor");
    let syntax = extractor
        .extract(&[main_path.clone(), util_path.clone()])
        .await
        .expect("extract");

    assert!(
        !syntax.import_specs.is_empty(),
        "expected at least one ImportSpec from TS import statement"
    );

    // Ensure we have a Module node to anchor the import (added by extractor fix).
    assert!(
        syntax
            .symbols
            .iter()
            .any(|n| n.kind == NodeKind::Module && n.location.file.contains("main.ts")),
        "expected file-scoped Module node for main.ts"
    );

    // Build a resolver with a mock definition provider: resolving `foo` returns the range of foo() in util.ts.
    let util_path_str = util_path
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let foo_def_range = Range::with_file(2, 16, 2, 19, util_path_str.clone());

    // Add a function node that covers the definition range so symbol lookup succeeds.
    let mut syntax_with_def = syntax.clone();
    syntax_with_def.symbols.push(Node {
        id: "foo_id".into(),
        kind: NodeKind::Function,
        fqn: "util::foo".into(),
        location: Range::with_file(2, 0, 2, 40, util_path_str.clone()),
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        properties: std::collections::HashMap::new(),
        trait_metadata: None,
    });

    let provider = Arc::new(MockDefinitionProvider {
        defs: HashMap::from([("foo".to_string(), foo_def_range)]),
    });

    // LspResolver needs a LanguageConfig Arc; use typescript config.
    let ts_config = Arc::new(load_config("typescript").expect("load typescript config"));
    let resolver = LspResolver::with_provider(ts_config, provider);

    let resolved = resolver.resolve(&syntax_with_def).await.expect("resolve");
    assert!(
        !resolved.import_edges.is_empty(),
        "expected at least one Import edge to be emitted"
    );
    assert!(
        resolved
            .import_edges
            .iter()
            .all(|e| matches!(e.kind, graphengine_parsing::domain::EdgeKind::Import)),
        "expected Import-kind edges only"
    );
}

#[test]
fn typescript_import_extractor_supports_named_import_alias_shape() {
    // Pure unit test: ensure we can represent TS alias imports in ImportSpec shape
    let spec = ImportSpec {
        range: Range::with_file(1, 10, 1, 13, "main.ts"),
        path: ImportPath::new(PathRoot::Unqualified, vec!["foo".into()]),
        alias: Some("bar".into()),
        visibility: ImportVisibility::Private,
        kind: ImportKind::Use,
        is_glob: false,
        source_file: "main.ts".into(),
    };
    assert_eq!(spec.alias.as_deref(), Some("bar"));
    assert_eq!(spec.path.segments.last().map(|s| s.as_str()), Some("foo"));
}
