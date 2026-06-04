//! Universal-fidelity T1 — post-P1.b `EdgeKind` wire-format pin.
//!
//! This test pins the **post-rework** serde wire format for every
//! shipping `EdgeKind` variant using literal string assertions. The
//! pre-rework hand-rolled `to_stable_str` / `from_stable_str` pair
//! was deleted in P1.b; the wire format is now produced entirely by
//! serde via `#[serde(tag = "kind", content = "sub")]` on `EdgeKind`.
//!
//! Why this file exists. A serde derive is easy to break silently —
//! adding `#[serde(rename = "...")]` on one variant, reordering
//! variants in a way that hits a derive edge case, or a future-Rust
//! compiler-flag change could all drift the on-disk representation
//! without a compile error. Pinning the literal wire string for every
//! variant converts any such drift into a test failure with a
//! human-readable diff that names the broken variant.
//!
//! Changing any assertion in this file is a breaking change to the
//! `parse.db` schema. The change MUST be accompanied by (a) a
//! `schema_version` bump in `SqliteRepository::initialize_schema`
//! and (b) an entry in `docs/04-architecture/EDGE_TAXONOMY.md`
//! documenting the rename or variant change.
//!
//! The variant list here is exhaustive by design. If you add a new
//! `EdgeKind` variant or a new `FrameworkKind` / `DeclarativeKind`
//! sub-variant and this test does not fail, the new variant is not
//! actually being exercised — add it to `all_shipping_variants()`
//! below.

use graphengine_parsing::application::ports::{CallSite, FrameworkBinding, UnresolvedReference};
use graphengine_parsing::domain::{DeclarativeKind, EdgeKind, FrameworkKind, Range};
use graphengine_parsing::infrastructure::lsp::build_resolved_call_edge;

fn all_shipping_variants() -> Vec<(EdgeKind, &'static str)> {
    vec![
        (EdgeKind::Call, r#"{"kind":"Call"}"#),
        (EdgeKind::Contains, r#"{"kind":"Contains"}"#),
        (EdgeKind::Import, r#"{"kind":"Import"}"#),
        (EdgeKind::Extends, r#"{"kind":"Extends"}"#),
        (EdgeKind::Implements, r#"{"kind":"Implements"}"#),
        (EdgeKind::Type, r#"{"kind":"Type"}"#),
        (EdgeKind::Uses, r#"{"kind":"Uses"}"#),
        (
            EdgeKind::Framework(FrameworkKind::VisualforcePage),
            r#"{"kind":"Framework","sub":"VisualforcePage"}"#,
        ),
        (
            EdgeKind::Declarative(DeclarativeKind::Flow),
            r#"{"kind":"Declarative","sub":"Flow"}"#,
        ),
    ]
}

#[test]
fn every_variant_serialises_to_its_pinned_wire_string() {
    for (kind, expected) in all_shipping_variants() {
        let got = serde_json::to_string(&kind).expect("serialize");
        assert_eq!(
            got, expected,
            "wire-format drift for {kind:?}: serialise-then-compare failed. \
             If this was intentional, bump schema_version in \
             SqliteRepository::initialize_schema AND update \
             docs/04-architecture/EDGE_TAXONOMY.md."
        );
    }
}

#[test]
fn every_variant_deserialises_from_its_pinned_wire_string() {
    for (kind, expected) in all_shipping_variants() {
        let got: EdgeKind = serde_json::from_str(expected).expect("deserialize pinned wire string");
        assert_eq!(
            got, kind,
            "pinned wire string {expected} no longer deserialises to {kind:?}"
        );
    }
}

#[test]
fn serde_deserialize_returns_err_on_unknown_tag() {
    // `Self::Unknown` wrapping happens at the `PersistedEdgeKind`
    // boundary (P1.c); raw `EdgeKind` deserialization of an unknown
    // tag must fail, never panic or silently default.
    let cases = [
        r#"{"kind":"NotAVariant"}"#,
        r#"{"kind":"Framework","sub":"NotASubvariant"}"#,
        r#"{"kind":"Declarative","sub":"NotAFlow"}"#,
        r#"{"kind":""}"#,
        r#"{}"#,
    ];
    for json in cases {
        let res: Result<EdgeKind, _> = serde_json::from_str(json);
        assert!(
            res.is_err(),
            "unknown kind string {json} must fail to deserialise into EdgeKind (got {:?})",
            res
        );
    }
}

#[test]
fn real_resolver_honours_unresolved_reference_edge_kind() {
    // UF-FU-005 regression gate.
    //
    // The P1.d design doc (`docs/workstreams/universal-fidelity/tasks/T1-rework.md`
    // §4 "Chosen shape") claims every resolver dispatches on
    // `UnresolvedReference`'s typed variant via `reference.edge_kind()`.
    // Before A3, `RealLspResolver::build_resolved_call_edge` silently
    // hardcoded `EdgeKind::Call` for any variant, breaking the claim
    // the moment a `FrameworkBinding` or `DeclarativeBinding` routes
    // through the LSP path.
    //
    // This test locks the typed-dispatch contract at the LSP edge-
    // construction helper. It fails against the pre-A3 code (hardcoded
    // `EdgeKind::Call`) and passes once the helper delegates to
    // `reference.edge_kind()`.
    let framework_reference = UnresolvedReference::FrameworkBinding(FrameworkBinding {
        framework: FrameworkKind::VisualforcePage,
        call_site: CallSite {
            location: Range::with_file(10, 0, 10, 20, "DummyController.cls".to_string()),
            function_name: "refreshJobs".to_string(),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        },
    });

    let edge = build_resolved_call_edge(
        "caller_node".to_string(),
        "callee_node".to_string(),
        &framework_reference,
        "refreshJobs",
    )
    .expect("non-self-loop edge must be produced");

    assert_eq!(
        edge.kind,
        EdgeKind::Framework(FrameworkKind::VisualforcePage),
        "build_resolved_call_edge must honour UnresolvedReference::edge_kind(); \
         hardcoding EdgeKind::Call here breaks the P1.d typed-dispatch contract \
         at the LSP consumer."
    );
}

#[test]
fn real_resolver_skips_self_loops_regardless_of_variant() {
    // Self-loop suppression is a behavioural invariant of
    // `build_resolved_call_edge`. The P1.d typed-dispatch change must
    // not inadvertently change this behaviour. Lock it for every
    // variant so a future signature change cannot silently widen or
    // narrow the self-loop policy per variant.
    let references: Vec<UnresolvedReference> = vec![
        UnresolvedReference::Call(CallSite {
            location: Range::with_file(1, 0, 1, 5, "f.rs".to_string()),
            function_name: "f".to_string(),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        }),
        UnresolvedReference::FrameworkBinding(FrameworkBinding {
            framework: FrameworkKind::VisualforcePage,
            call_site: CallSite {
                location: Range::with_file(1, 0, 1, 5, "f.cls".to_string()),
                function_name: "f".to_string(),
                receiver_range: None,
                receiver_text: None,
                arg_types: Vec::new(),
            },
        }),
    ];
    for reference in &references {
        let edge =
            build_resolved_call_edge("same_id".to_string(), "same_id".to_string(), reference, "f");
        assert!(
            edge.is_none(),
            "self-loop must return None for variant {reference:?}"
        );
    }
}

#[test]
fn framework_sub_and_declarative_sub_serialise_as_bare_strings() {
    // The `sub` field serialises as the externally-tagged form of the
    // inner enum — a bare string, not `{"VisualforcePage":null}`.
    // This invariant lets the SQLite `edges.kind` column stay
    // grep-friendly for a VF-specific scan.
    let wire = serde_json::to_string(&EdgeKind::Framework(FrameworkKind::VisualforcePage))
        .expect("serialize");
    assert!(
        wire.contains(r#""sub":"VisualforcePage""#),
        "expected bare-string `sub`, got {wire}"
    );

    let wire =
        serde_json::to_string(&EdgeKind::Declarative(DeclarativeKind::Flow)).expect("serialize");
    assert!(
        wire.contains(r#""sub":"Flow""#),
        "expected bare-string `sub`, got {wire}"
    );
}
