//! TR-A.5 acceptance driver — Visualforce `controller` / `extensions`
//! binding fixtures.
//!
//! Walks the three fixture scenarios under
//! `graphengine-parsing/tests/fixtures/apex_resolver/r23_a5_vf/` through
//! the real tree-sitter Apex extractor and the VF extraction pipeline
//! stage, and asserts each scenario synthesises the expected
//! `__vf_page__` container + body + Contains edge and emits the right
//! `CallSite` (Class::method) for every resolvable `{!...}` binding.
//!
//! # Why these three
//!
//! `PHASE_A_EXECUTION_PLAN.md` §7.3 lists exactly these fixture shapes:
//!
//! * `util_jobprogress` — §4.11 acceptance exemplar
//!   (`extensions="UTIL_JobProgress_CTRL"` in prose / `controller=` in
//!   the authoritative VF source; `{!refreshJobs}` must bind).
//! * `minimal_action` — bare `<apex:commandButton action="{!save}">`
//!   with controller only, no extensions.
//! * `multi_extension` — `extensions="ExtA, ExtB"`, `{!method}` declared
//!   on ExtB only → resolver must fall through past ExtA.
//!
//! # Scope
//!
//! This test exercises the VF stage in isolation (not the full
//! `ParsingPipeline`) because:
//!
//! * The full pipeline requires an async runtime + persistence layer
//!   that would dwarf the signal-to-noise of the VF-specific assertions
//!   this driver cares about.
//! * The VF stage's integration contract with the pipeline is just
//!   `(root, &mut SyntaxResults)` — exercising it directly still
//!   covers the one boundary the stage owns.
//!
//! The downstream semantic-resolution hop from CallSite → Call edge is
//! covered by the broader `apex_resolver_*` integration tests; TR-A.5
//! only owns the emission of the CallSite rows.

use graphengine_parsing::application::ports::{SyntaxExtractor, SyntaxResults};
use graphengine_parsing::domain::{EdgeKind, FrameworkKind, NodeKind};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::language::apex::vf_extraction_stage as vf_extraction;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::{Path, PathBuf};

fn scenario_dir(scenario: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("apex_resolver")
        .join("r23_a5_vf")
        .join(scenario)
}

fn cls_paths_in(scenario_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(scenario_dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", scenario_dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("cls"))
        .collect();
    out.sort();
    out
}

/// Drive the tree-sitter Apex extractor over the scenario's `.cls`
/// files so `SyntaxResults::class_symbols` is populated, then invoke
/// the VF extraction stage anchored at the scenario dir. Returns the
/// post-stage results so each test can assert against them.
async fn run_vf_stage(scenario: &str) -> SyntaxResults {
    let dir = scenario_dir(scenario);
    let cls_paths = cls_paths_in(&dir);
    assert!(
        !cls_paths.is_empty(),
        "scenario `{scenario}` has no .cls fixtures",
    );

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    let mut results = extractor.extract(&cls_paths).await.expect("parse apex");
    // The VF stage expects a workspace_root so synthetic FQNs carry
    // the project-relative path prefix. Use the scenario dir — the
    // FQN suffix is what assertions match on, not the prefix.
    results.set_workspace_root(dir.to_string_lossy().to_string());

    let stats = vf_extraction::run(&dir, &mut results).expect("run VF stage");
    assert!(
        stats.pages_parsed >= 1,
        "[{scenario}] expected >=1 page parsed; got {stats:?}",
    );
    assert_eq!(
        stats.pages_failed, 0,
        "[{scenario}] unexpected page parse failures: {stats:?}",
    );
    results
}

fn find_synthetic_vf_container<'a>(
    results: &'a SyntaxResults,
    page_name: &str,
) -> &'a graphengine_parsing::domain::Node {
    results
        .symbols
        .iter()
        .find(|n| {
            matches!(n.kind, NodeKind::Struct)
                && n.fqn.ends_with(&format!("::{page_name}"))
                && n.properties.get("synthetic_kind").and_then(|v| v.as_str())
                    == Some("apex_vf_page_container")
        })
        .unwrap_or_else(|| {
            panic!(
                "no synthetic VF container Struct for page `{page_name}` in: {:#?}",
                results
                    .symbols
                    .iter()
                    .map(|n| (&n.kind, &n.fqn))
                    .collect::<Vec<_>>()
            )
        })
}

fn find_synthetic_vf_body<'a>(
    results: &'a SyntaxResults,
    page_name: &str,
) -> &'a graphengine_parsing::domain::Node {
    results
        .symbols
        .iter()
        .find(|n| {
            matches!(n.kind, NodeKind::Function)
                && n.fqn.ends_with(&format!("::{page_name}::__vf_page__()"))
        })
        .unwrap_or_else(|| {
            panic!(
                "no synthetic __vf_page__ Function for page `{page_name}` in: {:#?}",
                results
                    .symbols
                    .iter()
                    .map(|n| (&n.kind, &n.fqn))
                    .collect::<Vec<_>>()
            )
        })
}

fn contains_edge_between(results: &SyntaxResults, from_id: &str, to_id: &str) -> bool {
    results.synthesized_edges.iter().any(|e| {
        matches!(e.kind, graphengine_parsing::domain::EdgeKind::Contains)
            && e.from_id == from_id
            && e.to_id == to_id
    })
}

/// Collect every `UnresolvedReference::FrameworkBinding` whose
/// call-site function name matches `name`. Post-P1.d the VF stage
/// emits framework bindings, not bare call sites with an `Option<hint>`.
fn framework_bindings_named<'a>(
    results: &'a SyntaxResults,
    name: &str,
) -> Vec<&'a graphengine_parsing::application::ports::FrameworkBinding> {
    results
        .iter_framework_bindings()
        .filter(|fb| fb.call_site.function_name == name)
        .collect()
}

/// Assert that every VF-emitted reference is a
/// `FrameworkBinding { framework: VisualforcePage, .. }`. The P1.d
/// rework moved this invariant from a runtime hint on `CallSite`
/// (`Option<EdgeKind>` the resolver was free to ignore) to a typed
/// variant on `UnresolvedReference` — a resolver that drops the
/// framework context now fails to compile, not silently regress.
fn assert_all_have_visualforce_binding(
    bindings: &[&graphengine_parsing::application::ports::FrameworkBinding],
    scenario: &str,
) {
    for fb in bindings {
        assert_eq!(
            fb.framework,
            FrameworkKind::VisualforcePage,
            "[{scenario}] FrameworkBinding `{}` is not VisualforcePage (got {:?}); \
             downstream edge would be mis-emitted as the wrong Framework(_) variant",
            fb.call_site.function_name,
            fb.framework,
        );
        // Secondary invariant: the derived edge kind must round-trip
        // to `Framework(VisualforcePage)`. Guards against a future
        // change that decouples `FrameworkKind` from
        // `UnresolvedReference::edge_kind()`.
        let reference =
            graphengine_parsing::application::ports::UnresolvedReference::FrameworkBinding(
                (*fb).clone(),
            );
        assert_eq!(
            reference.edge_kind(),
            EdgeKind::Framework(FrameworkKind::VisualforcePage),
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario 1 — `UTIL_JobProgress.page`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a5_util_jobprogress_binds_refresh_jobs() {
    let scenario = "util_jobprogress";
    let results = run_vf_stage(scenario).await;

    let container = find_synthetic_vf_container(&results, "UTIL_JobProgress");
    let body = find_synthetic_vf_body(&results, "UTIL_JobProgress");

    assert!(
        contains_edge_between(&results, &container.id, &body.id),
        "[{scenario}] Contains edge from container to __vf_page__ missing",
    );

    // `{!refreshJobs}` appears twice in the page (action="..." on both
    // <apex:page> and the commandButton). Each occurrence is an
    // independent attribute site and must produce its own CallSite.
    let refresh_sites = framework_bindings_named(&results, "UTIL_JobProgress_CTRL::refreshJobs");
    assert_eq!(
        refresh_sites.len(),
        2,
        "[{scenario}] expected 2 CallSites for UTIL_JobProgress_CTRL::refreshJobs; got {}: {:?}",
        refresh_sites.len(),
        results
            .iter_all_call_sites()
            .map(|cs| &cs.function_name)
            .collect::<Vec<_>>(),
    );
    assert_all_have_visualforce_binding(&refresh_sites, scenario);

    // `{!hasActiveJobs}` and `{!jobSummary}` are property accesses
    // (not methods). They must NOT emit CallSites — the VF stage
    // drops bindings whose identifier doesn't match any method.
    let property_sites: Vec<&str> = results
        .iter_all_call_sites()
        .map(|cs| cs.function_name.as_str())
        .filter(|n| n.ends_with("::hasActiveJobs") || n.ends_with("::jobSummary"))
        .collect();
    assert!(
        property_sites.is_empty(),
        "[{scenario}] property accessors must not become CallSites; leaked: {property_sites:?}",
    );
}

// ---------------------------------------------------------------------------
// Scenario 2 — `minimal_action.page`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a5_minimal_action_binds_save() {
    let scenario = "minimal_action";
    let results = run_vf_stage(scenario).await;

    let container = find_synthetic_vf_container(&results, "minimal_action");
    let body = find_synthetic_vf_body(&results, "minimal_action");
    assert!(
        contains_edge_between(&results, &container.id, &body.id),
        "[{scenario}] Contains edge missing",
    );

    let sites = framework_bindings_named(&results, "minimal_action::save");
    assert_eq!(
        sites.len(),
        1,
        "[{scenario}] expected 1 CallSite for minimal_action::save; got {}: {:?}",
        sites.len(),
        results
            .iter_all_call_sites()
            .map(|cs| &cs.function_name)
            .collect::<Vec<_>>(),
    );
    assert_all_have_visualforce_binding(&sites, scenario);
}

// ---------------------------------------------------------------------------
// Scenario 3 — `multi_extension.page` (declared-order fall-through)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn r23_a5_multi_extension_falls_through_to_ext_b() {
    let scenario = "multi_extension";
    let results = run_vf_stage(scenario).await;

    let _container = find_synthetic_vf_container(&results, "multi_extension");
    let _body = find_synthetic_vf_body(&results, "multi_extension");

    // ExtB owns computeFoo — resolver must pick that one, NOT ExtA.
    let ext_b_sites = framework_bindings_named(&results, "ExtB::computeFoo");
    assert_eq!(
        ext_b_sites.len(),
        1,
        "[{scenario}] expected 1 CallSite for ExtB::computeFoo; got {}: {:?}",
        ext_b_sites.len(),
        results
            .iter_all_call_sites()
            .map(|cs| &cs.function_name)
            .collect::<Vec<_>>(),
    );
    assert_all_have_visualforce_binding(&ext_b_sites, scenario);

    // Guard: no spurious ExtA binding from the same call site.
    let ext_a_sites = framework_bindings_named(&results, "ExtA::computeFoo");
    assert!(
        ext_a_sites.is_empty(),
        "[{scenario}] declared-order fall-through violated: resolver emitted ExtA::computeFoo",
    );
}
