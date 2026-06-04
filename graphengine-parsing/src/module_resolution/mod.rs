pub mod types;

mod bindings;
mod graph;
mod use_tree;

use std::collections::HashSet;

use crate::application::ports::SyntaxResults;

pub use bindings::NameBindingTable;
pub use graph::ModuleGraph;
pub use types::{BindingOrigin, ConfidenceWeight, ModulePath, ResolvedName};
pub use use_tree::UseTree;

#[derive(Debug, Clone)]
pub struct ModuleResolver {
    module_graph: ModuleGraph,
    pub(crate) name_bindings: NameBindingTable,
}

impl ModuleResolver {
    pub fn from_syntax(syntax: &SyntaxResults) -> Self {
        let module_graph = ModuleGraph::build(syntax);
        let use_tree = UseTree::build(syntax);
        let name_bindings = NameBindingTable::build(syntax, &module_graph, &use_tree);

        Self {
            module_graph,
            name_bindings,
        }
    }

    pub fn module_path_for_file(&self, file_path: &str) -> Option<&ModulePath> {
        self.module_graph.module_path_for_file(file_path)
    }

    pub fn shared_prefix_len_between(&self, a: &str, b: &str) -> usize {
        self.module_graph.shared_prefix_len_between(a, b)
    }

    pub fn resolve_name_in_context(
        &self,
        context_file: &str,
        raw_target: &str,
    ) -> Vec<ResolvedName> {
        let normalized = normalize_target_name(raw_target);
        let mut results: Vec<ResolvedName> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        if normalized.contains("::") {
            let explicit = self.resolve_explicit_path(context_file, &normalized);
            for candidate in explicit {
                if seen.insert(candidate.fqn.clone()) {
                    results.push(candidate);
                }
            }
        }

        let simple = normalized
            .rsplit("::")
            .next()
            .unwrap_or(normalized.as_str());

        let candidates = self.name_bindings.function_candidates(context_file, simple);
        for candidate in candidates {
            if seen.insert(candidate.fqn.clone()) {
                results.push(candidate.clone());
            }
        }

        results.sort_by(compare_candidates);
        results
    }

    fn resolve_explicit_path(&self, context_file: &str, normalized: &str) -> Vec<ResolvedName> {
        let segments: Vec<&str> = normalized.split("::").collect();
        if segments.is_empty() {
            return Vec::new();
        }

        let context_path = self
            .module_graph
            .module_path_for_file(context_file)
            .cloned()
            .unwrap_or_default();

        let mut index = 0usize;
        let base_origin_path = context_root(&context_path);

        let (base_path, origin) = if let Some(alias_path) =
            self.name_bindings.module_binding(context_file, segments[0])
        {
            index = 1;
            (alias_path.clone(), BindingOrigin::Alias)
        } else if let Some(candidates) = self
            .name_bindings
            .function_candidates(context_file, segments[0])
            .first()
        {
            // Check if the first segment is actually a function binding that points to a module
            // This handles cases like `nested` which might be re-exported as a function binding
            // but we need to resolve it as a module path
            // For now, try to construct the path from the candidate's FQN
            // This is a fallback - ideally module bindings should be created for glob imports
            let candidate_fqn = &candidates.fqn;
            let candidate_segments: Vec<&str> = candidate_fqn.split("::").collect();
            if candidate_segments.len() > 1 {
                // The module path is all segments except the last (which is the function name)
                let module_segments: Vec<String> = candidate_segments
                    [..candidate_segments.len() - 1]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                index = 1;
                (
                    ModulePath::from_segments(module_segments),
                    BindingOrigin::Alias,
                )
            } else {
                // Fall through to normal path resolution
                match segments.first().copied().unwrap_or("") {
                    "self" => {
                        index = 1;
                        (context_path.clone(), BindingOrigin::SelfPath)
                    }
                    "super" => {
                        let mut depth = 0u8;
                        while index < segments.len() && segments[index] == "super" {
                            depth = depth.saturating_add(1);
                            index += 1;
                        }
                        (
                            ascend(&context_path, depth),
                            BindingOrigin::SuperPath(depth),
                        )
                    }
                    "crate" => {
                        index = 1;
                        (base_origin_path.clone(), BindingOrigin::CratePath)
                    }
                    "" => {
                        index = 1;
                        (base_origin_path.clone(), BindingOrigin::CratePath)
                    }
                    _ => (context_path.clone(), BindingOrigin::ExternalCrate),
                }
            }
        } else {
            match segments.first().copied().unwrap_or("") {
                "self" => {
                    index = 1;
                    (context_path.clone(), BindingOrigin::SelfPath)
                }
                "super" => {
                    let mut depth = 0u8;
                    while index < segments.len() && segments[index] == "super" {
                        depth = depth.saturating_add(1);
                        index += 1;
                    }
                    (
                        ascend(&context_path, depth),
                        BindingOrigin::SuperPath(depth),
                    )
                }
                "crate" => {
                    index = 1;
                    (base_origin_path.clone(), BindingOrigin::CratePath)
                }
                "" => {
                    index = 1;
                    (base_origin_path.clone(), BindingOrigin::CratePath)
                }
                _ => (context_path.clone(), BindingOrigin::ExternalCrate),
            }
        };

        if segments.len() <= index {
            return Vec::new();
        }

        let mut module_segments = base_path.segments().to_vec();
        if segments.len() > index + 1 {
            module_segments.extend(
                segments[index..segments.len() - 1]
                    .iter()
                    .map(|segment| segment.to_string()),
            );
        }
        let module_path = ModulePath::from_segments(module_segments.clone());

        if self.module_graph.node(&module_path).is_none() {
            return Vec::new();
        }

        let simple_name = segments.last().unwrap().to_string();
        module_segments.push(simple_name.clone());
        let fqn = module_segments.join("::");

        vec![ResolvedName::new(
            fqn,
            module_path,
            origin,
            ConfidenceWeight::High,
        )]
    }
}

fn normalize_target_name(name: &str) -> String {
    let mut trimmed = name.trim();

    if let Some(paren_pos) = trimmed.find('(') {
        trimmed = &trimmed[..paren_pos];
    }

    trimmed = trimmed.trim();

    if let Some(generic_pos) = trimmed.find('<') {
        trimmed = &trimmed[..generic_pos];
    }

    trimmed
        .trim()
        .trim_end_matches(':')
        .trim_end_matches(':')
        .to_string()
}

fn compare_candidates(a: &ResolvedName, b: &ResolvedName) -> std::cmp::Ordering {
    let confidence_rank = |weight: ConfidenceWeight| match weight {
        ConfidenceWeight::High => 0,
        ConfidenceWeight::Medium => 1,
        ConfidenceWeight::Low => 2,
    };

    let origin_rank = |origin: BindingOrigin| match origin {
        BindingOrigin::Local => 0,
        BindingOrigin::Alias => 1,
        BindingOrigin::SelfPath => 2,
        BindingOrigin::SuperPath(_) => 3,
        BindingOrigin::CratePath => 4,
        BindingOrigin::Glob => 5,
        BindingOrigin::ExternalCrate => 6,
        BindingOrigin::Unresolved => 7,
    };

    confidence_rank(a.confidence)
        .cmp(&confidence_rank(b.confidence))
        .then_with(|| origin_rank(a.origin).cmp(&origin_rank(b.origin)))
}

fn context_root(path: &ModulePath) -> ModulePath {
    if let Some(first) = path.segments().first() {
        ModulePath::from_segments(vec![first.clone()])
    } else {
        ModulePath::new()
    }
}

fn ascend(path: &ModulePath, depth: u8) -> ModulePath {
    let mut segments = path.segments().to_vec();
    let steps = depth.min(segments.len() as u8) as usize;
    for _ in 0..steps {
        segments.pop();
    }
    ModulePath::from_segments(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::{
        ImportKind, ImportPath, ImportSpec, ImportVisibility, ModDecl, ModKind, PathRoot,
        SyntaxResults,
    };
    use crate::domain::{Node, Range};
    use tempfile::tempdir;

    fn setup_crate() -> (tempfile::TempDir, String, String, String) {
        let temp = tempdir().unwrap();
        let crate_dir = temp.path().to_path_buf();
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname=\"resolver_test\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();

        let lib_path = crate_dir.join("src/lib.rs");
        std::fs::write(&lib_path, "pub mod foo;\n").unwrap();
        let nested_path = crate_dir.join("src/foo.rs");
        std::fs::write(&nested_path, "pub fn call_me() -> i32 { 1 }\n").unwrap();

        let lib_path = std::fs::canonicalize(lib_path)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let nested_path = std::fs::canonicalize(nested_path)
            .unwrap()
            .to_string_lossy()
            .to_string();

        let crate_name = crate_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('-', "_");

        (temp, crate_name, lib_path, nested_path)
    }

    #[test]
    fn resolves_aliases_and_explicit_paths() {
        let (_temp, crate_name, lib_path, nested_path) = setup_crate();

        let mut syntax = SyntaxResults::new();
        syntax.add_mod_decl(ModDecl {
            name: "foo".into(),
            source_file: lib_path.clone(),
            range: Range::with_file(1, 0, 1, 10, lib_path.clone()),
            kind: ModKind::External,
            resolved_file: Some(nested_path.clone()),
        });

        let function_fqn = format!("{}::foo::call_me", crate_name);
        syntax.add_symbol(Node::function(
            function_fqn.clone(),
            Range::with_file(1, 0, 1, 25, nested_path.clone()),
        ));

        syntax.add_import_spec(ImportSpec {
            range: Range::with_file(2, 0, 2, 25, lib_path.clone()),
            path: ImportPath::new(PathRoot::Crate, vec!["foo".into(), "call_me".into()]),
            alias: Some("alias_call".into()),
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: false,
            source_file: lib_path.clone(),
        });

        let resolver = ModuleResolver::from_syntax(&syntax);
        let alias = resolver.resolve_name_in_context(&lib_path, "alias_call");
        assert_eq!(alias.len(), 1);
        assert_eq!(alias[0].fqn, function_fqn);
        assert!(matches!(alias[0].origin, BindingOrigin::Alias));

        let explicit = resolver.resolve_name_in_context(&lib_path, "crate::foo::call_me");
        assert_eq!(explicit.len(), 1);
        assert_eq!(explicit[0].fqn, function_fqn);
        assert!(matches!(explicit[0].origin, BindingOrigin::CratePath));
    }

    #[test]
    fn resolves_glob_imports() {
        let (_temp, crate_name, lib_path, nested_path) = setup_crate();

        let mut syntax = SyntaxResults::new();
        syntax.add_mod_decl(ModDecl {
            name: "foo".into(),
            source_file: lib_path.clone(),
            range: Range::with_file(1, 0, 1, 10, lib_path.clone()),
            kind: ModKind::External,
            resolved_file: Some(nested_path.clone()),
        });

        let function_fqn = format!("{}::foo::call_me", crate_name);
        syntax.add_symbol(Node::function(
            function_fqn.clone(),
            Range::with_file(1, 0, 1, 25, nested_path.clone()),
        ));

        syntax.add_import_spec(ImportSpec {
            range: Range::with_file(2, 0, 2, 15, lib_path.clone()),
            path: ImportPath::new(PathRoot::Crate, vec!["foo".into()]),
            alias: None,
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: true,
            source_file: lib_path.clone(),
        });

        let resolver = ModuleResolver::from_syntax(&syntax);
        let glob_candidates = resolver.resolve_name_in_context(&lib_path, "call_me");
        assert!(!glob_candidates.is_empty());
        assert!(glob_candidates
            .iter()
            .any(|c| c.fqn == function_fqn && matches!(c.origin, BindingOrigin::Glob)));
    }

    #[test]
    fn resolves_self_path() {
        let (_temp, crate_name, lib_path, nested_path) = setup_crate();

        let mut syntax = SyntaxResults::new();
        syntax.add_mod_decl(ModDecl {
            name: "foo".into(),
            source_file: lib_path.clone(),
            range: Range::with_file(1, 0, 1, 10, lib_path.clone()),
            kind: ModKind::External,
            resolved_file: Some(nested_path.clone()),
        });

        let function_fqn = format!("{}::foo::call_me", crate_name);
        syntax.add_symbol(Node::function(
            function_fqn.clone(),
            Range::with_file(1, 0, 1, 25, nested_path.clone()),
        ));

        let resolver = ModuleResolver::from_syntax(&syntax);
        let self_path = resolver.resolve_name_in_context(&nested_path, "self::call_me");
        assert!(!self_path.is_empty());
        assert!(self_path
            .iter()
            .any(|c| c.fqn == function_fqn && matches!(c.origin, BindingOrigin::SelfPath)));
    }

    #[test]
    fn resolves_super_path_with_depth() {
        let temp = tempdir().unwrap();
        let crate_dir = temp.path().to_path_buf();
        std::fs::create_dir_all(crate_dir.join("src/nested")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname=\"depth_test\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();

        let lib_path = crate_dir.join("src/lib.rs");
        std::fs::write(&lib_path, "pub mod nested;\n").unwrap();
        let nested_path = crate_dir.join("src/nested/mod.rs");
        std::fs::write(&nested_path, "pub fn root_fn() -> i32 { 1 }\n").unwrap();

        let lib_path = std::fs::canonicalize(lib_path)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let nested_path = std::fs::canonicalize(nested_path)
            .unwrap()
            .to_string_lossy()
            .to_string();

        let crate_name = crate_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('-', "_");

        let mut syntax = SyntaxResults::new();
        syntax.add_mod_decl(ModDecl {
            name: "nested".into(),
            source_file: lib_path.clone(),
            range: Range::with_file(1, 0, 1, 12, lib_path.clone()),
            kind: ModKind::External,
            resolved_file: Some(nested_path.clone()),
        });

        let root_fqn = format!("{}::root_fn", crate_name);
        syntax.add_symbol(Node::function(
            root_fqn.clone(),
            Range::with_file(1, 0, 1, 20, lib_path.clone()),
        ));

        let resolver = ModuleResolver::from_syntax(&syntax);
        let super_path = resolver.resolve_name_in_context(&nested_path, "super::root_fn");
        assert!(!super_path.is_empty());
        assert!(super_path
            .iter()
            .any(|c| matches!(c.origin, BindingOrigin::SuperPath(1))));
    }

    #[test]
    fn returns_empty_for_unresolvable_imports() {
        let (_temp, _crate_name, lib_path, _nested_path) = setup_crate();

        let syntax = SyntaxResults::new();
        let resolver = ModuleResolver::from_syntax(&syntax);

        let unresolved = resolver.resolve_name_in_context(&lib_path, "nonexistent::function");
        assert!(unresolved.is_empty());
    }

    #[test]
    fn resolves_nested_module_graphs() {
        let temp = tempdir().unwrap();
        let crate_dir = temp.path().to_path_buf();
        std::fs::create_dir_all(crate_dir.join("src/a/b")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname=\"nested_test\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();

        let lib_path = crate_dir.join("src/lib.rs");
        std::fs::write(&lib_path, "pub mod a;\n").unwrap();
        let a_path = crate_dir.join("src/a/mod.rs");
        std::fs::write(&a_path, "pub mod b;\n").unwrap();
        let b_path = crate_dir.join("src/a/b.rs");
        std::fs::write(&b_path, "pub fn deep() -> i32 { 42 }\n").unwrap();

        let lib_path = std::fs::canonicalize(lib_path)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let a_path = std::fs::canonicalize(a_path)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let b_path = std::fs::canonicalize(b_path)
            .unwrap()
            .to_string_lossy()
            .to_string();

        let crate_name = crate_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('-', "_");

        let mut syntax = SyntaxResults::new();
        syntax.add_mod_decl(ModDecl {
            name: "a".into(),
            source_file: lib_path.clone(),
            range: Range::with_file(1, 0, 1, 8, lib_path.clone()),
            kind: ModKind::External,
            resolved_file: Some(a_path.clone()),
        });
        syntax.add_mod_decl(ModDecl {
            name: "b".into(),
            source_file: a_path.clone(),
            range: Range::with_file(1, 0, 1, 8, a_path.clone()),
            kind: ModKind::External,
            resolved_file: Some(b_path.clone()),
        });

        let deep_fqn = format!("{}::a::b::deep", crate_name);
        syntax.add_symbol(Node::function(
            deep_fqn.clone(),
            Range::with_file(1, 0, 1, 20, b_path.clone()),
        ));

        let resolver = ModuleResolver::from_syntax(&syntax);
        let resolved = resolver.resolve_name_in_context(&lib_path, "a::b::deep");
        assert!(!resolved.is_empty());
        assert!(resolved.iter().any(|c| c.fqn == deep_fqn));
    }

    #[test]
    fn prefers_local_over_imported_bindings() {
        let (_temp, crate_name, lib_path, nested_path) = setup_crate();

        let mut syntax = SyntaxResults::new();
        syntax.add_mod_decl(ModDecl {
            name: "foo".into(),
            source_file: lib_path.clone(),
            range: Range::with_file(1, 0, 1, 10, lib_path.clone()),
            kind: ModKind::External,
            resolved_file: Some(nested_path.clone()),
        });

        let local_fqn = format!("{}::local_fn", crate_name);
        let imported_fqn = format!("{}::foo::call_me", crate_name);

        syntax.add_symbol(Node::function(
            local_fqn.clone(),
            Range::with_file(10, 0, 10, 20, lib_path.clone()),
        ));
        syntax.add_symbol(Node::function(
            imported_fqn.clone(),
            Range::with_file(1, 0, 1, 25, nested_path.clone()),
        ));

        syntax.add_import_spec(ImportSpec {
            range: Range::with_file(2, 0, 2, 25, lib_path.clone()),
            path: ImportPath::new(PathRoot::Crate, vec!["foo".into(), "call_me".into()]),
            alias: Some("local_fn".into()),
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: false,
            source_file: lib_path.clone(),
        });

        let resolver = ModuleResolver::from_syntax(&syntax);
        let candidates = resolver.resolve_name_in_context(&lib_path, "local_fn");
        assert!(!candidates.is_empty());
        let local_candidate = candidates
            .iter()
            .find(|c| c.fqn == local_fqn && matches!(c.origin, BindingOrigin::Local));
        assert!(local_candidate.is_some());
    }
}
