//! R41 — Apex field-initializer body end-to-end.
//!
//! Problem statement: Apex permits non-trivial default-value expressions
//! on field declarations:
//!
//! ```apex
//! public class ALLO_ManageAllocations_CTRL {
//!     private Map<Id, List<Allocation__c>> allocCache =
//!         new Map<Id, List<Allocation__c>>{
//!             opp.Id => getMappedAllocationsForOpp(opp)
//!         };
//! }
//! ```
//!
//! The call `getMappedAllocationsForOpp(opp)` lives at class scope, not
//! inside a method or constructor body. Pre-R41 the extractor walked
//! only `method_declaration` / `constructor_declaration` bodies for
//! call-site attribution, so every such field-initializer call was
//! silently dropped: the callee showed 0 incoming Call edges and
//! landed in `no_callers_high_confidence`.
//!
//! Fix (R41): `ApexExtractor::synthesize_symbol_siblings` now walks a
//! class body and, for every `variable_declarator` whose `value:`
//! initializer contains at least one `method_invocation` /
//! `object_creation_expression` subtree, synthesizes one `Function`
//! node with FQN `<path>::<ClassDotted>::<fieldName>.__init__()`
//! covering the initializer range. The resolver's
//! `find_enclosing_function` then attributes the enclosed call site
//! to the synthetic `__init__` caller.
//!
//! This test validates the full pipeline end-to-end:
//!   1. extractor emits the synthetic `__init__` Function,
//!   2. a `Contains` edge goes from the class Struct → synthetic
//!      Function,
//!   3. the synthetic range covers the top-level call site,
//!   4. the heuristic resolver produces a `Call` edge from the
//!      synthetic `__init__` to the sibling method.
//!
//! Regression shape matches NPSP's
//! `ALLO_ManageAllocations_CTRL::getMappedAllocationsForOpp` sample
//! (rev 9 Round 5 hand-audit sample #3) and the published R41 recipe
//! in `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md`.

use graphengine_parsing::application::ports::{SemanticResolver, SyntaxExtractor};
use graphengine_parsing::domain::{EdgeKind, Node, NodeKind};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::language::apex::ApexHeuristicResolver;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::PathBuf;
use tempfile::TempDir;

fn write_fixture(dir: &std::path::Path) -> Vec<PathBuf> {
    // Minimal reproduction of the NPSP `ALLO_ManageAllocations_CTRL`
    // shape: a map-literal field initializer whose value expression
    // calls a sibling instance method. Keeps the controller-and-helper
    // duality but strips everything the test doesn't need (Salesforce
    // SObjects, DML, controller wiring).
    let src = "\
public class AllocCache {
    public AllocCache(Opp opp) {
        this.opp = opp;
    }

    private Opp opp;

    private Map<Id, Integer> allocCache = new Map<Id, Integer>{
        opp.id => getMappedAllocationsForOpp(opp)
    };

    public Integer getMappedAllocationsForOpp(Opp o) {
        return 1;
    }
}

public class Opp {
    public Id id;
}
";
    let p = dir.join("AllocCache.cls");
    std::fs::write(&p, src).expect("write fixture");
    vec![p]
}

fn find_node<P: Fn(&Node) -> bool>(nodes: &[Node], pred: P) -> Option<&Node> {
    nodes.iter().find(|n| pred(n))
}

#[tokio::test]
async fn field_initializer_synthesizes_init_function_and_attributes_call() {
    let tmp = TempDir::new().expect("tempdir");
    let files = write_fixture(tmp.path());

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("extractor");
    let hints = extractor.extract(&files).await.expect("parse fixture");

    // 1. Synthetic `<field>.__init__` Function exists.
    let synthetic = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Function
            && n.fqn.ends_with("::AllocCache::allocCache.__init__()")
            && n.properties
                .get("synthetic")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            && n.properties.get("synthetic_kind").and_then(|v| v.as_str())
                == Some("apex_field_initializer")
    })
    .unwrap_or_else(|| {
        panic!(
            "synthetic `allocCache.__init__()` Function missing; symbols: {:?}",
            hints
                .symbols
                .iter()
                .map(|n| (n.kind, n.fqn.as_str()))
                .collect::<Vec<_>>()
        )
    });

    // 2. Class Struct → synthetic Contains edge exists.
    let class_struct = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Struct && n.fqn.ends_with("::AllocCache")
    })
    .expect("class Struct node missing");
    let has_contains = hints.synthesized_edges.iter().any(|e| {
        e.kind == EdgeKind::Contains && e.from_id == class_struct.id && e.to_id == synthetic.id
    });
    assert!(
        has_contains,
        "class Struct → __init__ Contains edge missing; synthesized_edges: {:?}",
        hints
            .synthesized_edges
            .iter()
            .map(|e| (e.kind, e.from_id.as_str(), e.to_id.as_str()))
            .collect::<Vec<_>>()
    );

    // 3. Synthetic range covers the enclosed call site.
    let call_site = hints
        .iter_all_call_sites()
        .find(|cs| {
            cs.function_name == "getMappedAllocationsForOpp"
                || cs.function_name.ends_with(":getMappedAllocationsForOpp")
        })
        .unwrap_or_else(|| {
            panic!(
                "expected a getMappedAllocationsForOpp call site; got: {:?}",
                hints
                    .iter_all_call_sites()
                    .map(|cs| cs.function_name.clone())
                    .collect::<Vec<_>>()
            )
        });
    let cs = &call_site.location;
    let syn = &synthetic.location;
    assert!(
        syn.start_line <= cs.start_line && syn.end_line >= cs.end_line,
        "synthetic __init__ range {:?} does not cover call site {:?}",
        syn,
        cs
    );

    // 4. Heuristic resolver produces a Call edge from __init__ to the sibling method.
    let resolver = ApexHeuristicResolver::with_standard_preload_only();
    let out = resolver.resolve(&hints).await.expect("resolve");

    let callee = find_node(&hints.symbols, |n| {
        n.kind == NodeKind::Function && n.fqn.contains("::AllocCache::getMappedAllocationsForOpp(")
    })
    .unwrap_or_else(|| {
        panic!(
            "getMappedAllocationsForOpp method missing; functions: {:?}",
            hints
                .symbols
                .iter()
                .filter(|n| n.kind == NodeKind::Function)
                .map(|n| n.fqn.as_str())
                .collect::<Vec<_>>()
        )
    });
    let call_edge = out
        .call_edges
        .iter()
        .find(|e| e.kind == EdgeKind::Call && e.from_id == synthetic.id && e.to_id == callee.id);
    assert!(
        call_edge.is_some(),
        "expected Call edge allocCache.__init__ → getMappedAllocationsForOpp; got: {:?}",
        out.call_edges
            .iter()
            .map(|e| (e.kind, e.from_id.as_str(), e.to_id.as_str()))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn literal_only_field_does_not_synthesize_init_function() {
    // Regression: primitive / literal-only field initializers should
    // NOT produce synthetic __init__ nodes. Emitting one for every
    // `private Integer n = 5;` would flood the graph with callerless
    // Function nodes and re-contaminate `no_callers_high_confidence`.
    let tmp = TempDir::new().expect("tempdir");
    let src = "\
public class Constants {
    public static final Integer MAX = 100;
    public static final String GREETING = 'hello';
    public Integer counter = 0;
}
";
    let p = tmp.path().join("Constants.cls");
    std::fs::write(&p, src).expect("write");
    let files = vec![p];

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("extractor");
    let hints = extractor.extract(&files).await.expect("parse");

    let synthetic_inits: Vec<&Node> = hints
        .symbols
        .iter()
        .filter(|n| {
            n.properties.get("synthetic_kind").and_then(|v| v.as_str())
                == Some("apex_field_initializer")
        })
        .collect();
    assert!(
        synthetic_inits.is_empty(),
        "literal-only fields produced synthetic __init__ nodes: {:?}",
        synthetic_inits
            .iter()
            .map(|n| n.fqn.as_str())
            .collect::<Vec<_>>()
    );
}
