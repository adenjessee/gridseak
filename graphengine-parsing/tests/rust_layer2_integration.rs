//! End-to-end integration test for the Rust Layer 2 semantic
//! resolver wrapper (`RustLayer2SemanticResolver`).
//!
//! T6 Gate 1.2 acceptance: the wrapper must implement the engine's
//! `SemanticResolver` port against a real two-file Rust workspace,
//! produce a `Confidence::High` call edge for the unambiguous
//! `main -> callee` reference, **and** record the call-site location
//! in `ResolvedEdges::resolved_call_sites` so the heuristic fallback
//! does not emit a duplicate Low-confidence edge. This test drives
//! the same adapter path the production factory wires up, but with
//! hand-built `SyntaxResults` so the assertions are deterministic
//! and do not depend on tree-sitter.

#![cfg(feature = "rust-layer2")]

use std::path::PathBuf;

use graphengine_parsing::application::ports::{
    CallSite, SemanticResolver, SyntaxResults, UnresolvedReference,
};
use graphengine_parsing::domain::{Node, Range};
use graphengine_parsing::infrastructure::RustLayer2SemanticResolver;

fn two_file_fixture_root() -> PathBuf {
    // Reuse the adapter crate's fixture — keeps the acceptance test
    // aligned with the adapter-level regression fixture so a single
    // shape is the canonical known-good workspace.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("graphengine-ra-ide-adapter")
        .join("tests")
        .join("fixtures")
        .join("two_file_call")
}

#[tokio::test]
async fn rust_layer2_resolves_main_to_callee_and_marks_site() {
    let root = two_file_fixture_root();
    let resolver =
        RustLayer2SemanticResolver::new(&root).expect("two-file fixture should load for adapter");

    assert_eq!(resolver.supported_language(), "rust");
    assert!(
        resolver.is_available().await,
        "adapter constructed cleanly, is_available() must be true"
    );

    let main_rs = root.join("src").join("main.rs");
    let lib_rs = root.join("src").join("lib.rs");

    // Hand-rolled SyntaxResults matching the shape of what
    // `TreeSitterExtractor` would emit for this fixture. We only
    // need:
    //   - One caller symbol (`main::main`) covering the call site
    //   - One callee symbol (`fixture_lib::callee`) at lib.rs:6
    //   - One `UnresolvedReference::Call` pointing at line 4 col 4
    //     of main.rs (the `callee()` invocation).
    let main_file = main_rs.to_string_lossy().to_string();
    let lib_file = lib_rs.to_string_lossy().to_string();

    // `fn main() { ... }` spans lines 3-5 in the fixture; we use a
    // generous range that contains the call site on line 4.
    let main_range = Range::with_file(3, 0, 5, 1, main_file.clone());
    let caller = Node::function("fixture_main::main".to_string(), main_range);

    // `callee()` body lives at lines 6-8 of lib.rs.
    let callee_range = Range::with_file(6, 0, 8, 1, lib_file.clone());
    let callee = Node::function("fixture_lib::callee".to_string(), callee_range);

    // The call site itself: `    callee();` at line 4 col 4.
    let call_site_location = Range::with_file(4, 4, 4, 10, main_file.clone());
    let call_site = CallSite {
        location: call_site_location.clone(),
        function_name: "callee".to_string(),
        receiver_range: None,
        receiver_text: None,
        arg_types: Vec::new(),
    };

    let mut hints = SyntaxResults::new();
    hints.language = Some("rust".to_string());
    hints.workspace_root = Some(root.to_string_lossy().to_string());
    hints.symbols.push(caller.clone());
    hints.symbols.push(callee.clone());
    hints.references.push(UnresolvedReference::Call(call_site));

    let edges = resolver
        .resolve(&hints)
        .await
        .expect("resolve must not error on happy path");

    assert_eq!(
        edges.call_edges.len(),
        1,
        "expected exactly one Layer-2 call edge, got {}: {:?}",
        edges.call_edges.len(),
        edges.call_edges
    );
    let edge = &edges.call_edges[0];
    assert_eq!(
        edge.from_id, caller.id,
        "from_id must be the caller node id"
    );
    assert_eq!(edge.to_id, callee.id, "to_id must be the callee node id");
    assert_eq!(
        edge.kind,
        graphengine_parsing::domain::EdgeKind::Call,
        "Layer-2 resolver must emit Call-kind edges"
    );
    assert_eq!(
        edge.provenance.confidence,
        graphengine_parsing::domain::Confidence::High,
        "unambiguous target must be High confidence"
    );
    assert_eq!(
        edge.provenance.source,
        graphengine_parsing::domain::ProvenanceSource::Lsp,
        "Layer-2 edges must carry Lsp provenance so downstream observers can distinguish them from heuristic edges"
    );

    assert!(
        edges.resolved_call_sites.contains(&call_site_location),
        "Layer-2 must mark the resolved call site so the heuristic fallback can dedupe; \
         set contained {:?}",
        edges.resolved_call_sites
    );

    assert_eq!(
        edges.stats.lsp_edges, 1,
        "lsp_edges stat must count the Layer-2 emission"
    );

    let snap = resolver.snapshot();
    assert_eq!(snap.call_refs_seen, 1);
    assert_eq!(snap.high_resolutions, 1);
    assert_eq!(snap.edges_emitted, 1);
    assert_eq!(
        snap.adapter_errors, 0,
        "no adapter errors expected on the happy-path fixture"
    );
}

/// Gate 1.2 dedupe contract: when the Rust Layer 2 resolver has
/// already resolved a call site, `ResolvedEdges::resolved_call_sites`
/// must contain the exact `Range` the fallback dedupe path checks.
/// This test is the compile-time proof that the contract surface
/// between the two layers does not drift — if `CallSite::location`
/// and `resolved_call_sites` ever stop using the same `Range` shape,
/// this assertion catches the drift before a duplicate-edge bug ships.
#[tokio::test]
async fn rust_layer2_dedupe_contract_uses_matching_range_shape() {
    let root = two_file_fixture_root();
    let resolver = RustLayer2SemanticResolver::new(&root).expect("adapter must load");

    let main_rs = root.join("src").join("main.rs");
    let lib_rs = root.join("src").join("lib.rs");
    let main_file = main_rs.to_string_lossy().to_string();
    let lib_file = lib_rs.to_string_lossy().to_string();

    let caller_range = Range::with_file(3, 0, 5, 1, main_file.clone());
    let caller = Node::function("fixture_main::main".to_string(), caller_range);
    let callee_range = Range::with_file(6, 0, 8, 1, lib_file.clone());
    let callee = Node::function("fixture_lib::callee".to_string(), callee_range);

    let call_site_location = Range::with_file(4, 4, 4, 10, main_file.clone());
    let mut hints = SyntaxResults::new();
    hints.language = Some("rust".to_string());
    hints.symbols.push(caller);
    hints.symbols.push(callee);
    hints.references.push(UnresolvedReference::Call(CallSite {
        location: call_site_location.clone(),
        function_name: "callee".to_string(),
        receiver_range: None,
        receiver_text: None,
        arg_types: Vec::new(),
    }));

    let edges = resolver
        .resolve(&hints)
        .await
        .expect("resolve must succeed");
    // The dedupe contract key: a *byte-for-byte equal* `Range`
    // instance must be present in `resolved_call_sites`.
    assert!(
        edges.resolved_call_sites.contains(&call_site_location),
        "dedupe key `Range` shape has drifted — fallback will double-emit"
    );
}
