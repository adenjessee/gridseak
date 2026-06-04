//! End-to-end heuristic-resolver coverage against the shared Apex corpus.
//!
//! The unit tests inside `apex/heuristic_resolver.rs` exercise the resolver
//! on synthetic hand-built `SyntaxResults`. That validates the *logic* but
//! not the *contract* between the Tree-sitter extractor and the heuristic
//! resolver — a change to symbol extraction could silently regress call-edge
//! resolution without those unit tests noticing.
//!
//! This suite closes that gap:
//!
//! 1. Parse every `.cls` / `.trigger` file under `tests/fixtures/apex/` via
//!    the real `TreeSitterExtractor`, producing a real `SyntaxResults`.
//! 2. Feed that to `ApexHeuristicResolver` and assert the minimum-viable
//!    edges a customer would expect to see on this corpus without an LSP.
//!
//! Intent: pin the heuristic floor. Any regression in symbol extraction,
//! call-site capture, or the trigger-context filter trips these tests.

use graphengine_parsing::application::ports::SemanticResolver;
use graphengine_parsing::application::ports::SyntaxExtractor;
use graphengine_parsing::domain::{Confidence, NodeKind, ProvenanceSource};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::language::apex::ApexHeuristicResolver;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::{Path, PathBuf};

/// Match a method FQN by its simple method name, ignoring the
/// parenthesised parameter-type signature introduced by Sprint E.2.
/// For Apex method/constructor FQNs (`path::Outer::Outer.Inner::name(sig)`)
/// this returns true when the trailing simple name equals `name`.
/// Keeps the existing corpus assertions readable without baking the
/// parameter-type shape into every `ends_with` check.
fn method_simple_name_matches(fqn: &str, name: &str) -> bool {
    let without_sig = match fqn.find('(') {
        Some(i) => &fqn[..i],
        None => fqn,
    };
    let last = without_sig.rsplit(['.', ':']).next().unwrap_or(without_sig);
    last == name
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("apex")
        .join("force-app")
        .join("main")
        .join("default")
}

fn collect_apex_sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let classes = fixture_root().join("classes");
    let triggers = fixture_root().join("triggers");
    for dir in [classes, triggers] {
        let entries =
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
        for entry in entries.flatten() {
            let p = entry.path();
            let is_apex = p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "cls" | "trigger"));
            if is_apex {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

async fn parse_corpus() -> graphengine_parsing::application::ports::SyntaxResults {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    let files = collect_apex_sources();
    assert!(
        !files.is_empty(),
        "apex corpus must contain at least one file"
    );
    extractor.extract(&files).await.expect("extract corpus")
}

// ---------------------------------------------------------------------------
// Tree-sitter extractor sanity — the input contract the resolver depends on
// ---------------------------------------------------------------------------

#[tokio::test]
async fn corpus_extracts_classes_triggers_and_functions() {
    let hints = parse_corpus().await;

    let class_names: Vec<&str> = hints
        .symbols
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Struct))
        .map(|n| n.fqn.rsplit(['.', ':']).next().unwrap_or(&n.fqn))
        .collect();

    for expected in [
        "AccountService",
        "AccountRepository",
        "ContactController",
        "ManagedPackageConsumer",
        "NightlyBatch",
        "AccountService_Test",
        "AccountTrigger",
    ] {
        assert!(
            class_names.contains(&expected),
            "corpus is missing @Struct for {expected}; got {class_names:?}"
        );
    }

    let fn_count = hints
        .symbols
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Function))
        .count();
    assert!(
        fn_count >= 15,
        "corpus should extract at least 15 functions across the fixture; got {fn_count}"
    );

    assert!(
        !hints.references.is_empty(),
        "corpus must surface call sites for the heuristic resolver to work"
    );

    // Trigger-specific properties — plan decision #3: triggers map to
    // NodeKind::Struct with `subtype=trigger` and `sobject=<name>` so
    // downstream analysis can build `Trigger -> SObject` edges without
    // introducing a new NodeKind.
    let trigger = hints
        .symbols
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Struct) && n.fqn.ends_with("AccountTrigger"))
        .expect("AccountTrigger Struct missing");
    assert_eq!(
        trigger.properties.get("subtype").and_then(|v| v.as_str()),
        Some("trigger"),
        "trigger struct must carry subtype=trigger"
    );
    assert_eq!(
        trigger.properties.get("sobject").and_then(|v| v.as_str()),
        Some("Account"),
        "trigger must carry its SObject binding; edge construction depends on this"
    );
}

#[tokio::test]
async fn corpus_tags_istest_class_and_testmethod_function() {
    let hints = parse_corpus().await;

    let is_test = |n: &graphengine_parsing::domain::Node| -> bool {
        n.properties
            .get("is_test")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };

    let test_class = hints
        .symbols
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Struct) && n.fqn.ends_with("AccountService_Test"))
        .expect("AccountService_Test class missing from symbols");
    assert!(
        is_test(test_class),
        "@IsTest class must carry properties.is_test=true from symbol extraction"
    );

    // Both methods inside that class must also be flagged (one via @IsTest,
    // one via the legacy `testMethod` keyword — both paths matter).
    let test_methods = hints
        .symbols
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Function))
        .filter(|n| n.location.file.ends_with("AccountService_Test.cls"))
        .filter(|n| is_test(n))
        .count();
    assert!(
        test_methods >= 2,
        "both methods inside AccountService_Test must be flagged is_test=true; got {test_methods}"
    );
}

#[tokio::test]
async fn corpus_tags_apex_sharing_on_top_level_classes() {
    let hints = parse_corpus().await;

    let sharing_for = |fqn_suffix: &str| -> Option<String> {
        hints
            .symbols
            .iter()
            // Restrict to Struct — the constructor Function shares the
            // class's short FQN and would otherwise shadow the lookup.
            .find(|n| matches!(n.kind, NodeKind::Struct) && n.fqn.ends_with(fqn_suffix))
            .and_then(|n| n.properties.get("apex_sharing"))
            .and_then(|v| v.as_str())
            .map(String::from)
    };

    assert_eq!(
        sharing_for("AccountService").as_deref(),
        Some("with_sharing"),
        "top-level `with sharing` class must carry apex_sharing=with_sharing"
    );
    assert_eq!(
        sharing_for("AccountRepository").as_deref(),
        Some("without_sharing")
    );
    assert_eq!(
        sharing_for("ManagedPackageConsumer").as_deref(),
        Some("inherited_sharing")
    );
    // AccountService_Test omits the modifier — recorded as `omitted` so
    // downstream security findings can flag it.
    assert_eq!(
        sharing_for("AccountService_Test").as_deref(),
        Some("omitted")
    );
}

#[tokio::test]
async fn corpus_tags_entry_points_on_auraenabled_and_invocable_methods() {
    let hints = parse_corpus().await;

    let tags_for = |fn_name: &str| -> Vec<String> {
        hints
            .symbols
            .iter()
            .find(|n| {
                matches!(n.kind, NodeKind::Function) && method_simple_name_matches(&n.fqn, fn_name)
            })
            .and_then(|n| n.properties.get("entry_points"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };

    assert!(
        tags_for("getById").iter().any(|t| t == "aura_enabled"),
        "AccountService.getById must be tagged aura_enabled; got {:?}",
        tags_for("getById")
    );
    assert!(
        tags_for("listByAccount")
            .iter()
            .any(|t| t == "aura_enabled"),
        "ContactController.listByAccount missing aura_enabled tag"
    );
    assert!(
        tags_for("promote").iter().any(|t| t == "invocable_method"),
        "ContactController.promote missing invocable_method tag"
    );
}

// ---------------------------------------------------------------------------
// Heuristic resolver — the user-facing recall floor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn heuristic_resolver_emits_call_edges_on_real_corpus() {
    let hints = parse_corpus().await;
    let resolver = ApexHeuristicResolver::with_standard_preload_only();
    let edges = resolver.resolve(&hints).await.expect("resolve");

    assert!(
        !edges.call_edges.is_empty(),
        "heuristic resolver produced zero call edges on a realistic corpus — \
         this is a regression in call-site capture or name resolution"
    );
    // Every edge must be heuristic-provenance, never masquerading as LSP.
    for e in &edges.call_edges {
        assert_eq!(e.provenance.source, ProvenanceSource::Heuristic);
        assert!(
            matches!(
                e.provenance.confidence,
                Confidence::Medium | Confidence::Low
            ),
            "heuristic confidence must be Medium or Low, never High; got {:?}",
            e.provenance.confidence
        );
    }

    // Stats must agree with what we observed.
    assert_eq!(edges.stats.lsp_edges, 0);
    assert_eq!(edges.stats.heuristic_edges, edges.call_edges.len());
}

#[tokio::test]
async fn heuristic_resolver_filters_trigger_context_accesses() {
    let hints = parse_corpus().await;
    let resolver = ApexHeuristicResolver::with_standard_preload_only();
    let edges = resolver.resolve(&hints).await.expect("resolve");

    // There is no user-defined class named `Trigger` in the corpus, so no
    // call edge may ever target one. Any hit here means the
    // `is_trigger_context_access` filter is broken.
    for e in &edges.call_edges {
        let callee = hints
            .symbols
            .iter()
            .find(|n| n.id == e.to_id)
            .expect("edge target must exist in symbols");
        assert!(
            !callee.fqn.to_ascii_lowercase().starts_with("trigger."),
            "heuristic emitted an edge into a phantom `Trigger.*` symbol: {}",
            callee.fqn
        );
    }
}

#[tokio::test]
async fn heuristic_resolver_connects_account_trigger_to_service() {
    let hints = parse_corpus().await;
    let resolver = ApexHeuristicResolver::with_standard_preload_only();
    let edges = resolver.resolve(&hints).await.expect("resolve");

    // The trigger body calls `svc.markProcessed(Trigger.new)` where `svc`
    // is an `AccountService`. Heuristic resolution strips the receiver and
    // matches on short name `markProcessed` — AccountService.markProcessed
    // is the only candidate in the corpus, so confidence must be Medium.
    let markprocessed_edges: Vec<_> = edges
        .call_edges
        .iter()
        .filter(|e| {
            hints
                .symbols
                .iter()
                .find(|n| n.id == e.to_id)
                .map(|n| method_simple_name_matches(&n.fqn, "markProcessed"))
                .unwrap_or(false)
        })
        .collect();

    assert!(
        !markprocessed_edges.is_empty(),
        "heuristic must resolve `AccountTrigger -> AccountService.markProcessed`; \
         none found in {} total call edges",
        edges.call_edges.len()
    );
    assert!(
        markprocessed_edges
            .iter()
            .any(|e| e.provenance.confidence == Confidence::Medium),
        "unambiguous `markProcessed` must be Medium confidence (single-candidate)"
    );
}
