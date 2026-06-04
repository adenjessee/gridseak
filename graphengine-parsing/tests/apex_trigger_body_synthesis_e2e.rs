//! Sprint E.3 — Synthetic `__trigger__` function end-to-end.
//!
//! Problem: Apex triggers have no enclosing method in source. Their
//! top level is a bare statement block hanging off `trigger_declaration`.
//! Without a caller Function in the graph the heuristic resolver's
//! range-containment lookup returns `None` for every top-level call
//! site, and those calls never become Call edges.
//!
//! Fix: for each trigger, `symbol_extractor` synthesises a `Function`
//! node spanning the trigger body (FQN
//! `<path>::<TriggerName>::__trigger__()`, `properties.synthetic =
//! true`) and emits a `Contains` edge from the trigger Struct to it.
//!
//! This test validates the full pipeline:
//!   1. extractor emits the synthetic Function,
//!   2. a Contains edge goes from trigger Struct → synthetic Function,
//!   3. call sites inside the trigger body resolve their caller to the
//!      synthetic Function (so Call edges exist and are attributed).

use graphengine_parsing::application::ports::{SemanticResolver, SyntaxExtractor};
use graphengine_parsing::domain::{EdgeKind, Node, NodeKind};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::language::apex::ApexHeuristicResolver;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::PathBuf;
use tempfile::TempDir;

fn write_fixture(dir: &std::path::Path) -> Vec<PathBuf> {
    // Handler class with a static entry point — the trigger delegates
    // to it. Represents the canonical "thin trigger, fat handler"
    // pattern used across every Apex style guide.
    let handler_src = "\
public class AccountTriggerHandler {
    public static void run(List<Account> accounts) {
        for (Account a : accounts) {
            a.Description = 'touched';
        }
    }
}
";

    // Trigger calls the handler statically at the top level of the
    // trigger body. Pre-E.3 this call had no caller to attach to and
    // was dropped from the graph.
    let trigger_src = "\
trigger AccountTrigger on Account (before insert, before update) {
    AccountTriggerHandler.run(Trigger.new);
}
";

    let files = [
        ("AccountTriggerHandler.cls", handler_src),
        ("AccountTrigger.trigger", trigger_src),
    ];
    let mut written = Vec::new();
    for (name, src) in files {
        let p = dir.join(name);
        std::fs::write(&p, src).expect("write fixture");
        written.push(p);
    }
    written
}

fn find_node<P: Fn(&Node) -> bool>(nodes: &[Node], pred: P) -> Option<&Node> {
    nodes.iter().find(|n| pred(n))
}

#[tokio::test]
async fn trigger_body_synthesises_function_and_attributes_top_level_calls() {
    let tmp = TempDir::new().expect("tempdir");
    let files = write_fixture(tmp.path());

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("extractor");
    let hints = extractor.extract(&files).await.expect("parse fixture");

    // -- 1. Synthetic `__trigger__` Function must exist ------------------
    let synthetic = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Function
            && n.fqn.ends_with("::AccountTrigger::__trigger__()")
            && n.properties
                .get("synthetic")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
    })
    .unwrap_or_else(|| {
        panic!(
            "synthetic __trigger__ Function missing; symbols: {:?}",
            hints
                .symbols
                .iter()
                .map(|n| (n.kind, n.fqn.as_str()))
                .collect::<Vec<_>>()
        )
    });

    // -- 2. Trigger Struct → synthetic Function Contains edge ------------
    let trigger_struct = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Struct
            && n.fqn.ends_with("::AccountTrigger")
            && n.properties.get("subtype").and_then(|v| v.as_str()) == Some("trigger")
    })
    .expect("trigger Struct node missing");

    let has_contains = hints.synthesized_edges.iter().any(|e| {
        e.kind == EdgeKind::Contains && e.from_id == trigger_struct.id && e.to_id == synthetic.id
    });
    assert!(
        has_contains,
        "trigger Struct → __trigger__ Contains edge missing; synthesized_edges: {:?}",
        hints
            .synthesized_edges
            .iter()
            .map(|e| (e.kind, e.from_id.as_str(), e.to_id.as_str()))
            .collect::<Vec<_>>()
    );

    // -- 3. Synthetic Function range must cover the top-level call ------
    //
    // If the range is wrong the resolver's "smallest enclosing function"
    // lookup can't attribute the call to __trigger__, which is the only
    // reason the synthetic node exists.
    let call_inside_trigger = hints
        .iter_all_call_sites()
        .find(|cs| {
            cs.location.file.ends_with("AccountTrigger.trigger")
                && (cs.function_name == "run" || cs.function_name.ends_with(":run"))
        })
        .unwrap_or_else(|| {
            panic!(
                "top-level call site inside trigger body missing; all call sites: {:?}",
                hints
                    .iter_all_call_sites()
                    .map(|cs| (cs.function_name.as_str(), cs.location.file.as_str()))
                    .collect::<Vec<_>>()
            )
        });
    let cs = &call_inside_trigger.location;
    let syn = &synthetic.location;
    assert!(
        syn.start_line <= cs.start_line && syn.end_line >= cs.end_line,
        "synthetic Function range {:?} does not cover call site {:?}",
        syn,
        cs
    );

    // -- 4. Heuristic resolver attributes the Call edge to __trigger__ ---
    let resolver = ApexHeuristicResolver::with_standard_preload_only();
    let out = resolver.resolve(&hints).await.expect("resolve");

    let callee = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Function && n.fqn.contains("::AccountTriggerHandler::run(")
    })
    .expect("handler run(...) function missing");

    let call_edge = out
        .call_edges
        .iter()
        .find(|e| e.kind == EdgeKind::Call && e.from_id == synthetic.id && e.to_id == callee.id);
    assert!(
        call_edge.is_some(),
        "expected Call edge __trigger__ → AccountTriggerHandler.run; got: {:?}",
        out.call_edges
            .iter()
            .map(|e| (e.kind, e.from_id.as_str(), e.to_id.as_str()))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn non_trigger_classes_do_not_get_synthetic_functions() {
    // Regression: sibling synthesis must fire ONLY for trigger
    // declarations. A plain class should not gain a phantom
    // `__trigger__` method.
    let tmp = TempDir::new().expect("tempdir");
    let src = "\
public class PlainService {
    public void greet() {
        System.debug('hi');
    }
}
";
    let p = tmp.path().join("PlainService.cls");
    std::fs::write(&p, src).expect("write");
    let files = vec![p];

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("extractor");
    let hints = extractor.extract(&files).await.expect("parse");

    let synthetic_count = hints
        .symbols
        .iter()
        .filter(|n| {
            n.properties
                .get("synthetic")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .count();
    assert_eq!(
        synthetic_count,
        0,
        "plain class fixture produced synthetic nodes: {:?}",
        hints
            .symbols
            .iter()
            .filter(|n| n.properties.contains_key("synthetic"))
            .map(|n| n.fqn.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        hints
            .synthesized_edges
            .iter()
            .all(|e| e.kind != EdgeKind::Contains
                || !hints.symbols.iter().any(|n| n.id == e.to_id
                    && n.properties
                        .get("synthetic")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false))),
        "plain class produced a Contains edge to a synthetic child"
    );
}
