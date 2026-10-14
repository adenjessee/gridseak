//! Rust through the SCIP reference-occurrence sweep, on real tree-sitter
//! nodes and a committed `rust-analyzer scip` index
//! (`tests/fixtures/rust_scip_calls`, regenerate with
//! `rust-analyzer scip . --output index.scip` in that directory).

#![cfg(feature = "scip")]

use std::collections::BTreeMap;
use std::path::PathBuf;

use graphengine_parsing::application::ports::{SemanticResolver, SyntaxExtractor};
use graphengine_parsing::domain::{EdgeKind, NodeKind, ProvenanceSource, UseKind};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::infrastructure::{IndexBackedSemanticResolver, ScipSemanticIndex};
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_scip_calls")
}

/// `(caller, callee) -> via` with FQNs trimmed to their last two segments.
async fn edges() -> BTreeMap<(String, String), Option<UseKind>> {
    let root = root();
    let files = vec![root.join("src/lib.rs"), root.join("src/shapes.rs")];
    let mut extractor = TreeSitterExtractor::new(load_config("rust").unwrap()).unwrap();
    extractor.set_workspace_root(root.to_string_lossy().to_string());
    let mut hints = extractor.extract(&files).await.unwrap();
    hints.language = Some("rust".into());
    let index =
        ScipSemanticIndex::from_file_at_workspace(root.join("index.scip"), root.clone(), "rust")
            .unwrap();
    let resolver = IndexBackedSemanticResolver::new(index, root.clone(), "rust");
    let out = resolver.resolve(&hints).await.unwrap();
    let name = |id: &str| {
        let n = hints.symbols.iter().find(|n| n.id == id).unwrap();
        let parts: Vec<&str> = n.fqn.rsplit("::").take(2).collect();
        format!("{}::{}", parts[1], parts[0])
    };
    let mut map = BTreeMap::new();
    for e in &out.call_edges {
        assert_eq!(e.kind, EdgeKind::Call);
        assert_eq!(e.provenance.source, ProvenanceSource::Compiler);
        map.insert((name(&e.from_id), name(&e.to_id)), e.provenance.via);
    }
    map
}

#[tokio::test]
async fn methods_are_named_by_their_impl_type() {
    let root = root();
    let mut extractor = TreeSitterExtractor::new(load_config("rust").unwrap()).unwrap();
    extractor.set_workspace_root(root.to_string_lossy().to_string());
    let hints = extractor
        .extract(&[root.join("src/shapes.rs"), root.join("src/lib.rs")])
        .await
        .unwrap();
    let fqns: Vec<&str> = hints
        .symbols
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .map(|n| n.fqn.as_str())
        .collect();
    for want in [
        "Circle::is_empty",
        "Square::is_empty",
        "Circle::area",
        "Square::area",
        "Area::twice",
        "tests::calls_direct",
    ] {
        assert!(
            fqns.iter().any(|f| f.ends_with(want)),
            "{want} missing from {fqns:?}"
        );
    }
    // Same body, different types: two nodes, not one.
    let empties: std::collections::HashSet<&str> = hints
        .symbols
        .iter()
        .filter(|n| n.fqn.ends_with("::is_empty"))
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(empties.len(), 2);
}

#[tokio::test]
async fn rust_call_shapes_resolve_like_the_compiler() {
    let e = edges().await;
    let has = |from: &str, to: &str| e.get(&(from.to_string(), to.to_string())).copied();
    let call = Some(Some(UseKind::Call));
    let dispatch = Some(Some(UseKind::Dispatch));

    assert_eq!(
        has("rust_scip_calls::direct", "rust_scip_calls::helper"),
        call
    );
    // Method call on a concrete type: the impl's method, not every `area`.
    assert_eq!(has("rust_scip_calls::method_call", "Circle::area"), call);
    assert_eq!(has("rust_scip_calls::method_call", "Square::area"), None);
    assert_eq!(has("rust_scip_calls::method_call", "Circle::new"), call);
    // UFCS, both `Type::m(x)` and `<Type as Trait>::m(x)`.
    assert_eq!(has("rust_scip_calls::ufcs", "Circle::area"), call);
    assert_eq!(has("rust_scip_calls::ufcs", "Square::area"), call);
    // Through `dyn Trait` / a generic bound: dispatch to every impl.
    for f in ["dyn_dispatch", "generic_bound"] {
        let from = format!("rust_scip_calls::{f}");
        assert_eq!(has(&from, "Circle::area"), dispatch, "{f}");
        assert_eq!(has(&from, "Square::area"), dispatch, "{f}");
    }
    // Default trait method through `dyn`: the default body is a dispatch target.
    assert_eq!(has("rust_scip_calls::twice_dyn", "Area::twice"), dispatch);
    // Inside the default body, `self.area()` dispatches too.
    assert_eq!(has("Area::twice", "Circle::area"), dispatch);
    // Calls inside a closure belong to the enclosing fn.
    assert_eq!(
        has("rust_scip_calls::in_closure", "rust_scip_calls::helper"),
        call
    );
    // Calls written in macro arguments (`format!`, `vec!`, user macros) count.
    assert_eq!(
        has("rust_scip_calls::in_macro", "rust_scip_calls::helper"),
        call
    );
    // A call produced only by a user macro's body is not in the index.
    assert_eq!(
        has("rust_scip_calls::via_user_macro", "rust_scip_calls::helper"),
        None
    );
    // Turbofish is still a call.
    assert_eq!(
        has("rust_scip_calls::turbofish", "rust_scip_calls::generic"),
        call
    );
    // A function passed as a value.
    assert_eq!(
        has("rust_scip_calls::value_use", "rust_scip_calls::helper"),
        Some(Some(UseKind::Value))
    );
    assert_eq!(
        has("rust_scip_calls::iter_map", "rust_scip_calls::helper"),
        Some(Some(UseKind::Value))
    );
    // Same-named inherent methods of different types stay apart.
    assert_eq!(has("rust_scip_calls::empties", "Circle::is_empty"), call);
    assert_eq!(has("rust_scip_calls::empties", "Square::is_empty"), call);
    // Tests in `#[cfg(test)] mod tests` are callers.
    assert_eq!(has("tests::calls_direct", "rust_scip_calls::direct"), call);
    // `#[derive]`-generated impls have no node: no edge from `p.clone()`.
    assert!(
        !e.keys()
            .any(|(from, _)| from == "rust_scip_calls::uses_derive"),
        "{e:?}"
    );
    // `use` declarations are not calls.
    assert!(
        !e.keys().any(|(from, _)| from.ends_with("__file_module__")),
        "{e:?}"
    );
}

/// No SCIP index: the scan falls back to the in-process rust-analyzer
/// resolver and the disclosure keeps the index skip reason (which the
/// report turns into `compiler_tier_unavailable_v1`).
#[cfg(feature = "rust-layer2")]
#[tokio::test]
async fn missing_index_falls_back_to_ra_ide_and_says_so() {
    use graphengine_parsing::application::resolution_disclosure::{ResolutionTierKind, SkipReason};
    use graphengine_parsing::application::use_cases::parse_repo::factory::UseCaseFactory;
    use graphengine_parsing::domain::Confidence;

    if graphengine_scip_adapter::provisioning::auto_provision_enabled() {
        return; // would run rust-analyzer scip
    }
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\n[package]\nname = \"t\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    let db = tempfile::NamedTempFile::new().unwrap();
    let use_case = UseCaseFactory::with_real_components(
        "rust".into(),
        Confidence::Low,
        db.path().to_str().unwrap(),
        Some(url::Url::from_directory_path(dir.path()).unwrap()),
    )
    .await
    .expect("factory");
    let d = use_case
        .semantic_resolver()
        .resolution_disclosure()
        .await
        .expect("disclosure");
    if d.tier_used == ResolutionTierKind::SubprocessLsp {
        return; // rust-layer2 could not load this machine's sysroot
    }
    assert_eq!(d.tier_attempted, ResolutionTierKind::Layer2);
    assert_eq!(d.tier_used, ResolutionTierKind::Layer2);
    assert_eq!(d.skip_reason, Some(SkipReason::IndexerNotProvisioned));
}
