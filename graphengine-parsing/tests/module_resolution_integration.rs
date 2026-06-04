use graphengine_parsing::application::ports::{
    ImportKind, ImportPath, ImportSpec, ImportVisibility, ModDecl, ModKind, PathRoot, SyntaxResults,
};
use graphengine_parsing::domain::{Node, Range};
use graphengine_parsing::module_resolution::ModuleResolver;
use graphengine_parsing::symbol_index::SymbolIndex;
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn resolves_imports_with_module_resolver() -> anyhow::Result<()> {
    let temp = tempdir()?;
    let crate_dir = temp.path().to_path_buf();
    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname=\"module_resolution_fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();

    let lib_path = crate_dir.join("src/lib.rs");
    std::fs::write(&lib_path, "pub mod foo;\n").unwrap();
    let foo_path = crate_dir.join("src/foo.rs");
    std::fs::write(
        &foo_path,
        "pub fn function_b() -> i32 { 3 }\npub fn helper() -> i32 { 4 }\n",
    )
    .unwrap();

    let lib_path = std::fs::canonicalize(lib_path)?
        .to_string_lossy()
        .to_string();
    let foo_path = std::fs::canonicalize(foo_path)?
        .to_string_lossy()
        .to_string();
    let crate_name = crate_dir.file_name().unwrap().to_string_lossy().to_string();

    let mut syntax = SyntaxResults::new();
    syntax.add_mod_decl(ModDecl {
        name: "foo".into(),
        source_file: lib_path.clone(),
        range: Range::with_file(1, 0, 1, 10, lib_path.clone()),
        kind: ModKind::External,
        resolved_file: Some(foo_path.clone()),
    });

    let function_fqn = format!("{}::foo::function_b", crate_name);
    syntax.add_symbol(Node::function(
        function_fqn.clone(),
        Range::with_file(1, 0, 1, 20, foo_path.clone()),
    ));
    let helper_fqn = format!("{}::foo::helper", crate_name);
    syntax.add_symbol(Node::function(
        helper_fqn.clone(),
        Range::with_file(2, 0, 2, 20, foo_path.clone()),
    ));

    syntax.add_import_spec(ImportSpec {
        range: Range::with_file(2, 0, 2, 25, lib_path.clone()),
        path: ImportPath::new(PathRoot::Crate, vec!["foo".into(), "function_b".into()]),
        alias: Some("beta_alias".into()),
        visibility: ImportVisibility::Private,
        kind: ImportKind::Use,
        is_glob: false,
        source_file: lib_path.clone(),
    });

    syntax.add_import_spec(ImportSpec {
        range: Range::with_file(3, 0, 3, 15, lib_path.clone()),
        path: ImportPath::new(PathRoot::Crate, vec!["foo".into()]),
        alias: None,
        visibility: ImportVisibility::Private,
        kind: ImportKind::Use,
        is_glob: true,
        source_file: lib_path.clone(),
    });

    let resolver = ModuleResolver::from_syntax(&syntax);
    let symbol_index = SymbolIndex::from_syntax(&syntax);

    let beta_alias = resolver.resolve_name_in_context(&lib_path, "beta_alias");
    assert!(!beta_alias.is_empty());
    assert_eq!(beta_alias[0].fqn, function_fqn);

    let helper_candidates = resolver.resolve_name_in_context(&lib_path, "helper");
    assert!(!helper_candidates.is_empty());
    assert!(helper_candidates
        .iter()
        .any(|candidate| candidate.fqn == helper_fqn));

    let resolved_symbol = symbol_index
        .resolve_function("beta_alias", &lib_path, &resolver)
        .expect("beta_alias resolved");
    assert_eq!(resolved_symbol.record.fqn, function_fqn);

    let explicit = resolver.resolve_name_in_context(&lib_path, "crate::foo::function_b");
    assert!(!explicit.is_empty());
    assert_eq!(explicit[0].fqn, function_fqn);

    Ok(())
}

#[test]
fn regression_fixture_validates_canonical_fqns() -> anyhow::Result<()> {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("function-relationship-test");

    if !fixture_root.exists() {
        eprintln!("Skipping regression test: fixture crate not found");
        return Ok(());
    }

    let crate_name = fixture_root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();

    let lib_path = fixture_root.join("src/lib.rs").canonicalize()?;
    let nested_mod_path = fixture_root.join("src/nested/mod.rs").canonicalize()?;
    let alpha_path = fixture_root.join("src/nested/alpha.rs").canonicalize()?;
    let beta_path = fixture_root.join("src/nested/beta.rs").canonicalize()?;

    let lib_path_str = lib_path.to_string_lossy().to_string();
    let nested_mod_path_str = nested_mod_path.to_string_lossy().to_string();
    let alpha_path_str = alpha_path.to_string_lossy().to_string();
    let beta_path_str = beta_path.to_string_lossy().to_string();

    let mut syntax = SyntaxResults::new();

    // Module declarations mirroring the fixture layout
    syntax.add_mod_decl(ModDecl {
        name: "nested".into(),
        source_file: lib_path_str.clone(),
        range: Range::with_file(1, 0, 20, 0, lib_path_str.clone()),
        kind: ModKind::External,
        resolved_file: Some(nested_mod_path_str.clone()),
    });
    syntax.add_mod_decl(ModDecl {
        name: "alpha".into(),
        source_file: nested_mod_path_str.clone(),
        range: Range::with_file(1, 0, 20, 0, nested_mod_path_str.clone()),
        kind: ModKind::External,
        resolved_file: Some(alpha_path_str.clone()),
    });
    syntax.add_mod_decl(ModDecl {
        name: "beta".into(),
        source_file: nested_mod_path_str.clone(),
        range: Range::with_file(1, 0, 20, 0, nested_mod_path_str.clone()),
        kind: ModKind::External,
        resolved_file: Some(beta_path_str.clone()),
    });
    syntax.add_mod_decl(ModDecl {
        name: "deep".into(),
        source_file: alpha_path_str.clone(),
        range: Range::with_file(5, 0, 15, 0, alpha_path_str.clone()),
        kind: ModKind::Inline,
        resolved_file: Some(alpha_path_str.clone()),
    });

    // Symbols from the fixture
    let alpha_base_fqn = format!("{}::nested::alpha::alpha_base", crate_name);
    syntax.add_symbol(Node::function(
        alpha_base_fqn.clone(),
        Range::with_file(1, 0, 3, 0, alpha_path_str.clone()),
    ));

    let deep_alpha_fqn = format!("{}::nested::alpha::deep::deep_alpha_call", crate_name);
    syntax.add_symbol(Node::function(
        deep_alpha_fqn.clone(),
        Range::with_file(6, 0, 10, 0, alpha_path_str.clone()),
    ));

    let beta_helper_fqn = format!("{}::nested::beta::beta_helper", crate_name);
    syntax.add_symbol(Node::function(
        beta_helper_fqn.clone(),
        Range::with_file(1, 0, 5, 0, beta_path_str.clone()),
    ));

    // Import specifications reflecting aliasing and globs
    syntax.add_import_spec(ImportSpec {
        range: Range::with_file(2, 0, 2, 40, nested_mod_path_str.clone()),
        path: ImportPath::new(
            PathRoot::SelfPath,
            vec!["alpha".into(), "alpha_base".into()],
        ),
        alias: Some("alias_alpha_base".into()),
        visibility: ImportVisibility::Pub,
        kind: ImportKind::Reexport,
        is_glob: false,
        source_file: nested_mod_path_str.clone(),
    });
    syntax.add_import_spec(ImportSpec {
        range: Range::with_file(3, 0, 3, 40, nested_mod_path_str.clone()),
        path: ImportPath::new(
            PathRoot::SelfPath,
            vec!["alpha".into(), "deep".into(), "deep_alpha_call".into()],
        ),
        alias: None,
        visibility: ImportVisibility::Pub,
        kind: ImportKind::Reexport,
        is_glob: false,
        source_file: nested_mod_path_str.clone(),
    });
    syntax.add_import_spec(ImportSpec {
        range: Range::with_file(4, 0, 4, 40, nested_mod_path_str.clone()),
        path: ImportPath::new(PathRoot::SelfPath, vec!["beta".into()]),
        alias: None,
        visibility: ImportVisibility::Pub,
        kind: ImportKind::Reexport,
        is_glob: true,
        source_file: nested_mod_path_str.clone(),
    });

    let resolver = ModuleResolver::from_syntax(&syntax);
    let symbol_index = SymbolIndex::from_syntax(&syntax);

    // Alias defined in nested::mod
    let alias_resolution =
        resolver.resolve_name_in_context(&nested_mod_path_str, "alias_alpha_base");
    assert!(
        alias_resolution
            .iter()
            .any(|candidate| candidate.fqn == alpha_base_fqn),
        "alias should map to alpha_base"
    );

    // Glob import for beta helper should resolve canonical FQN
    let beta_helper_resolution =
        resolver.resolve_name_in_context(&nested_mod_path_str, "beta_helper");
    assert!(
        beta_helper_resolution
            .iter()
            .any(|candidate| candidate.fqn == beta_helper_fqn),
        "glob import should expose beta_helper"
    );

    // Verify SymbolIndex integration uses canonical FQNs
    let resolved_symbol = symbol_index
        .resolve_function("alias_alpha_base", &nested_mod_path_str, &resolver)
        .expect("symbol index should resolve alias");
    assert_eq!(resolved_symbol.record.fqn, alpha_base_fqn);

    Ok(())
}
