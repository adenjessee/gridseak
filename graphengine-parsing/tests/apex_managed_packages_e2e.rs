//! End-to-end regression for B.7-P0: managed-package synthesis must
//! reach the final graph.
//!
//! Before this test existed, `managed_packages::extract` was fully
//! implemented and unit-tested but **never called** by any production
//! code path. The NPSP baseline surfaced the bug — every real scan
//! reported `managed_package_consumers: {}` despite consumer files
//! referencing `npsp.`, `npe01__`, `npo02__` etc. across hundreds of
//! classes.
//!
//! This suite pins the fix: running the real `TreeSitterExtractor`
//! against the committed Apex corpus must produce
//!
//! - one virtual `Module` node per distinct namespace discovered in the
//!   corpus (`npsp`, `pi`, …), all anchored to the synthetic external
//!   file sentinel so they never collide with real in-repo modules;
//! - one `Import` edge from the consumer file-module node to each
//!   external namespace node, with `ProvenanceSource::Heuristic` and
//!   `Confidence::High` preserved end-to-end (the B.7-P1 clobber fix
//!   is what lets us assert exact confidence here).
//!
//! If either assertion fails the scan is silently under-reporting
//! external coupling, which breaks the whole "managed package blast
//! radius" demo.

use graphengine_parsing::application::ports::ResolvedEdges;
use graphengine_parsing::application::ports::SyntaxExtractor;
use graphengine_parsing::application::use_cases::parse_repo::pipeline::graph_building::GraphBuilder;
use graphengine_parsing::domain::{Confidence, EdgeKind, NodeKind, ProvenanceSource};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::language::apex::{
    VIRTUAL_MANAGED_MODULE_FILE_SENTINEL, VIRTUAL_MANAGED_MODULE_FQN_PREFIX,
};
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::{Path, PathBuf};

fn corpus_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("apex")
        .join("force-app")
        .join("main")
        .join("default")
        .join("classes");
    std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", root.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("cls"))
        .collect()
}

#[tokio::test]
async fn managed_package_nodes_and_edges_materialize_in_syntax_results() {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    let hints = extractor
        .extract(&corpus_files())
        .await
        .expect("extract corpus");

    // Every discovered namespace must surface as a virtual Module node
    // anchored on the sentinel file path. At minimum `npsp` (from the
    // SOQL query) and `pi` (from the dotted-type fixture) must appear.
    let virtual_modules: Vec<&graphengine_parsing::domain::Node> = hints
        .symbols
        .iter()
        .filter(|n| n.kind == NodeKind::Module)
        .filter(|n| n.fqn.starts_with(VIRTUAL_MANAGED_MODULE_FQN_PREFIX))
        .collect();

    assert!(
        !virtual_modules.is_empty(),
        "no virtual managed-package Module nodes were synthesised from the corpus"
    );

    for ns in ["npsp", "pi"] {
        let found = virtual_modules
            .iter()
            .any(|n| n.fqn.ends_with(&format!("::{ns}")));
        assert!(
            found,
            "expected a virtual Module for namespace `{ns}` in corpus; \
             got {:?}",
            virtual_modules.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );
    }

    for m in &virtual_modules {
        assert_eq!(
            m.location.file, VIRTUAL_MANAGED_MODULE_FILE_SENTINEL,
            "virtual managed-package module must be anchored on the sentinel file path"
        );
        assert_eq!(
            m.provenance.source,
            ProvenanceSource::Heuristic,
            "virtual managed-package Module node must be tagged Heuristic provenance"
        );
    }

    // Each consumer file must emit at least one Import edge into one of
    // those namespace modules.
    assert!(
        !hints.synthesized_edges.is_empty(),
        "no synthesized Import edges reached SyntaxResults \u{2014} the P0 \
         wire-up is broken somewhere between `synthesize_external_references` \
         and `TreeSitterExtractor::parse_file`"
    );

    let import_edges: Vec<_> = hints
        .synthesized_edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Import)
        .collect();
    assert!(
        !import_edges.is_empty(),
        "expected at least one Import edge among synthesized edges; got {:?}",
        hints
            .synthesized_edges
            .iter()
            .map(|e| (e.kind, &e.from_id, &e.to_id))
            .collect::<Vec<_>>()
    );

    for edge in &import_edges {
        assert_eq!(
            edge.provenance.source,
            ProvenanceSource::Heuristic,
            "synthesized managed-package import edges are Heuristic-sourced"
        );
        assert_eq!(
            edge.provenance.confidence,
            Confidence::High,
            "synthesized managed-package import edges are High-confidence \
             (namespace detection is deterministic)"
        );
    }
}

#[tokio::test]
async fn managed_package_import_edges_survive_graph_building() {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    let hints = extractor
        .extract(&corpus_files())
        .await
        .expect("extract corpus");

    let graph = GraphBuilder::build_from_results(hints, ResolvedEdges::new(), Confidence::Low)
        .expect("build graph from corpus");

    let virtual_module_ids: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| {
            n.kind == NodeKind::Module && n.fqn.starts_with(VIRTUAL_MANAGED_MODULE_FQN_PREFIX)
        })
        .map(|n| n.id.as_str())
        .collect();

    assert!(
        !virtual_module_ids.is_empty(),
        "virtual managed-package Module nodes were dropped during graph building"
    );

    let import_hits = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Import)
        .filter(|e| virtual_module_ids.contains(&e.to_id.as_str()))
        .count();

    assert!(
        import_hits >= virtual_module_ids.len(),
        "expected \u{2265} {} Import edges into virtual managed-package modules, \
         got {import_hits} \u{2014} graph assembly is dropping synthesized edges",
        virtual_module_ids.len(),
    );
}

/// Sprint H.2 — curated managed-package registry wiring.
///
/// Asserts end-to-end that:
/// 1. The known ecosystem namespace `npsp` (present in the existing
///    fixture corpus) surfaces through `TreeSitterExtractor` with the
///    curated `display_name`, `vendor`, and `category` properties
///    populated AND `is_known_ecosystem_package = true`.
/// 2. The unknown-namespace branch stays a no-op for everything
///    currently in the corpus — i.e. we didn't accidentally hard-code
///    a fallback label.
#[tokio::test]
async fn known_ecosystem_namespace_module_nodes_carry_registry_metadata() {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    let hints = extractor
        .extract(&corpus_files())
        .await
        .expect("extract corpus");

    let npsp_node = hints
        .symbols
        .iter()
        .find(|n| {
            n.kind == NodeKind::Module
                && n.fqn.starts_with(VIRTUAL_MANAGED_MODULE_FQN_PREFIX)
                && n.fqn.ends_with("::npsp")
        })
        .expect("npsp virtual module must exist in corpus extraction output");

    assert_eq!(
        npsp_node.properties.get("is_known_ecosystem_package"),
        Some(&serde_json::json!(true)),
        "npsp is a curated ecosystem package and must be flagged as such"
    );
    assert_eq!(
        npsp_node.properties.get("display_name"),
        Some(&serde_json::json!("Nonprofit Success Pack")),
    );
    assert_eq!(
        npsp_node.properties.get("vendor"),
        Some(&serde_json::json!("salesforce_org")),
    );
    assert_eq!(
        npsp_node.properties.get("category"),
        Some(&serde_json::json!("nonprofit")),
    );
}

/// Sprint H.2 — unknown namespaces carry the flag but no labels, so
/// downstream consumers can tell "unknown package" apart from "absent
/// registry metadata". Uses `synthesize_managed_package_module_node`
/// directly — the corpus happens to only reference known namespaces.
#[test]
fn unknown_namespace_module_is_explicitly_marked_not_known() {
    use graphengine_parsing::syntax::language::apex::synthesize_managed_package_module_node;

    let node = synthesize_managed_package_module_node("some_random_appexchange_ns");
    assert_eq!(
        node.properties.get("is_known_ecosystem_package"),
        Some(&serde_json::json!(false)),
    );
    assert!(!node.properties.contains_key("display_name"));
    assert!(!node.properties.contains_key("vendor"));
    assert!(!node.properties.contains_key("category"));
    assert_eq!(
        node.properties.get("namespace"),
        Some(&serde_json::json!("some_random_appexchange_ns")),
        "unknown namespaces still expose the raw namespace string"
    );
}
