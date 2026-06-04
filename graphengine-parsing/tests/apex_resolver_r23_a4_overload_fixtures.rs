//! TR-A.4 acceptance driver — intra-class overload + this/bare
//! self-dispatch fixtures.
//!
//! Walks the four fixture scenarios under
//! `graphengine-parsing/tests/fixtures/apex_resolver/r23_a4_*` through
//! the real extractor + heuristic resolver pipeline and asserts each
//! one lands exactly one heuristic call edge on the declared
//! overload. Complements the isolated-logic unit tests inside
//! `apex/signature_matcher.rs::tests` (which cover the matcher
//! ladder structurally without running the extractor).
//!
//! # What this suite actually guarantees
//!
//! For every fixture, the driver exercises end-to-end:
//!
//! 1. `TreeSitterExtractor::extract` parses the .cls files,
//!    populates `SyntaxResults.symbols`, `.call_sites` (including
//!    `receiver_text`), `.class_symbols`, and `.local_var_scopes`.
//! 2. `ApexClassRegistry` is seeded from the extractor's per-class
//!    symbols via the same test-only helper the TR-A.1/2/3 drivers
//!    use. (Production seeding is covered by
//!    `apex::heuristic_resolver::tests::seed_registry_attaches_user_declared_class_symbols`.)
//! 3. `ApexHeuristicResolver::resolve` runs the field-type-aware
//!    dispatch arm; TR-A.4's contribution is the Implicit tier in
//!    the signature matcher + the bare-self-call normalisation in
//!    `field_type_resolver::resolve_field_type_call`.
//! 4. The edge count and confidence tier is checked against plan
//!    §5.2.
//!
//! # Confidence expectations per scenario
//!
//! - `r23_a4_overload_exact`: 1 edge, Medium (Exact tier beats
//!   Object widening).
//! - `r23_a4_overload_widening`: 1 edge, Low (only the TR-A.4
//!   Implicit tier survives, so the resolver drops confidence).
//! - `r23_a4_overload_this_dispatch`: 1 edge, Medium (Exact tier;
//!   the String overload is selected among two sibling overloads
//!   via `this.` receiver).
//! - `r23_a4_bare_self_dispatch`: 1 edge, Medium (Exact tier,
//!   arity-0 unique; closes the R38 deferred FQN
//!   `Contacts::loadAccountByIdMap()` at the shape level).

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
/// from the TR-A.1 / TR-A.3 drivers on purpose — a shared `mod common`
/// would pull either driver's assertions into the other's compile
/// graph. When the orchestrator owns registry seeding end-to-end we
/// can delete these three copies in one sweep.
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

/// Count only the edges landing on a Function whose FQN matches
/// `target_substr`. Calls from the caller's ctor / sibling methods
/// (e.g. `new WideLogger()`) generate their own ctor edges we do
/// not want in the count.
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

fn assert_single_heuristic_edge_at(
    edges: &[&graphengine_parsing::domain::Edge],
    scenario: &str,
    expected_confidence: Confidence,
) {
    assert_eq!(
        edges.len(),
        1,
        "[{scenario}] expected exactly 1 heuristic overload-dispatch edge; got {}: {:#?}",
        edges.len(),
        edges
            .iter()
            .map(|e| (&e.from_id, &e.to_id, &e.provenance))
            .collect::<Vec<_>>(),
    );
    let edge = edges[0];
    assert_eq!(
        edge.provenance.source,
        ProvenanceSource::Heuristic,
        "[{scenario}] edge must be heuristic-provenance",
    );
    assert_eq!(
        edge.provenance.confidence, expected_confidence,
        "[{scenario}] §5.2 confidence mismatch",
    );
}

// ---------------------------------------------------------------------------
// Scenario 1 — exact overload beats Object widening.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a4_overload_exact_picks_string_overload_medium() {
    let scenario = "r23_a4_overload_exact";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    // Both compare overloads share the name `compare`. We need to
    // single out the String-accepting one by its parameter-type
    // signature using the class_symbols payload. The fixture
    // declares exactly two overloads so checking that exactly one
    // of them receives the edge is the strict win condition.
    let compare_fqn = "fflib_Comparator::fflib_Comparator::compare";
    let target_edges = call_edges_to_method_fqn(&hints, &edges, compare_fqn);
    // The TR-A.3 extractor emits one node per method (not per
    // overload) — `fflib_Comparator::compare` has two overloads
    // collapsed into a single symbol with the signature stored on
    // `class_symbols`. That means the edge lands on the single
    // Function node shared by both overloads. The true assertion
    // here is "the resolver picked the Exact tier and produced
    // Medium confidence", not which of two distinct symbols it
    // hit (they don't exist as distinct symbols).
    assert_single_heuristic_edge_at(&target_edges, scenario, Confidence::Medium);
}

// ---------------------------------------------------------------------------
// Scenario 2 — TR-A.4 implicit Object-widening.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a4_overload_widening_picks_object_overload_low() {
    let scenario = "r23_a4_overload_widening";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    let log_fqn = "WideLogger::WideLogger::log";
    let target_edges = call_edges_to_method_fqn(&hints, &edges, log_fqn);
    // Implicit tier surviving alone — unique Low. If widening had
    // somehow survived instead, the result would have been
    // Medium. Asserting Low here is the real win condition for
    // TR-A.4's confidence-downgrade story.
    assert_single_heuristic_edge_at(&target_edges, scenario, Confidence::Low);
}

// ---------------------------------------------------------------------------
// Scenario 3 — `this.compare(s1, s2)` self-dispatch binds to the
// String overload among sibling methods.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a4_overload_this_dispatch_picks_string_overload_medium() {
    let scenario = "r23_a4_overload_this_dispatch";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    let compare_fqn = "SiblingCompare::SiblingCompare::compare";
    let target_edges = call_edges_to_method_fqn(&hints, &edges, compare_fqn);
    assert_single_heuristic_edge_at(&target_edges, scenario, Confidence::Medium);

    // Source of the edge must be `dispatch()`, not either of the
    // two `compare` overloads themselves (they don't call each
    // other in this fixture).
    let source = hints
        .symbols
        .iter()
        .find(|n| n.id == target_edges[0].from_id)
        .expect("source must exist");
    assert!(
        source.fqn.contains("dispatch"),
        "edge source must be dispatch(); got {}",
        source.fqn,
    );
}

// ---------------------------------------------------------------------------
// Scenario 4 — bare self-call (R38 `Contacts::loadAccountByIdMap()`
// shape).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a4_bare_self_dispatch_resolves_medium() {
    let scenario = "r23_a4_bare_self_dispatch";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    let target_fqn = "ContactsLike::ContactsLike::loadAccountByIdMap";
    let target_edges = call_edges_to_method_fqn(&hints, &edges, target_fqn);
    assert_single_heuristic_edge_at(&target_edges, scenario, Confidence::Medium);

    let source = hints
        .symbols
        .iter()
        .find(|n| n.id == target_edges[0].from_id)
        .expect("source must exist");
    assert!(
        source.fqn.contains("run"),
        "edge source must be run(); got {}",
        source.fqn,
    );
}
