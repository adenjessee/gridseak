//! Sprint E.1 + universal-fidelity T1 — EdgeKind taxonomy tests.
//!
//! Pins three invariants (post-P1.b rework):
//!   1. Every `EdgeKind` variant round-trips through serde JSON. The
//!      literal wire strings per variant are pinned in
//!      `t1_edgekind_roundtrip.rs`; this test is the exhaustive
//!      round-trip check.
//!   2. Extends / Implements are accepted by `Edge::new` when the
//!      from/to nodes differ (Sprint E.1 contract).
//!   3. `EdgeKind::Framework(FrameworkKind::VisualforcePage)` and
//!      `EdgeKind::Declarative(DeclarativeKind::Flow)` are accepted by
//!      the typed constructors (`Edge::framework` / `Edge::declarative`)
//!      and by `Edge::new`. The taxonomy is live, not aspirational.
//!
//! Stability contract: the wire format is the serde-tagged JSON
//! produced by `#[serde(tag = "kind", content = "sub")]`. Literal
//! wire strings are pinned in `t1_edgekind_roundtrip.rs`; any change
//! there is a breaking change to the `parse.db` schema and must be
//! accompanied by a `schema_version` bump.

use graphengine_parsing::domain::{DeclarativeKind, Edge, EdgeKind, FrameworkKind, Provenance};

fn all_variants() -> Vec<EdgeKind> {
    vec![
        EdgeKind::Call,
        EdgeKind::Contains,
        EdgeKind::Import,
        EdgeKind::Type,
        EdgeKind::Uses,
        EdgeKind::Extends,
        EdgeKind::Implements,
        EdgeKind::Framework(FrameworkKind::VisualforcePage),
        EdgeKind::Declarative(DeclarativeKind::Flow),
    ]
}

#[test]
fn edge_kind_serde_round_trip_includes_every_variant() {
    for kind in all_variants() {
        let json = serde_json::to_string(&kind).expect("serialize");
        let back: EdgeKind = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            kind, back,
            "serde round-trip lost variant {kind:?} (wire: {json})"
        );
    }
}

#[test]
fn edge_constructors_accept_extends_and_implements() {
    let p = Provenance::heuristic();
    let a = "node-a".to_string();
    let b = "node-b".to_string();

    let extends = Edge::extends(a.clone(), b.clone(), p);
    assert_eq!(extends.kind, EdgeKind::Extends);

    let implements = Edge::implements(a.clone(), b.clone(), p);
    assert_eq!(implements.kind, EdgeKind::Implements);

    let e = Edge::new(a.clone(), b.clone(), EdgeKind::Extends, p);
    assert_eq!(e.kind, EdgeKind::Extends);
    let i = Edge::new(a, b, EdgeKind::Implements, p);
    assert_eq!(i.kind, EdgeKind::Implements);
}

#[test]
fn edge_constructors_accept_framework_and_declarative_variants() {
    let p = Provenance::heuristic();
    let a = "node-a".to_string();
    let b = "node-b".to_string();

    let fw = Edge::framework(a.clone(), b.clone(), FrameworkKind::VisualforcePage, p);
    assert_eq!(
        fw.kind,
        EdgeKind::Framework(FrameworkKind::VisualforcePage),
        "Edge::framework constructor must produce the typed Framework variant"
    );

    let dec = Edge::declarative(a.clone(), b.clone(), DeclarativeKind::Flow, p);
    assert_eq!(dec.kind, EdgeKind::Declarative(DeclarativeKind::Flow));

    let via_new = Edge::new(a, b, EdgeKind::Framework(FrameworkKind::VisualforcePage), p);
    assert_eq!(
        via_new.kind,
        EdgeKind::Framework(FrameworkKind::VisualforcePage)
    );
}

#[test]
fn framework_and_declarative_are_call_like_but_not_containment() {
    let fw = EdgeKind::Framework(FrameworkKind::VisualforcePage);
    let dec = EdgeKind::Declarative(DeclarativeKind::Flow);
    assert!(
        fw.is_call_like(),
        "Framework must be in the call-like family"
    );
    assert!(
        dec.is_call_like(),
        "Declarative must be in the call-like family"
    );
    assert!(!fw.is_containment());
    assert!(!dec.is_containment());
    assert!(fw.is_structural());
    assert!(dec.is_structural());
}
