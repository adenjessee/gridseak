//! R39 — Apex property accessor body end-to-end.
//!
//! Problem statement: Apex properties have the shape
//!
//! ```apex
//! public Map<Id, Account> accountById {
//!     get {
//!         if (accountById == null) {
//!             loadAccountByIdMap();
//!         }
//!         return accountById;
//!     }
//!     set;
//! }
//! ```
//!
//! which the tree-sitter-sfapex grammar parses as a `field_declaration`
//! carrying an `accessor_list` child (NOT a separate
//! `property_declaration` node). Pre-R39 the extractor walked only
//! `method_declaration` / `constructor_declaration` bodies for call
//! attribution — accessor bodies were invisible to the resolver, so
//! every call inside a `get { ... }` / `set { ... }` body was silently
//! dropped.
//!
//! Fix (R39): `ApexExtractor::synthesize_symbol_siblings` now emits one
//! synthetic `Function` node per `accessor_declaration` with a non-null
//! `body:` block, FQN
//! `<path>::<ClassDotted>::<propertyName>.__get__()` or `.__set__()`
//! covering the accessor-body range. The resolver's
//! `find_enclosing_function` then attributes the enclosed call site
//! to the synthetic accessor.
//!
//! This test validates the pipeline end-to-end:
//!   1. extractor emits the synthetic `__get__` / `__set__` Function,
//!   2. `Contains` edge wires the class Struct → synthetic Function,
//!   3. synthetic range covers the accessor-body call site,
//!   4. heuristic resolver produces a `Call` edge from the synthetic
//!      accessor to the sibling method.
//!
//! Regression shape matches NPSP's
//! `Contacts::loadAccountByIdMap` sample (PR 4 post-ship gap documented
//! in `FOLLOWUP_RISKS.md` §R39).

use graphengine_parsing::application::ports::{SemanticResolver, SyntaxExtractor};
use graphengine_parsing::domain::{EdgeKind, Node, NodeKind};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::language::apex::ApexHeuristicResolver;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::PathBuf;
use tempfile::TempDir;

fn write_fixture(dir: &std::path::Path) -> Vec<PathBuf> {
    // Minimal reproduction of the NPSP `Contacts::loadAccountByIdMap`
    // shape: a property whose `get { ... }` body lazily invokes a
    // sibling instance method; `set { ... }` body invalidates via
    // another sibling call. Exercises both accessor halves.
    let src = "\
public class Contacts {
    private Map<Id, Integer> cachedById;

    public Map<Id, Integer> accountById {
        get {
            if (cachedById == null) {
                loadAccountByIdMap();
            }
            return cachedById;
        }
        set {
            invalidate();
        }
    }

    public void loadAccountByIdMap() {
        cachedById = new Map<Id, Integer>();
    }

    public void invalidate() {
        cachedById = null;
    }
}
";
    let p = dir.join("Contacts.cls");
    std::fs::write(&p, src).expect("write fixture");
    vec![p]
}

fn find_node<P: Fn(&Node) -> bool>(nodes: &[Node], pred: P) -> Option<&Node> {
    nodes.iter().find(|n| pred(n))
}

#[tokio::test]
async fn property_accessor_bodies_synthesize_functions_and_attribute_calls() {
    let tmp = TempDir::new().expect("tempdir");
    let files = write_fixture(tmp.path());

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("extractor");
    let hints = extractor.extract(&files).await.expect("parse fixture");

    // -- 1. Both synthetic accessor Function nodes exist -----------------
    let getter = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Function
            && n.fqn.ends_with("::Contacts::accountById.__get__()")
            && n.properties.get("synthetic_kind").and_then(|v| v.as_str())
                == Some("apex_property_get")
    })
    .unwrap_or_else(|| {
        panic!(
            "synthetic `accountById.__get__()` missing; functions: {:?}",
            hints
                .symbols
                .iter()
                .filter(|n| n.kind == NodeKind::Function)
                .map(|n| n.fqn.as_str())
                .collect::<Vec<_>>()
        )
    });

    let setter = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Function
            && n.fqn.ends_with("::Contacts::accountById.__set__()")
            && n.properties.get("synthetic_kind").and_then(|v| v.as_str())
                == Some("apex_property_set")
    })
    .expect("synthetic `accountById.__set__()` missing");

    // -- 2. Contains edges from the class Struct to each synthetic ------
    let class_struct = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Struct && n.fqn.ends_with("::Contacts")
    })
    .expect("Contacts Struct node missing");
    for syn in [getter, setter] {
        let has_contains = hints.synthesized_edges.iter().any(|e| {
            e.kind == EdgeKind::Contains && e.from_id == class_struct.id && e.to_id == syn.id
        });
        assert!(
            has_contains,
            "class Struct → {} Contains edge missing",
            syn.fqn
        );
    }

    // -- 3. Synthetic ranges cover the enclosed call sites --------------
    let call_to_load = hints
        .iter_all_call_sites()
        .find(|cs| {
            cs.function_name == "loadAccountByIdMap"
                || cs.function_name.ends_with(":loadAccountByIdMap")
        })
        .expect("loadAccountByIdMap call site missing");
    assert!(
        getter.location.start_line <= call_to_load.location.start_line
            && getter.location.end_line >= call_to_load.location.end_line,
        "getter range {:?} does not cover call site {:?}",
        getter.location,
        call_to_load.location
    );

    let call_to_invalidate = hints
        .iter_all_call_sites()
        .find(|cs| cs.function_name == "invalidate" || cs.function_name.ends_with(":invalidate"))
        .expect("invalidate call site missing");
    assert!(
        setter.location.start_line <= call_to_invalidate.location.start_line
            && setter.location.end_line >= call_to_invalidate.location.end_line,
        "setter range {:?} does not cover call site {:?}",
        setter.location,
        call_to_invalidate.location
    );

    // -- 4. Heuristic resolver emits Call edges from synthetics ---------
    let resolver = ApexHeuristicResolver::with_standard_preload_only();
    let out = resolver.resolve(&hints).await.expect("resolve");

    let load_fn = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Function && n.fqn.contains("::Contacts::loadAccountByIdMap(")
    })
    .expect("loadAccountByIdMap method missing");
    let invalidate_fn = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Function && n.fqn.contains("::Contacts::invalidate(")
    })
    .expect("invalidate method missing");

    let get_edge = out
        .call_edges
        .iter()
        .find(|e| e.kind == EdgeKind::Call && e.from_id == getter.id && e.to_id == load_fn.id);
    assert!(
        get_edge.is_some(),
        "expected Call edge __get__ → loadAccountByIdMap; got: {:?}",
        out.call_edges
            .iter()
            .map(|e| (e.kind, e.from_id.as_str(), e.to_id.as_str()))
            .collect::<Vec<_>>()
    );

    let set_edge = out.call_edges.iter().find(|e| {
        e.kind == EdgeKind::Call && e.from_id == setter.id && e.to_id == invalidate_fn.id
    });
    assert!(
        set_edge.is_some(),
        "expected Call edge __set__ → invalidate; got: {:?}",
        out.call_edges
            .iter()
            .map(|e| (e.kind, e.from_id.as_str(), e.to_id.as_str()))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn auto_implemented_property_has_no_accessor_bodies_and_no_synth() {
    // Regression: `public Integer n { get; set; }` — no accessor
    // bodies present → no synthetic Function nodes should be emitted.
    // Eagerly synthesising get/set for auto-implemented properties
    // would re-introduce callerless Function nodes at class scope and
    // re-contaminate `no_callers_high_confidence`.
    let tmp = TempDir::new().expect("tempdir");
    let src = "\
public class AutoProps {
    public Integer counter { get; set; }
    public String label { get; private set; }
}
";
    let p = tmp.path().join("AutoProps.cls");
    std::fs::write(&p, src).expect("write");
    let files = vec![p];

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("extractor");
    let hints = extractor.extract(&files).await.expect("parse");

    let accessor_synth: Vec<&Node> = hints
        .symbols
        .iter()
        .filter(|n| {
            matches!(
                n.properties.get("synthetic_kind").and_then(|v| v.as_str()),
                Some("apex_property_get") | Some("apex_property_set")
            )
        })
        .collect();
    assert!(
        accessor_synth.is_empty(),
        "auto-implemented properties produced synthetic accessor nodes: {:?}",
        accessor_synth
            .iter()
            .map(|n| n.fqn.as_str())
            .collect::<Vec<_>>()
    );
}
