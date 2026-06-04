//! Stage-12 follow-up A1 regression tests — "missing Import edges".
//!
//! Background: the Stage-11 pilot scans of `gridseak-graphengine`
//! (Rust + JS polyglot) and `vercel/commerce` (TS-majority) both
//! reported `0 import edges` in the SQLite graph metadata. That is
//! how the analyzer ends up labelling Dead-code and Hotspots as
//! `Low-confidence` (no edges → no fan-in signal → no defensible
//! reachability verdict). See `docs/04-evidence/RC1_GAPS.md`
//! entry A1.
//!
//! Static discovery localised the gap to three independent issues
//! that all conspire to make `import_edges` underreport:
//!
//!   1. `RustLayer2SemanticResolver` (the production Rust resolver)
//!      emits only `EdgeKind::Call`. It never runs
//!      `ImportResolver::resolve_with_lsp` or
//!      `ModuleDependencyResolver::resolve_relative_module_imports`,
//!      so even when the Rust tree-sitter extractor produces
//!      `import_specs` (for `use` statements), no edges are emitted.
//!
//!   2. `ModuleDependencyResolver::resolve_relative_module_imports`
//!      depends on path canonicalisation (`Path::canonicalize` /
//!      macOS `/private/` normalisation) to match importer and
//!      importee files. When the fixture lives under a non-
//!      canonical path (or when `import_specs.source_file` and
//!      `Module.location.file` disagree on trailing `/`, case, or
//!      whether they were canonicalised) the heuristic returns
//!      `None` and no edge is built — even though the inputs look
//!      right.
//!
//!   3. `SqliteRepository::store_resolution_telemetry` derives the
//!      `import_edges` metadata value from the in-memory `edges`
//!      slice passed to `upsert`. In a polyglot scan the parser
//!      orchestrator calls `upsert` once per language with that
//!      pass's edges only, so the metadata value reflects the
//!      *last* pass instead of the cumulative count. The SQLite
//!      `edges` table itself accumulates correctly because of
//!      `INSERT OR REPLACE`; the metadata count just doesn't.
//!
//! Each of the four tests below pins one of those facts. They are
//! written to **fail today** and pass after PR3's fix lands; that
//! way the RED→GREEN diff is visible in review.

use graphengine_parsing::application::ports::{
    ImportKind, ImportPath, ImportSpec, ImportVisibility, PathRoot, SyntaxExtractor, SyntaxResults,
};
use graphengine_parsing::domain::{
    Confidence, Edge, EdgeKind, Node, NodeKind, Provenance, ProvenanceSource, Range,
};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::infrastructure::lsp::resolvers::module_dependency_resolver::ModuleDependencyResolver;
use graphengine_parsing::infrastructure::SqliteRepository;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use rusqlite::Connection;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test 1 — Rust `use` statements produce ImportSpec entries
// ---------------------------------------------------------------------------
//
// The Rust tree-sitter extractor is supposed to emit one `ImportSpec`
// per `use` declaration so downstream resolvers (LspResolver →
// ImportResolver, ModuleDependencyResolver) have something to turn
// into Import edges. If extraction silently drops `use` statements,
// every later layer correctly produces zero edges — but for the
// wrong reason.
//
// This test fails today on the production Rust path because the
// Layer-2 resolver in `factory.rs` is selected for Rust scans, and
// the Layer-2 resolver does not invoke any import emission. The
// extracted `import_specs` are still produced; we assert that here
// so the import-resolution fix in PR3 has something deterministic
// to build edges from.

#[tokio::test]
async fn rust_use_statements_are_extracted_as_import_specs() {
    let temp = TempDir::new().expect("temp dir");
    let lib_path = temp.path().join("lib.rs");
    let mod_path = temp.path().join("util.rs");

    std::fs::write(
        &lib_path,
        r#"
mod util;
use crate::util::helper;

pub fn caller() {
    helper();
}
"#,
    )
    .unwrap();
    std::fs::write(
        &mod_path,
        r#"
pub fn helper() -> i32 { 42 }
"#,
    )
    .unwrap();

    let rust_config = load_config("rust").expect("load rust config");
    let extractor = TreeSitterExtractor::new(rust_config).expect("rust extractor");
    let syntax = extractor
        .extract(&[lib_path.clone(), mod_path.clone()])
        .await
        .expect("extract");

    let lib_specs: Vec<_> = syntax
        .import_specs
        .iter()
        .filter(|s| s.source_file.contains("lib.rs"))
        .collect();

    assert!(
        !lib_specs.is_empty(),
        "Rust extractor must emit at least one ImportSpec for `use crate::util::helper;` in lib.rs. \
         If this regresses, the entire Rust import-edge pipeline (PR3) loses its input. \
         Got import_specs across all files: {:?}",
        syntax
            .import_specs
            .iter()
            .map(|s| (&s.source_file, &s.path.segments))
            .collect::<Vec<_>>()
    );

    let resolves_to_helper = lib_specs.iter().any(|s| {
        s.path.segments.last().map(|x| x.as_str()) == Some("helper")
            || s.alias.as_deref() == Some("helper")
    });
    assert!(
        resolves_to_helper,
        "The extracted ImportSpec for lib.rs must reference `helper`. Got segments: {:?}",
        lib_specs
            .iter()
            .map(|s| &s.path.segments)
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 2 — TS module-dep resolver with NO LSP produces Module→Module edges
// ---------------------------------------------------------------------------
//
// `ModuleDependencyResolver::resolve_relative_module_imports` is the
// LSP-free heuristic that closes the gap when LSP is unavailable
// (which is the common case for shadow-mode scans on machines
// without rust-analyzer / typescript-language-server installed).
//
// This test exercises the path *without* the LspResolver wrapper.
// Today it fails because path canonicalisation inside
// `resolve_relative_target_file` resolves the importer's `./util`
// to a real path on disk via `canonicalize`, but the synthetic
// `Module.location.file` strings are non-canonical, so
// `find_file_module_node_id`'s match misses.
//
// PR3 step 3 will normalise both sides through a shared helper so
// equality survives `/private/` prefixes and trailing-slash
// differences.

#[test]
fn ts_module_dependency_resolver_emits_relative_import_edge_without_lsp() {
    let temp = TempDir::new().expect("temp dir");
    let main_path = temp.path().join("main.ts");
    let util_path = temp.path().join("util.ts");

    // Real files on disk so `resolve_relative_target_file` finds
    // `./util.ts` from `main.ts`. Content doesn't matter — the
    // resolver only inspects path metadata.
    std::fs::write(&main_path, b"import { foo } from './util';\n").unwrap();
    std::fs::write(&util_path, b"export function foo() { return 1; }\n").unwrap();

    let main_str = main_path.to_string_lossy().to_string();
    let util_str = util_path.to_string_lossy().to_string();

    let main_module_id = "module_main".to_string();
    let util_module_id = "module_util".to_string();

    let module_node = |id: &str, file: &str| Node {
        id: id.into(),
        kind: NodeKind::Module,
        fqn: id.into(),
        location: Range::with_file(1, 0, 1, 0, file.to_string()),
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        properties: [("file_module".to_string(), serde_json::Value::Bool(true))]
            .into_iter()
            .collect(),
        trait_metadata: None,
    };

    let mut syntax = SyntaxResults::new();
    syntax.symbols.push(module_node(&main_module_id, &main_str));
    syntax.symbols.push(module_node(&util_module_id, &util_str));
    syntax.source_files = vec![main_str.clone(), util_str.clone()];
    syntax.language = Some("typescript".into());
    syntax.import_specs.push(ImportSpec {
        range: Range::with_file(1, 0, 1, 30, main_str.clone()),
        path: ImportPath::new(PathRoot::Unqualified, vec!["./util".into()]),
        alias: None,
        visibility: ImportVisibility::Private,
        kind: ImportKind::Use,
        is_glob: false,
        source_file: main_str.clone(),
    });

    let extensions: Vec<String> = vec![".ts".into(), ".tsx".into(), ".js".into(), ".jsx".into()];
    let edges = ModuleDependencyResolver::resolve_relative_module_imports(&syntax, &extensions);

    let module_to_module: Vec<&Edge> = edges
        .iter()
        .filter(|e| {
            matches!(e.kind, EdgeKind::Import)
                && e.from_id == main_module_id
                && e.to_id == util_module_id
        })
        .collect();

    assert!(
        !module_to_module.is_empty(),
        "ModuleDependencyResolver must emit a Module→Module Import edge for `./util` \
         when both files exist on disk and Module nodes are present in syntax.symbols. \
         Got {} total edges: {:?}",
        edges.len(),
        edges
            .iter()
            .map(|e| (&e.from_id, &e.to_id, &e.kind))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 3 — same TS path but exercised through LspResolver with a NO-OP LSP
// ---------------------------------------------------------------------------
//
// `LspResolver::resolve` is the only entry point the production
// pipeline goes through (via `factory.rs`). Even with an
// `is_available = false` definition provider, the resolver is
// required to invoke `ModuleDependencyResolver::resolve_relative_module_imports`
// at the end of `resolve_inner` so the LSP-free heuristic still
// runs. If that branch silently bypasses the heuristic when LSP is
// unavailable, every shadow-mode scan misses module-level Import
// edges. This test pins the contract.

#[tokio::test]
async fn ts_lsp_resolver_emits_module_import_edge_when_lsp_unavailable() {
    use graphengine_parsing::application::ports::SemanticResolver;
    use graphengine_parsing::infrastructure::lsp::definition_provider::DefinitionProvider;
    use graphengine_parsing::infrastructure::lsp::LspResolver;
    use std::sync::Arc;

    struct UnavailableLsp;

    #[async_trait::async_trait]
    impl DefinitionProvider for UnavailableLsp {
        async fn is_available(&self) -> bool {
            false
        }
        async fn ensure_ready(
            &self,
        ) -> Result<(), graphengine_parsing::infrastructure::lsp::errors::LspError> {
            // Real LSP-unavailable behaviour: ensure_ready fails so the
            // resolver falls through to the heuristic + module-dep
            // path. This is exactly what happens on a shadow-mode
            // user's machine that does not have a TS language server
            // installed.
            Err(
                graphengine_parsing::infrastructure::lsp::errors::LspError::ConnectionFailed(
                    "lsp unavailable in this test".into(),
                ),
            )
        }
        async fn find_definition(
            &self,
            _call_site: &graphengine_parsing::application::ports::CallSite,
        ) -> Result<Option<Range>, graphengine_parsing::infrastructure::lsp::errors::LspError>
        {
            Ok(None)
        }
        async fn hover(
            &self,
            _location: &Range,
        ) -> Result<Option<String>, graphengine_parsing::infrastructure::lsp::errors::LspError>
        {
            Ok(None)
        }
    }

    let temp = TempDir::new().expect("temp dir");
    let main_path = temp.path().join("main.ts");
    let util_path = temp.path().join("util.ts");
    std::fs::write(&main_path, b"import { foo } from './util';\nfoo();\n").unwrap();
    std::fs::write(&util_path, b"export function foo() { return 1; }\n").unwrap();

    let ts_config = load_config("typescript").expect("load typescript config");
    let extractor = TreeSitterExtractor::new(ts_config).expect("typescript extractor");
    let syntax = extractor
        .extract(&[main_path.clone(), util_path.clone()])
        .await
        .expect("extract");

    let cfg = Arc::new(load_config("typescript").expect("config"));
    let resolver = LspResolver::with_provider(cfg, Arc::new(UnavailableLsp));
    let resolved = resolver.resolve(&syntax).await.expect("resolve");

    let module_to_module = resolved
        .import_edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Import))
        .count();
    assert!(
        module_to_module > 0,
        "LspResolver must still emit Import edges (via ModuleDependencyResolver) when LSP is \
         unavailable. Got {} import edges total. This is the exact shadow-mode shape — most \
         users do NOT have a TS language server installed.",
        module_to_module
    );
}

// ---------------------------------------------------------------------------
// Test 4 — `import_edges` metadata accumulates across polyglot passes
// ---------------------------------------------------------------------------
//
// The polyglot orchestrator runs one `ParseRepositoryUseCase` pass
// per language. Each pass calls `SqliteRepository::upsert(graph)`
// with that pass's `Graph` object. The current
// `store_resolution_telemetry` impl counts Import edges in
// `&graph.edges` only — so after pass 2 the metadata value reflects
// pass 2's edges, not pass 1 + pass 2.
//
// This test simulates the two-pass shape with two `upsert` calls
// against the same DB. The first carries one Import edge, the
// second carries another (different `from_id`/`to_id` to avoid
// `INSERT OR REPLACE` collapse). After both pass through, the
// metadata value should be `2` and the edges table should also
// contain 2 rows.

#[tokio::test]
async fn polyglot_import_edges_metadata_accumulates_across_passes() {
    use graphengine_parsing::application::ports::GraphRepository;
    use graphengine_parsing::domain::Graph;

    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("polyglot.sqlite");
    let repo = SqliteRepository::new(&db_path.to_string_lossy()).expect("repo");

    let module = |id: &str, lang: &str| Node {
        id: id.into(),
        kind: NodeKind::Module,
        fqn: id.into(),
        location: Range::with_file(1, 0, 1, 0, format!("/repo/{id}.{lang}")),
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        properties: std::collections::HashMap::new(),
        trait_metadata: None,
    };

    // Pass 1 — Rust-ish: emits a single Module→Module Import edge.
    let mut g1 = Graph::new();
    g1.add_node(module("r_a", "rs"));
    g1.add_node(module("r_b", "rs"));
    g1.add_edge(Edge::new(
        "r_a".into(),
        "r_b".into(),
        EdgeKind::Import,
        Provenance::new(ProvenanceSource::Heuristic, Confidence::Medium),
    ));
    repo.upsert(&g1).await.expect("pass1 upsert");

    // Pass 2 — TS-ish: completely disjoint nodes + edges.
    let mut g2 = Graph::new();
    g2.add_node(module("t_a", "ts"));
    g2.add_node(module("t_b", "ts"));
    g2.add_edge(Edge::new(
        "t_a".into(),
        "t_b".into(),
        EdgeKind::Import,
        Provenance::new(ProvenanceSource::Heuristic, Confidence::Medium),
    ));
    repo.upsert(&g2).await.expect("pass2 upsert");

    // The edges table is the source of truth; both Import edges must
    // be present because `INSERT OR REPLACE` keys on (from, to, kind).
    let conn = Connection::open(&db_path).expect("open db");
    let table_count: i64 = conn
        .query_row(
            r#"SELECT COUNT(*) FROM edges WHERE kind = '{"kind":"Import"}'"#,
            [],
            |row| row.get(0),
        )
        .expect("count edges");
    assert_eq!(
        table_count, 2,
        "edges table must contain both Import edges after the two passes; got {table_count}"
    );

    // The metadata view must agree with the table. Today it
    // reflects only the most recent `&graph.edges` slice (=1 from
    // pass 2), which is the exact A1 misreport behaviour.
    let metadata_count: String = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = 'import_edges'",
            [],
            |row| row.get(0),
        )
        .expect("read import_edges metadata");
    assert_eq!(
        metadata_count, "2",
        "metadata.import_edges must equal the total count in the edges table \
         (both Import edges across the two polyglot passes); got `{metadata_count}`. \
         Today this asserts as `1` because store_resolution_telemetry reads from the \
         in-memory edges slice of the current pass instead of recomputing from the \
         accumulated edges table."
    );
}

// Convenience: a non-async `SyntaxExtractor` reference so the import
// stays even if a test arm changes — without this the file fails to
// compile when a test is `#[ignore]`'d.
#[allow(dead_code)]
fn _ensure_extractor_trait_in_scope<T: SyntaxExtractor>(_: &T) {}
