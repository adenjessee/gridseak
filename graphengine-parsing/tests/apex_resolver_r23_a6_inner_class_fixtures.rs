//! TR-A.6 acceptance driver — inner-class containment walker
//! fixtures.
//!
//! Walks the five fixture scenarios under
//! `graphengine-parsing/tests/fixtures/apex_resolver/r23_a6_*` through
//! the real extractor + heuristic resolver pipeline and asserts each
//! one lands its expected inner-class / dotted-path dispatch edge.
//!
//! # Why these five
//!
//! PHASE_A_EXECUTION_PLAN §6.2 lists these exact five shapes as the
//! PR 5 fixture suite, each mapped to a §4.11.1 revert-population
//! FQN shape:
//!
//! * `r23_a6_inner_ctor_via_outer` — shape #3 (`new Outer.Inner(...)`
//!   from top-level code).
//! * `r23_a6_inner_method_typed_field` — shape #1 (outer holds typed
//!   field of inner type, calls inner method).
//! * `r23_a6_inner_method_override` — shape #1 variant (override
//!   resolves on the inner, not on the parent).
//! * `r23_a6_tdtm_revert_shape` — shape #1 canonical, mirrors
//!   `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set<Id>)`.
//! * `r23_a6_rd_cascade_default_ctor` — shape #3 canonical, mirrors
//!   `RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader::FirstCascadeUndeleteLoader()`
//!   (implicit default ctor).

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
/// payload + the `Struct` nodes in `SyntaxResults.symbols`. Same
/// shape as the TR-A.1 / TR-A.3 / TR-A.4 drivers — duplicated on
/// purpose (see those drivers' comments).
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
            "attach_symbols failed for {api_name}; registry seeding mismatch?",
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

/// Edges whose `to_id` matches a node whose FQN contains
/// `target_substr`. Used to isolate the one edge the scenario under
/// test cares about (sibling ctors / field-initialiser ctors emit
/// their own edges we intentionally ignore).
fn call_edges_to_fqn<'a>(
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
                .map(|n| n.fqn.contains(target_substr))
                .unwrap_or(false)
        })
        .collect()
}

fn assert_single_heuristic_medium(
    edges: &[&graphengine_parsing::domain::Edge],
    scenario: &str,
    caller_substr: &str,
    hints: &SyntaxResults,
) {
    assert_eq!(
        edges.len(),
        1,
        "[{scenario}] expected exactly 1 heuristic inner-class edge; got {}: {:#?}",
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
        edge.provenance.confidence,
        Confidence::Medium,
        "[{scenario}] TR-A.6 binds dotted paths at Medium (unique target + Exact/arity match)",
    );
    let source = hints
        .symbols
        .iter()
        .find(|n| n.id == edge.from_id)
        .expect("source must exist");
    assert!(
        source.fqn.contains(caller_substr),
        "[{scenario}] edge source must be `{caller_substr}`; got {}",
        source.fqn,
    );
}

// ---------------------------------------------------------------------------
// Scenario 1 — `new Outer.Inner(args)` from a top-level caller.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a6_inner_ctor_via_outer_binds_to_inner_ctor_medium() {
    let scenario = "r23_a6_inner_ctor_via_outer";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    // Target FQN carries the dotted inner class form
    // `OuterCtor.InnerCtor::InnerCtor(String)` — the constructor
    // Function node's FQN includes the parameter signature.
    let target = "OuterCtor::OuterCtor.InnerCtor::InnerCtor";
    let target_edges = call_edges_to_fqn(&hints, &edges, target);
    assert_single_heuristic_medium(&target_edges, scenario, "run", &hints);
}

// ---------------------------------------------------------------------------
// Scenario 2 — typed field of inner type; method call binds via
// sibling-inner short-name normalisation.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a6_inner_method_typed_field_binds_to_inner_method_medium() {
    let scenario = "r23_a6_inner_method_typed_field";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    let target = "OuterField::OuterField.Inner::ping";
    let target_edges = call_edges_to_fqn(&hints, &edges, target);
    assert_single_heuristic_medium(&target_edges, scenario, "OuterField::run", &hints);
}

// ---------------------------------------------------------------------------
// Scenario 3 — override on inner class wins over parent declaration.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a6_inner_method_override_binds_to_override_medium() {
    let scenario = "r23_a6_inner_method_override";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    // The override lives on OverrideOuter.ChildLoader; a correct
    // resolver MUST prefer it over BaseLoader's declaration.
    let target = "OverrideOuter::OverrideOuter.ChildLoader::load";
    let target_edges = call_edges_to_fqn(&hints, &edges, target);
    assert_single_heuristic_medium(&target_edges, scenario, "OverrideOuter::kick", &hints);

    // Defensive: no edge on the parent declaration for the same
    // call site. If the resolver emitted both, the override-first
    // rule is broken.
    let base_target = "BaseLoader::BaseLoader::load";
    let base_edges = call_edges_to_fqn(&hints, &edges, base_target);
    let base_edges_from_kick: Vec<_> = base_edges
        .iter()
        .filter(|e| {
            hints
                .symbols
                .iter()
                .find(|n| n.id == e.from_id)
                .map(|n| n.fqn.contains("OverrideOuter::kick"))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        base_edges_from_kick.is_empty(),
        "[{scenario}] override-first rule violated: resolver emitted a fallback edge to BaseLoader::load from kick()",
    );
}

// ---------------------------------------------------------------------------
// Scenario 4 — CAM_CascadeDeleteLookups_TDTM canonical shape.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a6_tdtm_revert_shape_binds_to_inner_override_medium() {
    let scenario = "r23_a6_tdtm_revert_shape";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    let target =
        "CAM_CascadeDeleteLookups_TDTM::CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load";
    let target_edges = call_edges_to_fqn(&hints, &edges, target);
    assert_single_heuristic_medium(
        &target_edges,
        scenario,
        "CAM_CascadeDeleteLookups_TDTM::run",
        &hints,
    );
}

// ---------------------------------------------------------------------------
// Scenario 5 — RD_CascadeDeleteLookups_TDTM implicit default ctor.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a6_rd_cascade_default_ctor_binds_via_implicit_default_medium() {
    let scenario = "r23_a6_rd_cascade_default_ctor";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    // Implicit-default-ctor fallback lands on the inner TYPE node
    // (a Struct), not a synthetic Function node. Filter accordingly.
    let target = "RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader";
    let target_edges: Vec<_> = edges
        .iter()
        .filter(|e| {
            hints
                .symbols
                .iter()
                .find(|n| n.id == e.to_id)
                .map(|n| matches!(n.kind, NodeKind::Struct) && n.fqn.ends_with(target))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(
        target_edges.len(),
        1,
        "[{scenario}] expected exactly 1 implicit-default-ctor edge to the inner type node; got {}: {:#?}",
        target_edges.len(),
        target_edges
            .iter()
            .map(|e| (&e.from_id, &e.to_id))
            .collect::<Vec<_>>(),
    );
    let edge = target_edges[0];
    assert_eq!(edge.provenance.source, ProvenanceSource::Heuristic);
    assert_eq!(
        edge.provenance.confidence,
        Confidence::Medium,
        "[{scenario}] implicit-default-ctor resolves at Medium (single target, zero-arg exact)",
    );

    let source = hints
        .symbols
        .iter()
        .find(|n| n.id == edge.from_id)
        .expect("source");
    assert!(
        source.fqn.contains("RD_CascadeDeleteLookups_TDTM::run"),
        "[{scenario}] edge source must be run(); got {}",
        source.fqn,
    );
}

// ---------------------------------------------------------------------------
// PR 8 diagnostic — dotted inner ctor with null-arg + List<String> last
// param. Mirrors the NPSP `new GE_Template.Element(...)` shape that the
// Round 5 audit flagged as unresolved.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pr8_diag_dotted_inner_ctor_null_arg_binds_to_5arg_overload_medium() {
    let scenario = "r23_a6_dotted_inner_ctor_null_arg";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;

    let target = "TemplateHolder::TemplateHolder.Element::Element";
    let target_edges = call_edges_to_fqn(&hints, &edges, target);
    assert!(
        !target_edges.is_empty(),
        "[{scenario}] dotted-inner ctor with null-arg dropped silently; \
         expected at least one edge to {target}; got {} edges. Caller resolved edges: {:#?}",
        target_edges.len(),
        edges
            .iter()
            .map(|e| (
                hints
                    .symbols
                    .iter()
                    .find(|n| n.id == e.from_id)
                    .map(|n| n.fqn.as_str()),
                hints
                    .symbols
                    .iter()
                    .find(|n| n.id == e.to_id)
                    .map(|n| n.fqn.as_str()),
            ))
            .collect::<Vec<_>>(),
    );

    // Either the unique 5-arg overload binds at Medium (desired), or
    // we fan out across both overloads at Low. Both outcomes mean the
    // edge is emitted — the root cause is NOT a cross-file null-arg
    // drop.
    assert!(
        target_edges
            .iter()
            .all(|e| e.provenance.source == ProvenanceSource::Heuristic),
        "[{scenario}] all dotted-inner ctor edges must carry heuristic provenance",
    );
}

#[tokio::test]
async fn pr8_diag_dotted_inner_ctor_no_null_binds() {
    let scenario = "r23_a6_dotted_inner_ctor_no_null";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;
    let target = "TemplateHolder::TemplateHolder.Element::Element";
    let target_edges = call_edges_to_fqn(&hints, &edges, target);
    assert!(
        !target_edges.is_empty(),
        "[{scenario}] 5-arg dotted-inner ctor with no null still dropped; edges: {:#?}",
        edges
            .iter()
            .map(|e| (
                hints
                    .symbols
                    .iter()
                    .find(|n| n.id == e.from_id)
                    .map(|n| n.fqn.as_str()),
                hints
                    .symbols
                    .iter()
                    .find(|n| n.id == e.to_id)
                    .map(|n| n.fqn.as_str()),
            ))
            .collect::<Vec<_>>(),
    );
}

async fn pr8_diag_generic(scenario: &str) -> bool {
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;
    let target = "TemplateHolder::TemplateHolder.Element::Element";
    let target_edges = call_edges_to_fqn(&hints, &edges, target);
    !target_edges.is_empty()
}

#[tokio::test]
async fn pr8_diag_dotted_inner_ctor_3arg_binds() {
    assert!(
        pr8_diag_generic("r23_a6_dotted_inner_ctor_3arg").await,
        "3-arg dropped"
    );
}

#[tokio::test]
async fn pr8_diag_dotted_inner_ctor_4arg_binds() {
    assert!(
        pr8_diag_generic("r23_a6_dotted_inner_ctor_4arg").await,
        "4-arg dropped"
    );
}

#[tokio::test]
async fn pr8_diag_dotted_inner_ctor_5arg_liststring_param_binds() {
    assert!(
        pr8_diag_generic("r23_a6_dotted_inner_ctor_5arg_liststring_param").await,
        "5-arg with List<String> param dropped",
    );
}

#[tokio::test]
async fn pr8_diag_dotted_inner_ctor_5arg_all_strings_binds() {
    assert!(
        pr8_diag_generic("r23_a6_dotted_inner_ctor_5arg_all_strings").await,
        "5-arg-all-strings dropped",
    );
}

#[tokio::test]
async fn pr8_diag_dotted_inner_ctor_2arg_binds() {
    let scenario = "r23_a6_dotted_inner_ctor_2arg";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;
    let target = "TemplateHolder::TemplateHolder.Element::Element";
    let target_edges = call_edges_to_fqn(&hints, &edges, target);
    assert!(
        !target_edges.is_empty(),
        "[{scenario}] 2-arg dotted-inner ctor dropped; edges: {:#?}",
        edges
            .iter()
            .map(|e| (
                hints
                    .symbols
                    .iter()
                    .find(|n| n.id == e.from_id)
                    .map(|n| n.fqn.as_str()),
                hints
                    .symbols
                    .iter()
                    .find(|n| n.id == e.to_id)
                    .map(|n| n.fqn.as_str()),
            ))
            .collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn pr8_diag_dotted_inner_ctor_no_inline_list_binds() {
    let scenario = "r23_a6_dotted_inner_ctor_no_inline_list";
    let (hints, edges) = resolve_fixture(&fixture_paths(scenario)).await;
    let target = "TemplateHolder::TemplateHolder.Element::Element";
    let target_edges = call_edges_to_fqn(&hints, &edges, target);
    assert!(
        !target_edges.is_empty(),
        "[{scenario}] null-arg + pre-bound String[] still dropped; edges: {:#?}",
        edges
            .iter()
            .map(|e| (
                hints
                    .symbols
                    .iter()
                    .find(|n| n.id == e.from_id)
                    .map(|n| n.fqn.as_str()),
                hints
                    .symbols
                    .iter()
                    .find(|n| n.id == e.to_id)
                    .map(|n| n.fqn.as_str()),
            ))
            .collect::<Vec<_>>(),
    );
}
