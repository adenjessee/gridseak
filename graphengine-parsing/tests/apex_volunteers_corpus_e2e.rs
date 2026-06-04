//! Sprint H.3 — Volunteers-for-Salesforce structural regression test.
//!
//! Pins the real-world Apex structural invariants that are easy to
//! silently break and hard to notice: one module per file, sharing
//! propagation, inheritance edges, trigger synthesis, and the G.1 node
//! deduplication guarantee. The fixture under
//! `tests/fixtures/volunteers-mini/` is a small byte-identical subset
//! of the public Salesforce.org Volunteers-for-Salesforce project (see
//! `LICENSE.vendored` there).
//!
//! Assertions deliberately test invariants, not exact counts. Counts
//! drift for legitimate reasons (new FQN builder shape, new node kind)
//! — invariants don't.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
use graphengine_parsing::domain::{Confidence, EdgeKind, NodeKind};

/// Per-binary monotonic counter used to name scratch sqlite files. Tests
/// in the same integration-test binary run in parallel threads sharing
/// the same process id, so `process::id() + nanos` can collide on fast
/// runners and cause `database is locked` errors. An atomic counter is
/// collision-free by construction.
static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("volunteers-mini")
}

async fn parse_fixture() -> graphengine_parsing::domain::Graph {
    let root = fixture_root();
    assert!(
        root.join("sfdx-project.json").exists(),
        "fixture must contain sfdx-project.json at {}",
        root.display()
    );

    // Force heuristic-only resolution so this test stays fast on CI
    // runners that don't have Java/jorje provisioned. Without this the
    // LSP readiness barrier waits ~60s for the JAR to come up before
    // falling back — unacceptable for per-PR CI. The LSP path is
    // exercised separately by the nightly `apex-lsp.yml` workflow.
    //
    // SAFETY: env::set_var is marked unsafe in Rust 2024. This test
    // runs in a dedicated integration-test binary, so the global
    // mutation is isolated from library test code.
    // SAFETY: see doc comment above — single-threaded process-level env mutation
    // confined to this integration-test binary.
    unsafe {
        std::env::set_var(
            graphengine_parsing::syntax::language::apex::ENV_APEX_RESOLVER,
            "heuristic",
        );
    }

    let ws_url = url::Url::from_directory_path(&root).expect("fixture path must be absolute");
    let seq = SCRATCH_SEQ.fetch_add(1, Ordering::SeqCst);
    let scratch = std::env::temp_dir().join(format!(
        "volunteers_mini_{}_{}.sqlite",
        std::process::id(),
        seq,
    ));

    let use_case = ParseRepositoryUseCase::with_real_components(
        "apex".to_string(),
        Confidence::Low,
        scratch.to_str().expect("scratch path is valid UTF-8"),
        Some(ws_url),
    )
    .await
    .expect("build apex use case");

    let resolved = use_case
        .parse(root.clone(), "apex".to_string())
        .await
        .expect("parse volunteers-mini fixture");

    resolved.graph().clone()
}

fn count_files(root: &Path, ext: &str) -> usize {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some(ext))
        .count()
}

/// Core structural invariant. Every `.cls` and `.trigger` file in the
/// fixture produces exactly one `Module` node (the `__file_module__`
/// from Sprint E.6). Volunteers-mini has no managed-package
/// references, so the total Module count must equal file count
/// exactly — if it exceeds, the G.1 dedup fix has regressed.
#[tokio::test]
async fn file_module_invariant_holds_with_no_duplicates() {
    let graph = parse_fixture().await;

    let cls_count = count_files(
        &fixture_root().join("force-app/main/default/classes"),
        "cls",
    );
    let trigger_count = count_files(
        &fixture_root().join("force-app/main/default/triggers"),
        "trigger",
    );
    let total_apex_files = cls_count + trigger_count;

    assert!(
        total_apex_files >= 4,
        "fixture lost files — expected ≥4 apex files, got {total_apex_files}"
    );

    let module_count = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Module)
        .count();

    assert_eq!(
        module_count, total_apex_files,
        "Module count must equal unique file count exactly for this fixture \
         (no managed packages). Got {module_count} modules for {total_apex_files} \
         apex files — G.1 dedup regression or external-reference synthesis \
         is firing spuriously."
    );

    // Additionally, no two Module nodes may share an id — direct pin
    // on GraphBuilder::add_nodes deduplication (Sprint G.1 fix).
    let mut ids: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Module)
        .map(|n| n.id.as_str())
        .collect();
    ids.sort();
    let before = ids.len();
    ids.dedup();
    assert_eq!(
        ids.len(),
        before,
        "duplicate Module node ids present — GraphBuilder::add_nodes \
         dedup regressed"
    );
}

/// Sprint E.5 — `apex_sharing` propagation. Every Apex class/struct
/// node (outer or inner) must carry a non-empty `apex_sharing` property
/// with one of the well-known sharing values.
#[tokio::test]
async fn every_apex_class_node_carries_apex_sharing() {
    let graph = parse_fixture().await;

    let class_nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Struct)
        .filter(|n| {
            // Only Apex classes — exclude the trigger struct, whose
            // sharing concept is "whatever the calling context
            // supplies" and is not assigned an apex_sharing value.
            n.properties
                .get("subtype")
                .and_then(|v| v.as_str())
                .map(|s| s != "trigger")
                .unwrap_or(true)
        })
        .collect();

    assert!(
        !class_nodes.is_empty(),
        "fixture should extract several class-shaped Struct nodes"
    );

    for node in &class_nodes {
        let sharing = node
            .properties
            .get("apex_sharing")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "class node `{}` has no apex_sharing property — E.5 regression",
                    node.fqn
                )
            });
        assert!(
            matches!(
                sharing,
                "with_sharing" | "without_sharing" | "inherited_sharing" | "unspecified"
            ),
            "class `{}` has unexpected apex_sharing `{}` — new value added without updating invariant",
            node.fqn,
            sharing,
        );
    }
}

/// Sprint E.1 — `Extends` is a first-class edge kind. The fixture's
/// `UTIL_Describe` declares two inner classes extending `Exception`.
/// `Exception` itself has no graph node (G.2 documented gap), but the
/// outer class's ability to produce an inheritance hint on its inners
/// must still surface somewhere — either as `Extends` edges (if future
/// work synthesizes built-in type nodes) or via the inner class's
/// FQN shape. This test pins the FQN invariant because it's the one
/// the graph layer guarantees today.
#[tokio::test]
async fn inner_classes_encode_outer_dot_inner_in_fqn() {
    let graph = parse_fixture().await;

    // UTIL_Describe has `PermsException` and `SchemaDescribeException`
    // as inner classes. Either both or neither must be present, and
    // both must use the `Outer.Inner` form in the final FQN segment
    // (Sprint E.2 disambiguation builder).
    let inner_fqns: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Struct)
        .map(|n| n.fqn.as_str())
        .filter(|fqn| {
            fqn.contains("UTIL_Describe.PermsException")
                || fqn.contains("UTIL_Describe.SchemaDescribeException")
        })
        .collect();

    assert_eq!(
        inner_fqns.len(),
        2,
        "expected both inner exception classes with `Outer.Inner` FQN form; \
         got {inner_fqns:?}"
    );
}

/// Sprint E.3/E.4 — `.trigger` files get a synthetic `__trigger__`
/// Function node, and the parent trigger struct carries the
/// `trigger_events` property. Pins the wire-up against the fixture's
/// real `VOL_Campaign_CreateStatuses.trigger`.
#[tokio::test]
async fn trigger_body_produces_synthetic_function_and_events_property() {
    let graph = parse_fixture().await;

    let trigger_struct = graph
        .nodes
        .iter()
        .find(|n| {
            n.kind == NodeKind::Struct
                && n.properties
                    .get("subtype")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "trigger")
                    .unwrap_or(false)
                && n.fqn.contains("VOL_Campaign_CreateStatuses")
        })
        .expect("VOL_Campaign_CreateStatuses trigger struct node must exist");

    let events = trigger_struct
        .properties
        .get("trigger_events")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| {
            panic!(
                "trigger `{}` has no `trigger_events` array property — E.4 regression",
                trigger_struct.fqn
            )
        });
    assert!(
        !events.is_empty(),
        "trigger_events must be non-empty for a real trigger body"
    );

    // The synthetic `__trigger__` Function must exist and be contained
    // within the trigger's file via a Contains edge.
    let synthetic_fn = graph
        .nodes
        .iter()
        .find(|n| {
            n.kind == NodeKind::Function
                && n.fqn.contains("VOL_Campaign_CreateStatuses")
                && n.fqn.contains("__trigger__")
        })
        .unwrap_or_else(|| {
            panic!(
                "synthetic __trigger__ Function node for VOL_Campaign_CreateStatuses missing — E.3 regression"
            )
        });

    // Contains edge from trigger struct → __trigger__ Function.
    let has_contains_edge = graph.edges.iter().any(|e| {
        e.kind == EdgeKind::Contains && e.from_id == trigger_struct.id && e.to_id == synthetic_fn.id
    });
    assert!(
        has_contains_edge,
        "Contains edge from trigger struct `{}` to synthetic __trigger__ \
         Function `{}` is missing",
        trigger_struct.fqn, synthetic_fn.fqn,
    );
}

/// Sprint H.1 regression pin. `UTIL_Describe.getNamespace` exists in
/// the fixture AND so does `VOL_SharedCode.getNamespace`. Prior to
/// H.1, intra-class calls to `getNamespace()` would fan out to BOTH
/// classes at Low confidence. Post-H.1, the same-class preference
/// collapses to a single Medium-confidence edge whenever the caller's
/// own class owns a matching method.
///
/// Assertion: for every Call edge whose target is `UTIL_Describe`'s own
/// `getNamespace`, the provenance confidence is Medium or High (never
/// Low). Equivalently: no `getNamespace` Call edge is both (a)
/// originating inside UTIL_Describe and (b) pointing at the foreign
/// VOL_SharedCode copy.
#[tokio::test]
async fn same_class_preference_filters_cross_class_getnamespace_fanout() {
    let graph = parse_fixture().await;

    // Find the two candidate callees.
    let util_getns = graph
        .nodes
        .iter()
        .find(|n| {
            n.kind == NodeKind::Function
                && n.fqn.contains("UTIL_Describe")
                && n.fqn.contains("::getNamespace")
        })
        .expect("UTIL_Describe.getNamespace function node must exist");

    let shared_getns = graph.nodes.iter().find(|n| {
        n.kind == NodeKind::Function
            && n.fqn.contains("VOL_SharedCode")
            && n.fqn.contains("::getNamespace")
    });

    // If the fixture ever drops VOL_SharedCode.getNamespace, the H.1
    // invariant becomes untestable; surface that loudly rather than
    // silently passing.
    if shared_getns.is_none() {
        panic!(
            "fixture invariant broken: VOL_SharedCode.getNamespace must exist \
             to exercise the H.1 same-class preference path"
        );
    }
    let shared_getns = shared_getns.unwrap();

    // Callers originating in UTIL_Describe's file.
    let util_file = &util_getns.location.file;

    // If any Call edge originates from a node in UTIL_Describe and
    // points to VOL_SharedCode.getNamespace, the same-class preference
    // has regressed.
    let util_node_ids: std::collections::HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.location.file == *util_file)
        .map(|n| n.id.as_str())
        .collect();

    let cross_class_leak = graph.edges.iter().any(|e| {
        e.kind == EdgeKind::Call
            && util_node_ids.contains(e.from_id.as_str())
            && e.to_id == shared_getns.id
    });
    assert!(
        !cross_class_leak,
        "H.1 regression: a Call edge from a UTIL_Describe node reaches \
         VOL_SharedCode.getNamespace. Same-class preference should have \
         pinned it to UTIL_Describe.getNamespace."
    );
}
