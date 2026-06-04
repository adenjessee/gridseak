//! TR-A.3 acceptance driver — field-type-aware dispatch fixtures.
//!
//! Walks the three fixture scenarios under
//! `graphengine-parsing/tests/fixtures/apex_resolver/r23_a3_*` through
//! the real extractor + heuristic resolver pipeline and asserts each
//! one lands exactly one Medium-confidence call edge on the declared
//! target method. This is the live-recall counterpart to the
//! isolated-logic unit tests inside
//! `apex/field_type_resolver.rs::tests`.
//!
//! # What this suite actually guarantees
//!
//! For every fixture, the driver exercises end-to-end:
//!
//! 1. `TreeSitterExtractor::extract` parses the .cls files,
//!    populates `SyntaxResults.symbols`, `.call_sites` (including
//!    the new `receiver_text` carried over from the shared
//!    extractor YAML), `.class_symbols`, and `.local_var_scopes`.
//! 2. `ApexClassRegistry` is seeded from the extractor's per-class
//!    symbols (same helper used by the TR-A.1 / TR-A.2 driver in
//!    `apex_resolver_r23_ctor_fixtures.rs`).
//! 3. `ApexHeuristicResolver::resolve` runs the field-type-aware
//!    dispatch arm (see
//!    `apex/field_type_resolver.rs::resolve_field_type_call`) for
//!    each call site, falling through to the existing short-name
//!    path only when dispatch declines.
//! 4. The edge count and confidence tier is checked against plan
//!    §4.2.
//!
//! The driver mirrors
//! [`crate::apex_resolver_r23_ctor_fixtures`]'s registry-seeding
//! caveat: the test-only `build_registry_from_results` helper will
//! be deleted once the orchestrator wires registry seeding on the
//! live `ge-scan` path (see `PHASE_A_EXECUTION_PLAN.md` §10 commit
//! sequencing).

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
/// payload + the `Struct` nodes in `SyntaxResults.symbols`. Duplicated
/// from `apex_resolver_r23_ctor_fixtures.rs` so the two drivers don't
/// share a `mod common` that pulls either one's assertions into the
/// other's compile graph.
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

/// Count only the edges that land on a Function whose FQN matches
/// the plan's target. Calls from the caller's ctor / other methods
/// (e.g. `new UTIL_Permissions()`) generate their own ctor edges;
/// we don't want those polluting the count.
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

fn assert_single_medium_heuristic_edge(
    edges: &[&graphengine_parsing::domain::Edge],
    scenario: &str,
) {
    assert_eq!(
        edges.len(),
        1,
        "[{scenario}] expected exactly 1 field-dispatch call edge to the target method; got {}: {:#?}",
        edges.len(),
        edges.iter().map(|e| (&e.from_id, &e.to_id, &e.provenance)).collect::<Vec<_>>(),
    );
    let edge = edges[0];
    assert_eq!(
        edge.provenance.source,
        ProvenanceSource::Heuristic,
        "[{scenario}] edge must be heuristic-provenance",
    );
    assert_eq!(
        edge.provenance.confidence,
        Confidence::Medium,
        "[{scenario}] §4.2 requires Medium confidence (unique method match via field type)",
    );
}

// ---------------------------------------------------------------------------
// Scenario 1 — typed-field dispatch (enclosing-class fields path).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a3_typed_field_dispatch_resolves_single_medium_edge() {
    let scenario = "r23_a3_typed_field_dispatch";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;
    let target_edges = call_edges_to_method_fqn(
        &hints,
        &edges,
        "UTIL_Permissions::UTIL_Permissions::canUpdate",
    );
    assert_single_medium_heuristic_edge(&target_edges, scenario);

    // Source should be the caller's instance method, not the ctor
    // (the ctor invokes `new UTIL_Permissions()` but not
    // `canUpdate`).
    let source = hints
        .symbols
        .iter()
        .find(|n| n.id == target_edges[0].from_id)
        .expect("source must exist");
    assert!(
        source.fqn.contains("checkAccountUpdate"),
        "edge source must be checkAccountUpdate; got {}",
        source.fqn,
    );
}

// ---------------------------------------------------------------------------
// Scenario 2 — DI-constructor-injected field (enclosing-class fields
// path, field populated by ctor parameter).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a3_di_constructor_injected_resolves_single_medium_edge() {
    let scenario = "r23_a3_di_constructor_injected";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;
    let target_edges = call_edges_to_method_fqn(
        &hints,
        &edges,
        "SfdoInstrumentationService::SfdoInstrumentationService::log",
    );
    assert_single_medium_heuristic_edge(&target_edges, scenario);

    let source = hints
        .symbols
        .iter()
        .find(|n| n.id == target_edges[0].from_id)
        .expect("source must exist");
    assert!(
        source.fqn.contains("emit"),
        "edge source must be emit(); got {}",
        source.fqn,
    );
}

// ---------------------------------------------------------------------------
// Scenario 3 — domain-layer typed local (method-body local-scope path).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a3_domain_layer_typed_local_resolves_single_medium_edge() {
    let scenario = "r23_a3_domain_layer_typed";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;
    let target_edges =
        call_edges_to_method_fqn(&hints, &edges, "Contacts::Contacts::loadAccountByIdMap");
    assert_single_medium_heuristic_edge(&target_edges, scenario);

    let source = hints
        .symbols
        .iter()
        .find(|n| n.id == target_edges[0].from_id)
        .expect("source must exist");
    assert!(
        source.fqn.contains("fetchAll"),
        "edge source must be fetchAll(); got {}",
        source.fqn,
    );
    assert!(
        source.location.file.ends_with("ContactsDomainCaller.cls"),
        "source must live in the caller file; got {}",
        source.location.file,
    );
}
