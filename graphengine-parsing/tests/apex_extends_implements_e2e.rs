//! Sprint E.1 — Extends / Implements end-to-end coverage for Apex.
//!
//! Flow exercised:
//! 1. Write an in-memory SFDX slice to a tempdir (interface + abstract base
//!    + concrete subclass that extends the base and implements the interface).
//! 2. Parse via the real `TreeSitterExtractor`, which runs `apex.yaml`'s
//!    `inheritance` query through `TypeRefExtractor::extract_inheritance`.
//! 3. Run `ApexHeuristicResolver` on the result.
//! 4. Assert the output carries distinct `EdgeKind::Extends` and
//!    `EdgeKind::Implements` edges — NOT the legacy collapsed `EdgeKind::Type`.
//!
//! Pinning this end-to-end ensures every layer (Tree-sitter query →
//! `TypeUsageKind::Extends/Implements` → `EdgeKind::Extends/Implements`)
//! stays aligned. A regression in any one layer trips this test.

use graphengine_parsing::application::ports::{SemanticResolver, SyntaxExtractor};
use graphengine_parsing::domain::{EdgeKind, NodeKind};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::language::apex::ApexHeuristicResolver;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::PathBuf;
use tempfile::TempDir;

/// Fixture: interface + abstract class + concrete class with both
/// `extends` and `implements` heritage on the concrete class. Kept in
/// this test file so it lives and dies with the test — no risk of it
/// getting mutated out from under us by an unrelated fixture change.
fn write_fixture(dir: &std::path::Path) -> Vec<PathBuf> {
    let interface_src = "\
public interface IAccountRepository {
    List<Account> query(Integer limitCount);
    void persist(List<Account> accounts);
}
";
    let abstract_src = "\
public abstract class AccountRepositoryBase {
    protected Integer defaultLimit() {
        return 50;
    }
}
";
    let concrete_src = "\
public class MyAccountRepo extends AccountRepositoryBase implements IAccountRepository {
    public List<Account> query(Integer limitCount) {
        return [SELECT Id FROM Account LIMIT :limitCount];
    }

    public void persist(List<Account> accounts) {
        update accounts;
    }
}
";

    let files = [
        ("IAccountRepository.cls", interface_src),
        ("AccountRepositoryBase.cls", abstract_src),
        ("MyAccountRepo.cls", concrete_src),
    ];

    let mut written = Vec::new();
    for (name, src) in files {
        let p = dir.join(name);
        std::fs::write(&p, src).expect("write fixture");
        written.push(p);
    }
    written
}

#[tokio::test]
async fn concrete_class_emits_extends_and_implements_edges() {
    let tmp = TempDir::new().expect("tempdir");
    let files = write_fixture(tmp.path());

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("extractor");
    let hints = extractor.extract(&files).await.expect("parse fixture");

    // Sanity: every expected symbol was extracted.
    let has_symbol = |fqn_suffix: &str, kind: NodeKind| -> bool {
        hints
            .symbols
            .iter()
            .any(|n| n.kind == kind && n.fqn.ends_with(fqn_suffix))
    };
    assert!(
        has_symbol("IAccountRepository", NodeKind::Interface),
        "interface symbol missing"
    );
    assert!(
        has_symbol("AccountRepositoryBase", NodeKind::Struct),
        "abstract class symbol missing"
    );
    assert!(
        has_symbol("MyAccountRepo", NodeKind::Struct),
        "concrete class symbol missing"
    );

    // The type references surfaced by `extract_inheritance` carry the
    // right `TypeUsageKind`. The resolver only gets to mint Extends /
    // Implements edges if this upstream step sets the right kind.
    use graphengine_parsing::application::ports::TypeUsageKind;
    let type_refs = &hints.type_references;
    assert!(
        type_refs
            .iter()
            .any(|t| t.type_name == "AccountRepositoryBase"
                && t.usage_kind == TypeUsageKind::Extends),
        "upstream TypeUsageKind::Extends missing for AccountRepositoryBase; got {type_refs:?}"
    );
    assert!(
        type_refs
            .iter()
            .any(|t| t.type_name == "IAccountRepository"
                && t.usage_kind == TypeUsageKind::Implements),
        "upstream TypeUsageKind::Implements missing for IAccountRepository; got {type_refs:?}"
    );

    let resolver = ApexHeuristicResolver::with_standard_preload_only();
    let out = resolver.resolve(&hints).await.expect("resolve");

    // ---- Extends: MyAccountRepo -> AccountRepositoryBase ---------------
    let concrete_id = hints
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Struct && n.fqn.ends_with("MyAccountRepo"))
        .expect("concrete node")
        .id
        .clone();
    let base_id = hints
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Struct && n.fqn.ends_with("AccountRepositoryBase"))
        .expect("base node")
        .id
        .clone();
    let iface_id = hints
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Interface && n.fqn.ends_with("IAccountRepository"))
        .expect("iface node")
        .id
        .clone();

    let has_edge = |kind: EdgeKind, from: &str, to: &str| -> bool {
        out.type_edges
            .iter()
            .any(|e| e.kind == kind && e.from_id == from && e.to_id == to)
    };
    assert!(
        has_edge(EdgeKind::Extends, &concrete_id, &base_id),
        "missing Extends edge MyAccountRepo -> AccountRepositoryBase; type_edges: {:?}",
        out.type_edges
            .iter()
            .map(|e| (e.kind, e.from_id.as_str(), e.to_id.as_str()))
            .collect::<Vec<_>>()
    );
    assert!(
        has_edge(EdgeKind::Implements, &concrete_id, &iface_id),
        "missing Implements edge MyAccountRepo -> IAccountRepository; type_edges: {:?}",
        out.type_edges
            .iter()
            .map(|e| (e.kind, e.from_id.as_str(), e.to_id.as_str()))
            .collect::<Vec<_>>()
    );

    // Regression guard: inheritance must NOT appear as the legacy
    // collapsed `Type` edge. If any Type edge sits between these three
    // nodes, extract_inheritance has started spraying legacy kinds again.
    for e in &out.type_edges {
        if e.kind == EdgeKind::Type
            && (e.from_id == concrete_id)
            && (e.to_id == base_id || e.to_id == iface_id)
        {
            panic!(
                "Sprint E.1 regression: inheritance edge emitted as EdgeKind::Type \
                 instead of Extends/Implements — {:?}",
                e
            );
        }
    }
}
