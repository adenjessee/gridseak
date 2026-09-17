//! Phase B witness fusion: heuristic corroboration on resolved call sites.

use graphengine_parsing::application::ports::{
    GlobalSymbolTable, ResolvedEdges, SymbolInfo, SyntaxResults,
};
use graphengine_parsing::application::use_cases::parse_repo::resolution::fallback::FallbackEdgeBuilder;
use graphengine_parsing::domain::{
    Confidence, Edge, EdgeKind, NodeKind, Provenance, ProvenanceSource, Range,
};

fn symbol(id: &str, name: &str, fqn: &str, file: &str, range: Range) -> SymbolInfo {
    SymbolInfo {
        id: id.to_string(),
        name: name.to_string(),
        fqn: fqn.to_string(),
        file: file.to_string(),
        range,
        kind: NodeKind::Function,
        trait_metadata: None,
    }
}

#[test]
fn agreement_stamps_corroboration_without_sibling_edge() {
    let caller_range = Range::with_file(1, 0, 20, 0, "main.rs".to_string());
    let call_loc = Range::with_file(5, 4, 5, 12, "main.rs".to_string());

    let mut syntax = SyntaxResults::new();
    syntax.set_language("rust".into());
    syntax.add_call_site(call_loc.clone(), "callee".into());

    let mut global = GlobalSymbolTable::new();
    global.add_symbol(symbol(
        "caller",
        "caller",
        "crate::caller",
        "main.rs",
        caller_range,
    ));
    global.add_symbol(symbol(
        "callee",
        "callee",
        "crate::callee",
        "lib.rs",
        Range::with_file(1, 0, 10, 0, "lib.rs".to_string()),
    ));

    let mut resolved = ResolvedEdges::new();
    resolved.mark_call_site_resolved(call_loc);
    resolved.add_call_edge(Edge::new(
        "caller".into(),
        "callee".into(),
        EdgeKind::Call,
        Provenance::new(ProvenanceSource::Compiler, Confidence::High),
    ));

    let out =
        FallbackEdgeBuilder::create_fallback_edges(&syntax, &global, resolved).expect("fallback");

    assert_eq!(
        out.call_edges.len(),
        1,
        "must not emit a sibling heuristic edge"
    );
    assert!(
        out.call_edges[0]
            .provenance
            .corroborating
            .contains(ProvenanceSource::Heuristic),
        "heuristic agreement must stamp corroboration"
    );
    assert_eq!(
        out.call_edges[0].provenance.source,
        ProvenanceSource::Compiler
    );
}

#[test]
fn disagreement_stamps_nothing_and_no_sibling() {
    let caller_range = Range::with_file(1, 0, 20, 0, "main.rs".to_string());
    let call_loc = Range::with_file(5, 4, 5, 12, "main.rs".to_string());

    let mut syntax = SyntaxResults::new();
    syntax.set_language("rust".into());
    syntax.add_call_site(call_loc.clone(), "other".into());

    let mut global = GlobalSymbolTable::new();
    global.add_symbol(symbol(
        "caller",
        "caller",
        "crate::caller",
        "main.rs",
        caller_range,
    ));
    global.add_symbol(symbol(
        "semantic",
        "semantic_target",
        "crate::semantic_target",
        "lib.rs",
        Range::with_file(1, 0, 10, 0, "lib.rs".to_string()),
    ));
    global.add_symbol(symbol(
        "other",
        "other",
        "crate::other",
        "other.rs",
        Range::with_file(1, 0, 10, 0, "other.rs".to_string()),
    ));

    let mut resolved = ResolvedEdges::new();
    resolved.mark_call_site_resolved(call_loc);
    resolved.add_call_edge(Edge::new(
        "caller".into(),
        "semantic".into(),
        EdgeKind::Call,
        Provenance::compiler(),
    ));

    let out =
        FallbackEdgeBuilder::create_fallback_edges(&syntax, &global, resolved).expect("fallback");

    assert_eq!(out.call_edges.len(), 1);
    assert!(out.call_edges[0].provenance.corroborating.is_empty());
}

#[test]
fn old_provenance_json_without_corroborating_field_parses() {
    let legacy = r#"{"source":"Lsp","confidence":"High"}"#;
    let p: Provenance = serde_json::from_str(legacy).expect("legacy parse");
    assert_eq!(p.source, ProvenanceSource::Lsp);
    assert!(p.corroborating.is_empty());
}
