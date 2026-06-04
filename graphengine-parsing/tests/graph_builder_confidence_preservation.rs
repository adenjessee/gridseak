//! Regression test for the B.7-P1 confidence clobber.
//!
//! Before this test existed, `GraphBuilder::build_from_results` silently
//! overwrote every node's `provenance.confidence` to `Medium` and every
//! edge's to `High`, erasing the deliberate tiering that extractors and
//! resolvers carefully set (e.g. `ApexHeuristicResolver` emits `Low` for
//! ambiguous short-name call matches and `Medium` for unique matches).
//!
//! The clobber was masking quality signal that downstream findings
//! (`ResolutionDegraded`, managed-package coupling) rely on. This test
//! pins the post-fix contract: **every confidence tier that goes into
//! `build_from_results` comes out unchanged** on the corresponding node
//! or edge in the final graph.

use graphengine_parsing::application::ports::{ResolvedEdges, SyntaxResults};
use graphengine_parsing::application::use_cases::parse_repo::pipeline::graph_building::GraphBuilder;
use graphengine_parsing::domain::{
    Confidence, Edge, EdgeKind, Node, NodeKind, Provenance, ProvenanceSource, Range,
};

fn range(file: &str, sl: u32, el: u32) -> Range {
    Range::with_file(sl, 0, el, 10, file.to_string())
}

fn function_node(fqn: &str, file: &str, sl: u32, el: u32, conf: Confidence) -> Node {
    Node::new(
        NodeKind::Function,
        fqn.to_string(),
        range(file, sl, el),
        Provenance::new(ProvenanceSource::TreeSitter, conf),
    )
}

#[test]
fn confidence_is_preserved_across_all_tiers_for_nodes() {
    let mut syntax = SyntaxResults::new();

    let high_node = function_node("crate::high", "a.rs", 1, 10, Confidence::High);
    let medium_node = function_node("crate::medium", "b.rs", 1, 10, Confidence::Medium);
    let low_node = function_node("crate::low", "c.rs", 1, 10, Confidence::Low);

    let high_id = high_node.id.clone();
    let medium_id = medium_node.id.clone();
    let low_id = low_node.id.clone();

    syntax.add_symbol(high_node);
    syntax.add_symbol(medium_node);
    syntax.add_symbol(low_node);

    let graph = GraphBuilder::build_from_results(syntax, ResolvedEdges::new(), Confidence::Low)
        .expect("graph builds cleanly with mixed-confidence nodes");

    let find = |id: &str| {
        graph
            .nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("node {id} missing from graph"))
    };

    assert_eq!(
        find(&high_id).provenance.confidence,
        Confidence::High,
        "High-confidence node must keep its tier"
    );
    assert_eq!(
        find(&medium_id).provenance.confidence,
        Confidence::Medium,
        "Medium-confidence node must keep its tier (the prior clobber would have made it Medium anyway; this locks in that Medium is the extractor's choice, not the pipeline's)"
    );
    assert_eq!(
        find(&low_id).provenance.confidence,
        Confidence::Low,
        "Low-confidence node must keep its tier \u{2014} this is the regression the clobber was erasing"
    );
}

#[test]
fn confidence_is_preserved_across_all_tiers_for_edges() {
    let mut syntax = SyntaxResults::new();

    let caller = function_node("crate::caller", "caller.rs", 1, 50, Confidence::Medium);
    let high_callee = function_node("crate::high_callee", "h.rs", 1, 10, Confidence::Medium);
    let medium_callee = function_node("crate::medium_callee", "m.rs", 1, 10, Confidence::Medium);
    let low_callee = function_node("crate::low_callee", "l.rs", 1, 10, Confidence::Medium);

    let caller_id = caller.id.clone();
    let high_callee_id = high_callee.id.clone();
    let medium_callee_id = medium_callee.id.clone();
    let low_callee_id = low_callee.id.clone();

    syntax.add_symbol(caller);
    syntax.add_symbol(high_callee);
    syntax.add_symbol(medium_callee);
    syntax.add_symbol(low_callee);

    let mut edges = ResolvedEdges::new();
    edges.add_call_edge(Edge::call(
        caller_id.clone(),
        high_callee_id.clone(),
        Provenance::new(ProvenanceSource::Lsp, Confidence::High),
    ));
    edges.add_call_edge(Edge::call(
        caller_id.clone(),
        medium_callee_id.clone(),
        Provenance::new(ProvenanceSource::Heuristic, Confidence::Medium),
    ));
    edges.add_call_edge(Edge::call(
        caller_id.clone(),
        low_callee_id.clone(),
        Provenance::new(ProvenanceSource::Heuristic, Confidence::Low),
    ));

    let graph = GraphBuilder::build_from_results(syntax, edges, Confidence::Low)
        .expect("graph builds with Low-tier edges \u{2014} pipeline must not reject them");

    let find_edge = |to: &str| {
        graph
            .edges
            .iter()
            .find(|e| e.from_id == caller_id && e.to_id == to && e.kind == EdgeKind::Call)
            .unwrap_or_else(|| panic!("call edge to {to} missing from graph"))
    };

    let e_high = find_edge(&high_callee_id);
    assert_eq!(e_high.provenance.source, ProvenanceSource::Lsp);
    assert_eq!(e_high.provenance.confidence, Confidence::High);

    let e_medium = find_edge(&medium_callee_id);
    assert_eq!(e_medium.provenance.source, ProvenanceSource::Heuristic);
    assert_eq!(
        e_medium.provenance.confidence,
        Confidence::Medium,
        "Heuristic/Medium edge must reach the graph as Medium, not be promoted to High"
    );

    let e_low = find_edge(&low_callee_id);
    assert_eq!(e_low.provenance.source, ProvenanceSource::Heuristic);
    assert_eq!(
        e_low.provenance.confidence,
        Confidence::Low,
        "Heuristic/Low edge must reach the graph as Low \u{2014} this is the core B.7-P1 regression"
    );
}

#[test]
fn low_confidence_edges_do_not_fail_the_pipeline() {
    // The previous clobber behaviour stopped this from failing only because
    // it rewrote edges to High before validation. After the fix, the pipeline
    // still must not fail \u2014 heuristic runs legitimately produce Low edges.
    let mut syntax = SyntaxResults::new();
    let from = function_node("crate::caller", "a.rs", 1, 50, Confidence::Medium);
    let to = function_node("crate::callee", "b.rs", 1, 10, Confidence::Medium);
    let from_id = from.id.clone();
    let to_id = to.id.clone();
    syntax.add_symbol(from);
    syntax.add_symbol(to);

    let mut edges = ResolvedEdges::new();
    edges.add_call_edge(Edge::call(
        from_id,
        to_id,
        Provenance::new(ProvenanceSource::Heuristic, Confidence::Low),
    ));

    // Even if the caller passes a strict-looking min_confidence, the
    // pipeline must still succeed; min_confidence is advisory. Strict
    // callers can use `count_below_confidence` post-hoc.
    let graph = GraphBuilder::build_from_results(syntax, edges, Confidence::High)
        .expect("pipeline must not fail on legitimately Low-confidence heuristic edges");

    let below_high = GraphBuilder::count_below_confidence(&graph, Confidence::High);
    assert_eq!(
        below_high, 1,
        "count_below_confidence must surface the one Low-tier edge \
         so callers can make advisory decisions without breaking the parse"
    );
}
