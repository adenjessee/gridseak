//! R40 acceptance driver — `ClassName.staticMethod(...)` and
//! `Outer.Inner.staticMethod(...)` dispatch fixtures.
//!
//! The NPSP rev-7 Round-5 hand-audit recorded
//! `ADV_PackageInfo_SVC::getApiNPSP()` with `fan_in = 0` and the
//! `no_callers` tag, even though `TDTM_ObjectDataGateway` invokes
//! it twice on lines 99-100 of the real source. Root cause was the
//! missing static-dispatch arm in `field_type_resolver`: the
//! receiver `ADV_PackageInfo_SVC` was a `BareIdent` that wasn't a
//! local variable or instance field, and the resolver declined
//! rather than probing the registry as a TypeName. The same shape
//! existed for dotted receivers (`Outer.Inner.staticMethod()`) —
//! `DottedDefer` previously discarded the text and auto-deferred
//! to TR-A.6, which itself has no static-dispatch arm.
//!
//! This driver walks the two fixture pairs end-to-end through the
//! real extractor + heuristic resolver and asserts the exact edge
//! count + Medium-confidence tier on the static method targets.
//!
//! Companion unit tests in
//! `apex/field_type_resolver.rs::tests` cover:
//!   - `normalises_this_bare_and_dotted_field` — payload of
//!     `DottedDefer(text)` survives normalisation.
//!   - `declines_on_dotted_receiver` — receivers that don't match
//!     a registered type still return `None` (no spurious edges).
//!
//! Together these guard the R40 ladder structurally and at the
//! pipeline integration level.

use graphengine_parsing::application::ports::{SemanticResolver, SyntaxExtractor, SyntaxResults};
use graphengine_parsing::domain::apex::class_symbols::ApexClassSymbols;
use graphengine_parsing::domain::{Confidence, NodeKind, ProvenanceSource};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::language::apex::class_registry::{
    ApexClassRegistry, ApexTypeKind,
};
use graphengine_parsing::syntax::language::apex::ApexHeuristicResolver;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::{Path, PathBuf};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("apex_resolver")
}

fn fixture_paths(scenario: &str) -> Vec<PathBuf> {
    let dir = fixture_root().join(scenario);
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("cls"))
        .collect();
    out.sort();
    assert!(
        !out.is_empty(),
        "no .cls files found under fixture scenario `{scenario}`",
    );
    out
}

async fn extract(paths: &[PathBuf]) -> SyntaxResults {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    extractor.extract(paths).await.expect("parse apex fixtures")
}

/// Seed an `ApexClassRegistry` from the extractor's `class_symbols`
/// payload + the `Struct` nodes in `SyntaxResults.symbols`.
/// Mirrors the helper used by `apex_resolver_r23_a4_overload_fixtures.rs`
/// and the TR-A.1 / TR-A.3 drivers — kept inline on purpose to avoid
/// pulling either driver's assertions into the other's compile graph.
fn build_registry_from_results(results: &SyntaxResults) -> ApexClassRegistry {
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

    for (api_name, _json) in &results.class_symbols {
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
            "attach_symbols failed for {api_name}; is the registry entry missing?",
        );
    }
    registry
}

async fn resolve_fixture(
    paths: &[PathBuf],
) -> (SyntaxResults, Vec<graphengine_parsing::domain::Edge>) {
    let hints = extract(paths).await;
    let registry = build_registry_from_results(&hints);
    let resolver = ApexHeuristicResolver::new(registry);
    let edges = resolver.resolve(&hints).await.expect("resolve").call_edges;
    (hints, edges)
}

fn call_edges_to_method_fqn<'a>(
    hints: &'a SyntaxResults,
    edges: &'a [graphengine_parsing::domain::Edge],
    target_substr: &str,
) -> Vec<&'a graphengine_parsing::domain::Edge> {
    edges
        .iter()
        .filter(|e| {
            hints
                .symbols
                .iter()
                .find(|n| n.id == e.to_id)
                .map(|n| matches!(n.kind, NodeKind::Function) && n.fqn.contains(target_substr))
                .unwrap_or(false)
        })
        .collect()
}

fn assert_heuristic_edges_on(
    edges: &[&graphengine_parsing::domain::Edge],
    scenario: &str,
    expected_count: usize,
    expected_confidence: Confidence,
) {
    assert_eq!(
        edges.len(),
        expected_count,
        "[{scenario}] expected {expected_count} heuristic static-dispatch edges; got {}: {:#?}",
        edges.len(),
        edges
            .iter()
            .map(|e| (&e.from_id, &e.to_id, &e.provenance))
            .collect::<Vec<_>>(),
    );
    for edge in edges {
        assert_eq!(
            edge.provenance.source,
            ProvenanceSource::Heuristic,
            "[{scenario}] edges must be heuristic-provenance",
        );
        assert_eq!(
            edge.provenance.confidence, expected_confidence,
            "[{scenario}] §5.2 confidence mismatch on edge {edge:?}",
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario 1 — `ClassName.staticMethod()` (NPSP `ADV_PackageInfo_SVC`
// shape).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r40_static_dispatch_bare_class_name_resolves_two_edges_medium() {
    let scenario = "r23_a4_static_dispatch_bare";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    // The caller invokes both `useAdv()` and `getApiNPSP()`. Each
    // should land exactly one Medium-confidence edge.
    let useadv_edges = call_edges_to_method_fqn(
        &hints,
        &edges,
        "ADV_PackageInfo_SVC_Like::ADV_PackageInfo_SVC_Like::useAdv",
    );
    assert_heuristic_edges_on(&useadv_edges, scenario, 1, Confidence::Medium);

    let getapinpsp_edges = call_edges_to_method_fqn(
        &hints,
        &edges,
        "ADV_PackageInfo_SVC_Like::ADV_PackageInfo_SVC_Like::getApiNPSP",
    );
    assert_heuristic_edges_on(&getapinpsp_edges, scenario, 1, Confidence::Medium);

    // Source on every edge must be the caller's `run()` method,
    // never one of the static methods themselves (they don't call
    // each other in this fixture).
    for edge in useadv_edges.iter().chain(getapinpsp_edges.iter()) {
        let source = hints
            .symbols
            .iter()
            .find(|n| n.id == edge.from_id)
            .expect("source must exist");
        assert!(
            source.fqn.contains("ADV_PackageInfo_SVC_Like_Caller") && source.fqn.contains("run"),
            "edge source must be caller::run(); got {}",
            source.fqn,
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario 2 — `Outer.Inner.staticMethod()` dotted-receiver dispatch.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r40_static_dispatch_dotted_inner_resolves_one_edge_medium() {
    let scenario = "r23_a4_static_dispatch_dotted_inner";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    // The static method lives at `OuterHolder.InnerSvc::describe()`;
    // the registry keys the inner class as `OuterHolder.InnerSvc`.
    let target_edges = call_edges_to_method_fqn(
        &hints,
        &edges,
        "OuterHolder::OuterHolder.InnerSvc::describe",
    );
    assert_heuristic_edges_on(&target_edges, scenario, 1, Confidence::Medium);

    let edge = target_edges[0];
    let source = hints
        .symbols
        .iter()
        .find(|n| n.id == edge.from_id)
        .expect("source must exist");
    assert!(
        source.fqn.contains("OuterHolderCaller") && source.fqn.contains("run"),
        "edge source must be OuterHolderCaller::run(); got {}",
        source.fqn,
    );
}
