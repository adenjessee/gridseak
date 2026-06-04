//! R46 acceptance driver — cross-language keyword-name extraction.
//!
//! Before R46 was fixed, `name_validator::is_reserved_keyword` applied a
//! monolithic union of keywords from every supported language. Apex
//! methods named `match` (Rust keyword), `type` (TypeScript keyword),
//! `lambda` (Python keyword), `object` / `in` (C# keywords), etc., were
//! silently dropped at extraction — their Function nodes never appeared
//! in `SyntaxResults.symbols`, and every inbound call edge to them went
//! unresolved, producing silent `no_callers` false positives downstream.
//!
//! The R46 fix makes the validator language-scoped: each language has
//! its own audited reserved-word list, and the two call sites
//! (`symbol_extractor.rs`, `trait_context_detector.rs`) pass the current
//! extractor's `LanguageSpecificExtractor::language()`.
//!
//! # What this driver guarantees end-to-end
//!
//! For each cross-language-keyword Apex method in the fixture:
//!
//! 1. The method is emitted as a `Function` node in
//!    `SyntaxResults.symbols` with the expected FQN shape
//!    (`CrossLangKeywordNames::CrossLangKeywordNames::<name>()`).
//! 2. The inbound call from `callAll()` resolves to exactly one edge
//!    targeting that Function node (via the same bare-self-dispatch
//!    path TR-A.4 exercises for regular Apex identifiers).
//!
//! If either assertion fires, R46 has regressed and Apex methods named
//! after keywords from other languages are being dropped again.

use graphengine_parsing::application::ports::{SemanticResolver, SyntaxExtractor, SyntaxResults};
use graphengine_parsing::domain::apex::class_symbols::ApexClassSymbols;
use graphengine_parsing::domain::{Edge, NodeKind};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::language::apex::class_registry::{
    ApexClassRegistry, ApexTypeKind,
};
use graphengine_parsing::syntax::language::apex::ApexHeuristicResolver;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::{Path, PathBuf};

fn fixture_paths() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("apex_resolver")
        .join("r46_cross_language_keyword_names");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("cls"))
        .collect();
    out.sort();
    out
}

async fn extract(paths: &[PathBuf]) -> SyntaxResults {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    extractor.extract(paths).await.expect("parse r46 fixture")
}

fn build_registry(results: &SyntaxResults) -> ApexClassRegistry {
    let mut registry = ApexClassRegistry::with_standard_preload();
    let path_for = |api_name: &str| -> PathBuf {
        results
            .symbols
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Struct))
            .find(|n| n.fqn.ends_with(api_name))
            .map(|n| PathBuf::from(&n.location.file))
            .unwrap_or_else(|| PathBuf::from(format!("{api_name}.cls")))
    };
    for (api_name, _) in &results.class_symbols {
        let enclosing = api_name
            .rsplit_once('.')
            .map(|(outer, _)| outer.to_string());
        registry.insert_user_declared(api_name, ApexTypeKind::Class, path_for(api_name), enclosing);
    }
    for (api_name, json) in &results.class_symbols {
        let symbols: ApexClassSymbols =
            serde_json::from_str(json).unwrap_or_else(|e| panic!("deserialise {api_name}: {e}"));
        assert!(
            registry.attach_symbols(api_name, symbols),
            "attach_symbols failed for {api_name}",
        );
    }
    registry
}

async fn resolve() -> (SyntaxResults, Vec<Edge>) {
    let paths = fixture_paths();
    assert!(!paths.is_empty(), "fixture bundle empty");
    let hints = extract(&paths).await;
    let registry = build_registry(&hints);
    let resolver = ApexHeuristicResolver::new(registry);
    let edges = resolver.resolve(&hints).await.expect("resolve").call_edges;
    (hints, edges)
}

/// The seven cross-language keyword-names under test. Every entry must
/// round-trip: extracted as a Function node AND targeted by the one
/// expected inbound call from `callAll`.
const KEYWORD_METHOD_NAMES: &[&str] = &["match", "type", "lambda", "in", "of", "object", "where"];

#[tokio::test]
async fn r46_all_cross_language_keyword_methods_extract_as_function_nodes() {
    let (hints, _edges) = resolve().await;
    let function_fqns: Vec<&str> = hints
        .symbols
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Function))
        .map(|n| n.fqn.as_str())
        .collect();

    for &name in KEYWORD_METHOD_NAMES {
        let expected_fqn_tail = format!("CrossLangKeywordNames::CrossLangKeywordNames::{name}()",);
        let matched = function_fqns
            .iter()
            .any(|fqn| fqn.ends_with(&expected_fqn_tail));
        assert!(
            matched,
            "R46 regression: Apex method named `{name}` was NOT emitted as a Function node. \
             Extracted functions were: {function_fqns:#?}",
        );
    }
}

fn resolve_id_to_fqn<'a>(hints: &'a SyntaxResults, id: &str) -> Option<&'a str> {
    hints
        .symbols
        .iter()
        .find(|n| n.id == id)
        .map(|n| n.fqn.as_str())
}

#[tokio::test]
async fn r46_each_keyword_method_receives_exactly_one_call_edge_from_callall() {
    let (hints, edges) = resolve().await;

    let caller_tail = "CrossLangKeywordNames::CrossLangKeywordNames::callAll()";

    for &name in KEYWORD_METHOD_NAMES {
        let callee_tail = format!("CrossLangKeywordNames::CrossLangKeywordNames::{name}()",);
        let inbound_from_callall = edges
            .iter()
            .filter(|e| {
                let from_fqn = resolve_id_to_fqn(&hints, &e.from_id);
                let to_fqn = resolve_id_to_fqn(&hints, &e.to_id);
                from_fqn.is_some_and(|f| f.ends_with(caller_tail))
                    && to_fqn.is_some_and(|t| t.ends_with(&callee_tail))
            })
            .count();
        assert_eq!(
            inbound_from_callall, 1,
            "R46 regression: method `{name}` expected 1 inbound call edge from \
             CrossLangKeywordNames::callAll(), got {inbound_from_callall}. \
             Either the method was dropped at extraction (Function node missing) or \
             the call site inside callAll() was dropped (call_site extraction miss).",
        );
    }
}
