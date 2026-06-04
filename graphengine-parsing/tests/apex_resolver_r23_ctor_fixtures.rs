//! TR-A.1 + TR-A.2 acceptance driver — constructor resolution fixtures.
//!
//! Walks the 7 fixture scenarios under
//! `graphengine-parsing/tests/fixtures/apex_resolver/` through the
//! real extractor + heuristic resolver pipeline and asserts each one
//! lands the exact number of call edges the plan's §3.2 table
//! requires. This is the live-recall counterpart to the extractor-
//! level locks in `extractor_constructor_fixtures.rs` and the
//! isolated-logic unit tests inside `apex/heuristic_resolver.rs`.
//!
//! # What this suite actually guarantees
//!
//! For every fixture, this driver exercises **end-to-end** the path
//! that produces a heuristic call edge:
//!
//! 1. `TreeSitterExtractor::extract` parses the .cls files and
//!    populates `SyntaxResults.symbols`, `.call_sites` (with
//!    `CallSite.arg_types` inferred), and `.class_symbols`.
//! 2. The `ApexClassRegistry` is seeded from the extractor's
//!    per-class symbols (see `build_registry_from_results`). The
//!    production orchestrator does not yet wire this in PR-1 — seeding
//!    here keeps the test hermetic and mirrors the Commit 2 wiring the
//!    plan stages under §10.
//! 3. `ApexHeuristicResolver::resolve` runs the TR-A.1 + TR-A.2
//!    constructor arm against the registry + call sites.
//! 4. The edge count and confidence tier is checked against §3.2.
//!
//! # Registry seeding caveat
//!
//! The `build_registry_from_results` helper is a test-only shim that
//! stands in for the production pipeline step the plan schedules in
//! PR-2 / Commit 2 (see `PHASE_A_EXECUTION_PLAN.md` §10 "Commit
//! sequencing"). When that production wiring lands, this helper
//! SHOULD be deleted and the test driver should rely on whatever
//! orchestrator-level hook the pipeline exposes. Leaving this here
//! indefinitely creates a drift risk — registry seeding on the live
//! `ge-scan` path and on this test path would be two implementations
//! of one contract.

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

/// Resolve every `.cls` file directly underneath the scenario
/// directory. Apex requires the file stem to equal the top-level
/// class name, so scenario identity lives on the containing directory
/// (matching the plan's `r23_a1_*` / `r23_a2_*` fixture names) and
/// each enclosed `.cls` file is named for the class it declares. See
/// the per-fixture `*.cls` header for the plan's §8.3 FQN row.
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
/// payload + the `Struct` nodes in `SyntaxResults.symbols`. See the
/// module-level "Registry seeding caveat" for why this lives in the
/// test and not in the production pipeline yet.
///
/// Inner vs. top-level is derived from the `api_name` shape: a dotted
/// key (`Outer.Inner`) is registered with `enclosing = Some(outer)`
/// so the registry's primary-key lookup (dotted) binds it, and the
/// resolver's §3.1 step 1 sibling-inner fast path can rewrite short
/// calls into the dotted form before looking up symbols.
fn build_registry_from_results(results: &SyntaxResults) -> ApexClassRegistry {
    let mut registry = ApexClassRegistry::with_standard_preload();

    // File path per declared api_name: Struct nodes carry the file
    // the class was declared in. `class_symbols` does not carry the
    // path, so we stitch them here. If no Struct node is found we
    // fall back to a synthetic path — the registry only uses path
    // for reporting, not for resolution correctness.
    let path_for = |api_name: &str| -> PathBuf {
        // The Struct node's FQN ends with the api_name (dotted form
        // for inner classes, short for top-level).
        results
            .symbols
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Struct))
            .find(|n| n.fqn.ends_with(api_name))
            .map(|n| PathBuf::from(&n.location.file))
            .unwrap_or_else(|| PathBuf::from(format!("{api_name}.cls")))
    };

    // First pass — declare every class in the registry. `attach_symbols`
    // requires the entry to already exist.
    for (api_name, _json) in &results.class_symbols {
        let enclosing = api_name
            .rsplit_once('.')
            .map(|(outer, _)| outer.to_string());
        registry.insert_user_declared(api_name, ApexTypeKind::Class, path_for(api_name), enclosing);
    }

    // Second pass — attach the deserialised symbols. We lean on the
    // extractor's own JSON serialisation as the round-trip — if it
    // ever drifts from the deserialiser, this test is the one that
    // will trip.
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

/// Run the full pipeline on the given fixture set and return the
/// resolver edges alongside the hints (for cross-referencing ids).
async fn resolve_fixture(
    paths: &[PathBuf],
) -> (SyntaxResults, Vec<graphengine_parsing::domain::Edge>) {
    let hints = extract(paths).await;
    let registry = build_registry_from_results(&hints);
    let resolver = ApexHeuristicResolver::new(registry);
    let edges = resolver.resolve(&hints).await.expect("resolve").call_edges;
    (hints, edges)
}

/// Common assertion shape: §3.2 rows all say "1 edge, Medium". The
/// driver asserts the count AND the provenance so a drop to Low (or a
/// split into two edges) is flagged loudly instead of silently
/// passing a count-only check.
fn assert_single_medium_heuristic_edge(
    edges: &[graphengine_parsing::domain::Edge],
    scenario: &str,
) {
    assert_eq!(
        edges.len(),
        1,
        "[{scenario}] expected exactly 1 call edge; got {} edges: {:#?}",
        edges.len(),
        edges.iter().map(|e| &e.to_id).collect::<Vec<_>>(),
    );
    let edge = &edges[0];
    assert_eq!(
        edge.provenance.source,
        ProvenanceSource::Heuristic,
        "[{scenario}] edge must be heuristic-provenance",
    );
    assert_eq!(
        edge.provenance.confidence,
        Confidence::Medium,
        "[{scenario}] §3.2 requires Medium confidence (single unambiguous match)",
    );
}

// ---------------------------------------------------------------------------
// TR-A.1 intra-file — sibling-inner ctor fast path (§3.1 step 1).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a1_intra_file_inner_ctor_resolves_single_medium_edge() {
    let (hints, edges) = resolve_fixture(&fixture_paths("r23_a1_intra_file_inner_ctor")).await;
    assert_single_medium_heuristic_edge(&edges, "r23_a1_intra_file_inner_ctor");

    let edge = &edges[0];
    let target = hints
        .symbols
        .iter()
        .find(|n| n.id == edge.to_id)
        .expect("edge target must exist in symbols");
    assert!(
        target.fqn.contains("RD2_DataMigrationBase_BATCH.Logger"),
        "edge must land on the inner `Logger` ctor, not a top-level or other class; \
         got fqn={}",
        target.fqn,
    );
}

#[tokio::test]
async fn r23_a1_intra_file_sibling_class_resolves_single_medium_edge() {
    let (hints, edges) = resolve_fixture(&fixture_paths("r23_a1_intra_file_sibling_class")).await;
    assert_single_medium_heuristic_edge(&edges, "r23_a1_intra_file_sibling_class");

    let target = hints
        .symbols
        .iter()
        .find(|n| n.id == edges[0].to_id)
        .expect("target must exist");
    assert!(
        target.fqn.contains("CallableApi"),
        "edge must land on the sibling `CallableApi` class; got {}",
        target.fqn,
    );
}

#[tokio::test]
async fn r23_a1_ctor_util_jobprogress_resolves_single_medium_edge() {
    let (hints, edges) = resolve_fixture(&fixture_paths("r23_a1_ctor_util_jobprogress")).await;
    assert_single_medium_heuristic_edge(&edges, "r23_a1_ctor_util_jobprogress");

    let target = hints
        .symbols
        .iter()
        .find(|n| n.id == edges[0].to_id)
        .expect("target must exist");
    assert!(
        target.fqn.contains("BatchJob"),
        "edge must land on the inner `BatchJob` ctor; got {}",
        target.fqn,
    );
}

#[tokio::test]
async fn r23_a1_fflib_testsobject_resolves_single_medium_edge() {
    let (hints, edges) = resolve_fixture(&fixture_paths("r23_a1_fflib_testsobject")).await;
    assert_single_medium_heuristic_edge(&edges, "r23_a1_fflib_testsobject");

    let target = hints
        .symbols
        .iter()
        .find(|n| n.id == edges[0].to_id)
        .expect("target must exist");
    assert!(
        target.fqn.contains("TestSObjectDisableBehaviour"),
        "edge must land on the inner `TestSObjectDisableBehaviour` ctor; got {}",
        target.fqn,
    );
}

// ---------------------------------------------------------------------------
// TR-A.2 cross-file — registry-wide bindings.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a2_cross_file_ctor_resolves_single_medium_edge() {
    let (hints, edges) = resolve_fixture(&fixture_paths("r23_a2_cross_file_ctor")).await;
    assert_single_medium_heuristic_edge(&edges, "r23_a2_cross_file_ctor");

    let target = hints
        .symbols
        .iter()
        .find(|n| n.id == edges[0].to_id)
        .expect("target must exist");
    assert!(
        target.fqn.contains("HouseholdMembers"),
        "edge must land on the `HouseholdMembers` ctor; got {}",
        target.fqn,
    );
    // The caller must live in the sibling `HouseholdMembersCaller.cls`
    // — this proves the cross-file binding path, not a same-file
    // fallback. Pin by file-stem-only so the absolute path under
    // CARGO_MANIFEST_DIR doesn't leak into the assertion.
    let source = hints
        .symbols
        .iter()
        .find(|n| n.id == edges[0].from_id)
        .expect("source must exist");
    assert!(
        source.location.file.ends_with("HouseholdMembersCaller.cls"),
        "edge source must live in HouseholdMembersCaller.cls; got {}",
        source.location.file,
    );
}

#[tokio::test]
async fn r23_a2_cross_file_default_ctor_resolves_single_medium_edge_to_type_node() {
    let (hints, edges) = resolve_fixture(&fixture_paths("r23_a2_cross_file_default_ctor")).await;
    assert_single_medium_heuristic_edge(&edges, "r23_a2_cross_file_default_ctor");

    // Implicit default ctor has no `Function` node — the edge lands on
    // the class's `Struct` node per `constructor_resolver.rs` §
    // default-ctor arm. Verify the edge target is a Struct.
    let target = hints
        .symbols
        .iter()
        .find(|n| n.id == edges[0].to_id)
        .expect("target must exist");
    assert!(
        matches!(target.kind, NodeKind::Struct),
        "implicit default-ctor edge must target the class `Struct` node (no \
         phantom Function synthesised); got kind={:?}",
        target.kind,
    );
    assert!(
        target.fqn.contains("RD_InstallScript_BATCH"),
        "edge must land on `RD_InstallScript_BATCH` Struct; got {}",
        target.fqn,
    );
}

#[tokio::test]
async fn r23_a2_cross_file_overloaded_ctor_picks_integer_overload() {
    let (hints, edges) = resolve_fixture(&fixture_paths("r23_a2_cross_file_overloaded_ctor")).await;
    assert_single_medium_heuristic_edge(&edges, "r23_a2_cross_file_overloaded_ctor");

    // The discriminator is an integer literal `42`. The matcher must
    // pick the `GiftBatch(Integer)` overload, not `GiftBatch(String)`.
    let target = hints
        .symbols
        .iter()
        .find(|n| n.id == edges[0].to_id)
        .expect("target must exist");
    assert!(
        target.fqn.to_ascii_lowercase().contains("integer"),
        "overload-ctor edge must bind to the `GiftBatch(Integer)` overload \
         (literal `42` discriminator); bound to {} instead",
        target.fqn,
    );
    assert!(
        !target.fqn.to_ascii_lowercase().contains("string"),
        "overload-ctor edge must NOT bind to the `GiftBatch(String)` overload; \
         bound to {}",
        target.fqn,
    );
}
