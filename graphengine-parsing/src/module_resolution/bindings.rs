use std::collections::{HashMap, HashSet};

use crate::application::ports::{PathRoot, SyntaxResults};
use crate::domain::NodeKind;

use super::graph::{ModuleGraph, ModuleNodeKind};
use super::types::{BindingOrigin, ConfidenceWeight, ModulePath, ResolvedName};
use super::use_tree::{ImportBinding, UseTree};

#[derive(Debug, Default, Clone)]
pub struct FileBindings {
    function_bindings: HashMap<String, Vec<ResolvedName>>,
    module_bindings: HashMap<String, ModulePath>,
}

#[derive(Debug, Default, Clone)]
pub struct NameBindingTable {
    per_file: HashMap<String, FileBindings>,
}

#[derive(Debug, Clone)]
struct SymbolInfo {
    fqn: String,
    module_path: ModulePath,
    simple_name: String,
}

impl NameBindingTable {
    pub fn build(syntax: &SyntaxResults, graph: &ModuleGraph, use_tree: &UseTree) -> Self {
        tracing::info!(
            "[BINDINGS] Starting NameBindingTable::build with {} import_specs",
            syntax.import_specs.len()
        );
        let (symbols_by_file, symbols_by_module) = collect_symbols(syntax);

        let mut table = NameBindingTable::default();

        let mut file_set: HashSet<String> = symbols_by_file.keys().cloned().collect();
        for file in use_tree.files() {
            file_set.insert(file.clone());
        }

        tracing::info!("[BINDINGS] Processing {} files in file_set", file_set.len());
        for file in file_set {
            let mut bindings = FileBindings::default();

            if let Some(symbols) = symbols_by_file.get(&file) {
                for symbol in symbols {
                    let resolved = ResolvedName::new(
                        symbol.fqn.clone(),
                        symbol.module_path.clone(),
                        BindingOrigin::Local,
                        ConfidenceWeight::High,
                    );
                    bindings
                        .function_bindings
                        .entry(symbol.simple_name.clone())
                        .or_default()
                        .push(resolved);
                }
            }

            let file_bindings = use_tree.bindings_for_file(&file);
            for binding in file_bindings {
                process_import_binding(
                    &file,
                    binding,
                    graph,
                    &symbols_by_module,
                    &mut bindings,
                    &table,
                    syntax,
                );
            }

            table.per_file.insert(file, bindings);
        }

        table
    }

    pub fn function_candidates(&self, file: &str, name: &str) -> &[ResolvedName] {
        self.per_file
            .get(file)
            .and_then(|bindings| bindings.function_bindings.get(name))
            .map(|vec| vec.as_slice())
            .unwrap_or(&[])
    }

    pub fn module_binding(&self, file: &str, name: &str) -> Option<&ModulePath> {
        self.per_file
            .get(file)
            .and_then(|bindings| bindings.module_bindings.get(name))
    }
}

fn collect_symbols(
    syntax: &SyntaxResults,
) -> (
    HashMap<String, Vec<SymbolInfo>>,
    HashMap<ModulePath, Vec<SymbolInfo>>,
) {
    let mut by_file: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
    let mut by_module: HashMap<ModulePath, Vec<SymbolInfo>> = HashMap::new();

    for symbol in &syntax.symbols {
        if symbol.kind != NodeKind::Function {
            continue;
        }

        let parts: Vec<String> = symbol.fqn.split("::").map(|s| s.to_string()).collect();
        if parts.is_empty() {
            continue;
        }

        let simple_name = parts.last().cloned().unwrap();
        let module_segments = if !parts.is_empty() {
            parts[..parts.len() - 1].to_vec()
        } else {
            Vec::new()
        };
        let module_path = ModulePath::from_segments(module_segments.clone());

        let info = SymbolInfo {
            fqn: symbol.fqn.clone(),
            module_path: module_path.clone(),
            simple_name,
        };

        by_file
            .entry(symbol.location.file.clone())
            .or_default()
            .push(info.clone());
        by_module.entry(module_path).or_default().push(info);
    }

    (by_file, by_module)
}

fn process_import_binding(
    file: &str,
    binding: &ImportBinding,
    graph: &ModuleGraph,
    symbols_by_module: &HashMap<ModulePath, Vec<SymbolInfo>>,
    bindings: &mut FileBindings,
    table: &NameBindingTable,
    syntax: &SyntaxResults,
) {
    let spec = &binding.record.spec;
    let context_path = graph
        .module_path_for_range(file, &spec.range)
        .or_else(|| graph.module_path_for_file(file).cloned())
        .unwrap_or_default();

    let base_path = determine_base_path(&context_path, &spec.path.root);

    if spec.is_glob {
        let module_path = extend_path(&base_path, &spec.path.segments);
        // Create module binding for the glob-imported module itself
        bindings
            .module_bindings
            .insert(binding.record.binding.clone(), module_path.clone());

        if let Some(symbols) = symbols_by_module.get(&module_path) {
            for symbol in symbols {
                let resolved = ResolvedName::new(
                    symbol.fqn.clone(),
                    symbol.module_path.clone(),
                    BindingOrigin::Glob,
                    ConfidenceWeight::Medium,
                );
                bindings
                    .function_bindings
                    .entry(symbol.simple_name.clone())
                    .or_default()
                    .push(resolved);
            }
        }

        // Also create module bindings for submodules that are glob-imported
        // This allows paths like `nested::nested_function` to work after `pub use re_export_source::*;`
        // We iterate through all nodes in the graph to find submodules
        for submodule_node in graph.nodes_iter() {
            let submodule_path = &submodule_node.module_path;
            if submodule_path.segments().len() == module_path.segments().len() + 1 {
                let submodule_prefix: Vec<String> =
                    submodule_path.segments()[..module_path.segments().len()].to_vec();
                if submodule_prefix == module_path.segments() {
                    let submodule_name = submodule_path.segments().last().unwrap().clone();
                    // Create a module binding for the submodule
                    bindings
                        .module_bindings
                        .entry(submodule_name.clone())
                        .or_insert_with(|| submodule_path.clone());
                }
            }
        }

        return;
    }

    if spec.path.segments.is_empty() {
        let module_path = base_path;
        bindings
            .module_bindings
            .insert(binding.record.binding.clone(), module_path);
        return;
    }

    let mut full_segments = base_path.segments().to_vec();
    full_segments.extend(spec.path.segments.iter().cloned());

    let (module_path, symbol_name) = if !full_segments.is_empty() {
        let mut module_segments = full_segments.clone();
        let symbol = module_segments.pop().unwrap_or_default();
        (ModulePath::from_segments(module_segments), symbol)
    } else {
        (ModulePath::new(), binding.record.binding.clone())
    };

    let has_module_node = graph
        .node(&module_path)
        .map(|node| {
            matches!(
                node.kind,
                ModuleNodeKind::Inline
                    | ModuleNodeKind::External
                    | ModuleNodeKind::File
                    | ModuleNodeKind::Root
            )
        })
        .unwrap_or(false);

    let has_symbol = symbols_by_module
        .get(&module_path)
        .map(|symbols| symbols.iter().any(|s| s.simple_name == symbol_name))
        .unwrap_or(false);

    // Check if this is a re-export: check if the binding name (what we're importing) is re-exported
    // in the target module's file. We need to check this even if has_symbol is true, because
    // the symbol might exist in a submodule but be re-exported at the module level.
    // For example: `use crate::nested::beta::beta_base as nested_beta_base;` imports from beta,
    // but `nested_beta_base` is the alias, and we need to check if it's re-exported in nested/mod.rs
    let mut resolved_fqn = None;

    // SPECIAL CASE: If this is a re-export WITH an alias (pub use X::Y as Z),
    // we need to resolve the path X::Y to get the FQN, then create a binding for Z.
    // This handles cases like:
    //   - `pub use re_export_source::source_function as aliased_source;` (same file)
    //   - `pub use circular_c::circular_a_func as root_circular_a;` (nested module)
    // Note: The binding.record.binding is the alias name (Z), and spec.path.segments is [X, Y]
    let is_re_export_with_alias = spec.kind == crate::application::ports::ImportKind::Reexport
        && spec.alias.is_some()
        && spec.alias.as_ref() == Some(&binding.record.binding);

    if is_re_export_with_alias {
        // This is a re-export with an alias matching our binding name
        // For inline modules, the symbol might actually be in the context module (same file),
        // not in a nested module path. We need to find the actual symbol's FQN.
        let target_symbol_name = spec.path.segments.last().unwrap_or(&binding.record.binding);

        // First, try to find the symbol by name in the context module (same file)
        // This handles inline modules where symbols are in the parent module
        if let Some(symbols) = symbols_by_module.get(&context_path) {
            if let Some(symbol) = symbols
                .iter()
                .find(|s| s.simple_name == *target_symbol_name)
            {
                resolved_fqn = Some(symbol.fqn.clone());
            }
        }

        // If not found in context, try the path-based approach
        // Build the module path from the spec's path segments
        if resolved_fqn.is_none() {
            let re_export_base = determine_base_path(&context_path, &spec.path.root);
            let re_export_path = extend_path(&re_export_base, &spec.path.segments);
            let target_module_path = if !spec.path.segments.is_empty() {
                let mut segments = re_export_path.segments().to_vec();
                let _symbol_name = segments.pop();
                ModulePath::from_segments(segments)
            } else {
                re_export_path.clone()
            };

            // Try to find the symbol in the target module
            if let Some(symbols) = symbols_by_module.get(&target_module_path) {
                if let Some(symbol) = symbols
                    .iter()
                    .find(|s| s.simple_name == *target_symbol_name)
                {
                    resolved_fqn = Some(symbol.fqn.clone());
                }
            }

            // If still not found, search all modules for matching symbol name
            // This handles cases where the symbol is re-exported through a chain
            // (e.g., circular_c::circular_a_func where circular_a_func is actually in circular_a)
            if resolved_fqn.is_none() {
                for (module_path, symbols) in symbols_by_module.iter() {
                    if let Some(symbol) = symbols
                        .iter()
                        .find(|s| s.simple_name == *target_symbol_name)
                    {
                        let target_segments = target_module_path.segments();
                        let symbol_segments = module_path.segments();

                        // Check if target path starts with symbol path (symbol is in parent module)
                        let is_prefix = target_segments.len() >= symbol_segments.len()
                            && target_segments
                                .iter()
                                .take(symbol_segments.len())
                                .zip(symbol_segments.iter())
                                .all(|(a, b)| a == b);

                        // Check if symbol path starts with target path (symbol is in submodule)
                        let is_suffix = symbol_segments.len() >= target_segments.len()
                            && symbol_segments
                                .iter()
                                .take(target_segments.len())
                                .zip(target_segments.iter())
                                .all(|(a, b)| a == b);

                        // Check if they share a common parent (for re-export chains through sibling modules)
                        // e.g., circular_c::circular_a_func where circular_a_func is in circular_a
                        // Both are children of chained_re_exports
                        let common_parent = target_segments.len() > 1
                            && symbol_segments.len() > 1
                            && target_segments[..target_segments.len().saturating_sub(1)]
                                == symbol_segments[..symbol_segments.len().saturating_sub(1)];

                        // If any relationship matches, use this symbol (re-export chain will validate access)
                        if is_prefix || is_suffix || common_parent {
                            resolved_fqn = Some(symbol.fqn.clone());
                            break;
                        }
                    }
                }
            }

            // Last resort: construct the FQN from the path
            if resolved_fqn.is_none() {
                resolved_fqn = Some(re_export_path.segments().join("::"));
            }
        }

        // If we found a resolved FQN, create the binding now and return early
        // to avoid creating duplicate bindings with wrong FQNs
        if let Some(fqn) = resolved_fqn {
            let confidence = ConfidenceWeight::High; // Re-exports are high confidence
            let resolved = ResolvedName::new(
                fqn.clone(),
                context_path.clone(),
                binding.origin,
                confidence,
            );

            bindings
                .function_bindings
                .entry(binding.record.binding.clone())
                .or_default()
                .push(resolved);
            return;
        }
    }

    // First, check if the binding name itself is re-exported (for aliases like nested_beta_base)
    // Check the module itself (for cases like nested/mod.rs where alias_alpha_base is re-exported)
    // We need to find the file that corresponds to this module path, not just use graph.node()
    // because graph.node() might return the wrong file for directory modules
    if let Some(module_node) = graph.node(&module_path) {
        if binding.record.binding == "alias_alpha_base"
            || binding.record.binding == "nested_beta_base"
        {
            tracing::debug!(
                "  Checking module {:?} (file: {}) for re-export of binding '{}'",
                module_path,
                module_node.file,
                binding.record.binding
            );
        }
        for import_spec in &syntax.import_specs {
            if import_spec.source_file == module_node.file
                && import_spec.kind == crate::application::ports::ImportKind::Reexport
            {
                if let Some(alias) = &import_spec.alias {
                    if alias == &binding.record.binding {
                        // Found a re-export matching our binding name
                        let re_export_base =
                            determine_base_path(&module_path, &import_spec.path.root);
                        let re_export_path =
                            extend_path(&re_export_base, &import_spec.path.segments);
                        let fqn = re_export_path.segments().join("::");
                        if binding.record.binding == "alias_alpha_base"
                            || binding.record.binding == "nested_beta_base"
                        {
                            tracing::debug!(
                                "  Found re-export in module {:?}: '{}' -> '{}'",
                                module_path,
                                binding.record.binding,
                                fqn
                            );
                        }
                        resolved_fqn = Some(fqn);
                        break;
                    }
                }
            }
        }
    }

    // Also check all files that might correspond to this module path
    // For directory modules (like nested/), we need to check nested/mod.rs
    if resolved_fqn.is_none() {
        // Try to find the file for this module path using module_path_for_file
        // We'll check all import_specs that have source_file matching this module path
        for import_spec in &syntax.import_specs {
            if let Some(spec_module_path) = graph.module_path_for_file(&import_spec.source_file) {
                if spec_module_path == &module_path
                    && import_spec.kind == crate::application::ports::ImportKind::Reexport
                {
                    if let Some(alias) = &import_spec.alias {
                        if alias == &binding.record.binding {
                            // Found a re-export matching our binding name
                            let re_export_base =
                                determine_base_path(&module_path, &import_spec.path.root);
                            let re_export_path =
                                extend_path(&re_export_base, &import_spec.path.segments);
                            let fqn = re_export_path.segments().join("::");
                            if binding.record.binding == "alias_alpha_base"
                                || binding.record.binding == "nested_beta_base"
                            {
                                tracing::debug!(
                                    "  Found re-export in file '{}' (module {:?}): '{}' -> '{}'",
                                    import_spec.source_file,
                                    module_path,
                                    binding.record.binding,
                                    fqn
                                );
                            }
                            resolved_fqn = Some(fqn);
                            break;
                        }
                    }
                }
            }
        }
    }

    // Also check parent module if module itself didn't have the re-export
    if resolved_fqn.is_none() {
        if let Some(parent_path) = module_path.parent() {
            if let Some(parent_node) = graph.node(&parent_path) {
                if binding.record.binding == "alias_alpha_base"
                    || binding.record.binding == "nested_beta_base"
                {
                    tracing::debug!(
                        "  Checking parent module {:?} (file: {}) for re-export of binding '{}'",
                        parent_path,
                        parent_node.file,
                        binding.record.binding
                    );
                }
                for import_spec in &syntax.import_specs {
                    if import_spec.source_file == parent_node.file
                        && import_spec.kind == crate::application::ports::ImportKind::Reexport
                    {
                        if let Some(alias) = &import_spec.alias {
                            if alias == &binding.record.binding {
                                // Found a re-export matching our binding name
                                let re_export_base =
                                    determine_base_path(&parent_path, &import_spec.path.root);
                                let re_export_path =
                                    extend_path(&re_export_base, &import_spec.path.segments);
                                let fqn = re_export_path.segments().join("::");
                                if binding.record.binding == "alias_alpha_base"
                                    || binding.record.binding == "nested_beta_base"
                                {
                                    tracing::debug!(
                                        "  Found re-export in parent: '{}' -> '{}'",
                                        binding.record.binding,
                                        fqn
                                    );
                                }
                                resolved_fqn = Some(fqn);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // Also check if the symbol_name is re-exported in the target module (original logic)
    if resolved_fqn.is_none() && has_module_node && !has_symbol {
        if binding.record.binding == "alias_alpha_base"
            || binding.record.binding == "nested_beta_base"
        {
            tracing::debug!(
                "  Checking for re-export of symbol '{}' in module_path={:?}",
                symbol_name,
                module_path
            );
        }
        if let Some(module_node) = graph.node(&module_path) {
            if binding.record.binding == "alias_alpha_base"
                || binding.record.binding == "nested_beta_base"
            {
                tracing::debug!(
                    "  module_node.file='{}', checking {} import_specs",
                    module_node.file,
                    syntax.import_specs.len()
                );
            }
            // Check if the symbol is re-exported in the target module's file
            // Look for import specs in the target file that re-export this symbol
            for import_spec in &syntax.import_specs {
                if import_spec.source_file == module_node.file
                    && import_spec.kind == crate::application::ports::ImportKind::Reexport
                {
                    if binding.record.binding == "alias_alpha_base"
                        || binding.record.binding == "nested_beta_base"
                    {
                        tracing::debug!("    Checking import_spec: source_file='{}', alias={:?}, segments={:?}, kind={:?}", 
                            import_spec.source_file, import_spec.alias, import_spec.path.segments, import_spec.kind);
                    }
                    // Check if this import re-exports the symbol we're looking for
                    if let Some(alias) = &import_spec.alias {
                        if alias == &symbol_name {
                            // This is a re-export with an alias matching our symbol
                            // Resolve the path to get the actual FQN
                            // Example: `pub use alpha::alpha_base as alias_alpha_base;` in nested/mod.rs
                            // We need to resolve `alpha::alpha_base` relative to `module_path` (which is ["function-relationship-test", "nested"])
                            let re_export_base =
                                determine_base_path(&module_path, &import_spec.path.root);
                            let re_export_path =
                                extend_path(&re_export_base, &import_spec.path.segments);
                            // The path segments are the full path to the symbol, use it directly
                            let fqn = re_export_path.segments().join("::");
                            if symbol_name == "alias_alpha_base"
                                || symbol_name == "nested_beta_base"
                            {
                                tracing::debug!(
                                    "process_import_binding: found re-export '{}' -> '{}' in file '{}'",
                                    symbol_name,
                                    fqn,
                                    file
                                );
                            }
                            resolved_fqn = Some(fqn);
                            break;
                        }
                    } else if import_spec.path.segments.last().map(|s| s.as_str())
                        == Some(&symbol_name)
                    {
                        // Direct re-export without alias
                        let re_export_base =
                            determine_base_path(&module_path, &import_spec.path.root);
                        let re_export_path =
                            extend_path(&re_export_base, &import_spec.path.segments);
                        resolved_fqn = Some(re_export_path.segments().join("::"));
                        break;
                    }
                }
            }

            // Fallback: check table if it exists (for cases where file was already processed)
            if resolved_fqn.is_none() {
                if let Some(re_exported) = table
                    .function_candidates(&module_node.file, &symbol_name)
                    .first()
                {
                    resolved_fqn = Some(re_exported.fqn.clone());
                }
            }
        }
    }

    let full_module_path = ModulePath::from_segments(full_segments.clone());

    // For aliases, always create function bindings even if has_symbol is false,
    // because aliases refer to symbols that may be in different modules.
    // For non-aliases, only create module bindings if there's a module node but no symbol and no re-export found.
    if has_module_node
        && !has_symbol
        && resolved_fqn.is_none()
        && !matches!(binding.origin, BindingOrigin::Alias)
    {
        bindings
            .module_bindings
            .insert(binding.record.binding.clone(), full_module_path);
        return;
    }

    // Determine the FQN: prefer re-export FQN, then actual symbol FQN if found, otherwise construct from path
    let fqn = if let Some(re_export_fqn) = resolved_fqn {
        re_export_fqn
    } else if has_symbol {
        // If the symbol exists, use its actual FQN from the symbol index
        // This ensures we match what's in the symbol index, not a constructed path
        if let Some(symbols) = symbols_by_module.get(&module_path) {
            if let Some(symbol) = symbols.iter().find(|s| s.simple_name == symbol_name) {
                symbol.fqn.clone()
            } else {
                // Fallback to constructed path if symbol not found (shouldn't happen)
                full_segments.join("::")
            }
        } else {
            full_segments.join("::")
        }
    } else if full_segments.is_empty() {
        binding.record.binding.clone()
    } else {
        full_segments.join("::")
    };

    let confidence = if has_symbol {
        ConfidenceWeight::High
    } else {
        match binding.origin {
            BindingOrigin::ExternalCrate => ConfidenceWeight::Low,
            _ => ConfidenceWeight::Medium,
        }
    };

    let resolved = ResolvedName::new(fqn.clone(), module_path, binding.origin, confidence);
    bindings
        .function_bindings
        .entry(binding.record.binding.clone())
        .or_default()
        .push(resolved);
}

fn determine_base_path(context: &ModulePath, root: &PathRoot) -> ModulePath {
    match root {
        PathRoot::Crate | PathRoot::Absolute => context_root(context),
        PathRoot::SelfPath => context.clone(),
        PathRoot::Super(depth) => ascend(context, *depth),
        PathRoot::ExternalCrate(name) => ModulePath::from_segments(vec![name.clone()]),
        PathRoot::Unqualified => context.clone(),
    }
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

fn extend_path(base: &ModulePath, extra: &[String]) -> ModulePath {
    let mut segments = base.segments().to_vec();
    segments.extend(extra.iter().cloned());
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
            "[package]\nname=\"binding_test\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();

        let lib_path = crate_dir.join("src/lib.rs");
        std::fs::write(&lib_path, "pub mod foo;\n").unwrap();
        let foo_path = crate_dir.join("src/foo.rs");
        std::fs::write(&foo_path, "pub fn call_me() {}\n").unwrap();

        let lib_path = std::fs::canonicalize(lib_path)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let foo_path = std::fs::canonicalize(foo_path)
            .unwrap()
            .to_string_lossy()
            .to_string();

        let crate_name = crate_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('-', "_");

        (temp, crate_name, lib_path, foo_path)
    }

    #[test]
    fn resolves_alias_and_glob_bindings() {
        let (_temp, crate_name, lib_path, foo_path) = setup_crate();

        let mut syntax = SyntaxResults::new();
        syntax.add_mod_decl(ModDecl {
            name: "foo".into(),
            source_file: lib_path.clone(),
            range: Range::with_file(1, 0, 1, 10, lib_path.clone()),
            kind: ModKind::External,
            resolved_file: Some(foo_path.clone()),
        });

        let function_fqn = format!("{}::foo::call_me", crate_name);
        syntax.add_symbol(Node::function(
            function_fqn.clone(),
            Range::with_file(1, 0, 1, 15, foo_path.clone()),
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

        syntax.add_import_spec(ImportSpec {
            range: Range::with_file(3, 0, 3, 15, lib_path.clone()),
            path: ImportPath::new(PathRoot::Crate, vec!["foo".into()]),
            alias: None,
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: true,
            source_file: lib_path.clone(),
        });

        let graph = ModuleGraph::build(&syntax);
        let use_tree = UseTree::build(&syntax);
        let bindings = NameBindingTable::build(&syntax, &graph, &use_tree);

        let alias_candidates = bindings.function_candidates(&lib_path, "alias_call");
        assert_eq!(alias_candidates.len(), 1);
        assert_eq!(alias_candidates[0].fqn, function_fqn);
        assert!(matches!(alias_candidates[0].origin, BindingOrigin::Alias));

        let glob_candidates = bindings.function_candidates(&lib_path, "call_me");
        assert!(!glob_candidates.is_empty());
        assert!(glob_candidates.iter().any(|cand| cand.fqn == function_fqn));
        assert!(glob_candidates
            .iter()
            .any(|cand| matches!(cand.origin, BindingOrigin::Glob)));
    }
}
