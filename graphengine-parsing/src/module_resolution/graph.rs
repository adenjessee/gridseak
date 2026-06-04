use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::application::ports::{ModDecl, ModKind, SyntaxResults};
use crate::domain::Range;

use super::types::ModulePath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleNodeKind {
    Root,
    File,
    Inline,
    External,
    Virtual,
}

#[derive(Debug, Clone)]
pub struct ModuleNode {
    pub module_path: ModulePath,
    pub file: String,
    pub kind: ModuleNodeKind,
    pub range: Option<Range>,
    pub parent: Option<ModulePath>,
    pub children: Vec<ModulePath>,
}

#[derive(Debug, Clone)]
struct InlineModuleEntry {
    module_path: ModulePath,
    range: Range,
}

#[derive(Debug, Default, Clone)]
pub struct ModuleGraph {
    nodes: HashMap<ModulePath, ModuleNode>,
    file_primary_module: HashMap<String, ModulePath>,
    inline_modules_by_file: HashMap<String, Vec<InlineModuleEntry>>,
}

impl ModuleGraph {
    pub fn build(syntax: &SyntaxResults) -> Self {
        let mut graph = ModuleGraph::default();
        graph.initialize_primary_modules(syntax);
        graph.process_module_declarations(syntax);
        graph
    }

    pub fn module_path_for_file(&self, file: &str) -> Option<&ModulePath> {
        self.file_primary_module.get(file)
    }

    pub fn module_path_for_range(&self, file: &str, range: &Range) -> Option<ModulePath> {
        let mut best_match: Option<&InlineModuleEntry> = None;
        if let Some(entries) = self.inline_modules_by_file.get(file) {
            for entry in entries {
                if range_contains(&entry.range, range) {
                    match best_match {
                        None => best_match = Some(entry),
                        Some(current) => {
                            let current_depth = current.module_path.segments().len();
                            let candidate_depth = entry.module_path.segments().len();
                            if candidate_depth > current_depth {
                                best_match = Some(entry);
                            }
                        }
                    }
                }
            }
        }

        if let Some(entry) = best_match {
            return Some(entry.module_path.clone());
        }

        self.module_path_for_file(file).cloned()
    }

    pub fn shared_prefix_len_between(&self, a: &str, b: &str) -> usize {
        match (self.module_path_for_file(a), self.module_path_for_file(b)) {
            (Some(path_a), Some(path_b)) => path_a.shared_prefix_len(path_b),
            _ => 0,
        }
    }

    pub fn node(&self, module_path: &ModulePath) -> Option<&ModuleNode> {
        self.nodes.get(module_path)
    }

    pub fn nodes_iter(&self) -> impl Iterator<Item = &ModuleNode> {
        self.nodes.values()
    }

    fn initialize_primary_modules(&mut self, syntax: &SyntaxResults) {
        let mut files: HashSet<String> = HashSet::new();

        for symbol in &syntax.symbols {
            files.insert(symbol.location.file.clone());
        }
        for reference in &syntax.references {
            files.insert(reference.call_site().location.file.clone());
        }
        for import in &syntax.imports {
            files.insert(import.file.clone());
        }
        for type_ref in &syntax.type_refs {
            files.insert(type_ref.file.clone());
        }
        for spec in &syntax.import_specs {
            files.insert(spec.source_file.clone());
        }
        for decl in &syntax.mod_decls {
            files.insert(decl.source_file.clone());
            if let Some(resolved) = &decl.resolved_file {
                files.insert(resolved.clone());
            }
        }

        for file in files {
            let module_path = module_path_from_file(&file);
            self.file_primary_module
                .insert(file.clone(), module_path.clone());
            self.ensure_node(module_path.clone(), file, ModuleNodeKind::File, None);
        }
    }

    fn ensure_node(
        &mut self,
        module_path: ModulePath,
        file: String,
        kind: ModuleNodeKind,
        range: Option<Range>,
    ) {
        if self.nodes.contains_key(&module_path) {
            return;
        }

        let parent = module_path.parent();
        let mut node = ModuleNode {
            module_path: module_path.clone(),
            file: file.clone(),
            kind,
            range,
            parent: parent.clone(),
            children: Vec::new(),
        };

        if let Some(parent_path) = parent.clone() {
            if let Some(parent_node) = self.nodes.get_mut(&parent_path) {
                parent_node.children.push(module_path.clone());
            } else {
                // Insert placeholder parent node to maintain tree structure.
                self.ensure_node(
                    parent_path.clone(),
                    file.clone(),
                    ModuleNodeKind::Virtual,
                    None,
                );
                if let Some(parent_node) = self.nodes.get_mut(&parent_path) {
                    parent_node.children.push(module_path.clone());
                }
            }
        } else {
            node.kind = ModuleNodeKind::Root;
        }

        self.nodes.insert(module_path, node);
    }

    fn process_module_declarations(&mut self, syntax: &SyntaxResults) {
        let mut decls_by_file: HashMap<String, Vec<&ModDecl>> = HashMap::new();
        for decl in &syntax.mod_decls {
            decls_by_file
                .entry(decl.source_file.clone())
                .or_default()
                .push(decl);
        }

        for (file, decls) in decls_by_file.into_iter() {
            let mut sorted_decls = decls;
            sorted_decls.sort_by(|a, b| {
                (a.range.start_line, a.range.start_char)
                    .cmp(&(b.range.start_line, b.range.start_char))
            });

            let base_path = match self.module_path_for_file(&file) {
                Some(path) => path.clone(),
                None => module_path_from_file(&file),
            };

            let mut scope_stack: Vec<(ModulePath, Option<Range>)> = vec![(base_path.clone(), None)];

            for decl in sorted_decls {
                // Reconcile current scope based on range nesting
                while let Some((_, maybe_range)) = scope_stack.last() {
                    if let Some(scope_range) = maybe_range {
                        if range_contains(scope_range, &decl.range) {
                            break;
                        } else {
                            scope_stack.pop();
                        }
                    } else {
                        break;
                    }
                }

                let parent_path = scope_stack
                    .last()
                    .map(|(path, _)| path.clone())
                    .unwrap_or_else(|| base_path.clone());

                let mut child_path = parent_path.clone();
                child_path.push(decl.name.clone());

                match decl.kind {
                    ModKind::Inline => {
                        self.ensure_node(
                            child_path.clone(),
                            file.clone(),
                            ModuleNodeKind::Inline,
                            Some(decl.range.clone()),
                        );

                        self.inline_modules_by_file
                            .entry(file.clone())
                            .or_default()
                            .push(InlineModuleEntry {
                                module_path: child_path.clone(),
                                range: decl.range.clone(),
                            });

                        scope_stack.push((child_path, Some(decl.range.clone())));
                    }
                    ModKind::External => {
                        if let Some(resolved_file) = &decl.resolved_file {
                            self.file_primary_module
                                .insert(resolved_file.clone(), child_path.clone());
                            self.ensure_node(
                                child_path.clone(),
                                resolved_file.clone(),
                                ModuleNodeKind::External,
                                None,
                            );
                        } else {
                            self.ensure_node(
                                child_path.clone(),
                                file.clone(),
                                ModuleNodeKind::External,
                                None,
                            );
                        }
                    }
                }
            }
        }
    }
}

fn module_path_from_file(file_path: &str) -> ModulePath {
    let path = Path::new(file_path);
    let mut segments = Vec::new();

    if let Some(crate_name) = detect_crate_name(path) {
        segments.push(crate_name);
    }

    if let Some(parent) = path.parent() {
        let components: Vec<String> = parent
            .components()
            .filter_map(|component| component.as_os_str().to_str().map(|s| s.to_string()))
            .collect();

        if let Some(src_index) = components.iter().position(|c| c == "src") {
            for component in &components[src_index + 1..] {
                if component != "lib" && component != "main" {
                    // Keep hyphens to match symbol FQNs which use directory names directly
                    segments.push(component.clone());
                }
            }
        }
    }

    if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
        match file_stem {
            "lib" | "main" | "mod" => {}
            // Keep hyphens to match symbol FQNs which use file names directly
            other => segments.push(other.to_string()),
        }
    }

    ModulePath(segments)
}

fn detect_crate_name(path: &Path) -> Option<String> {
    let mut current = path.parent();
    while let Some(dir) = current {
        let candidate = dir.join("Cargo.toml");
        if candidate.exists() {
            // Use directory name directly (with hyphens) to match symbol FQNs
            // Symbol FQNs use directory name from extract_crate_name which doesn't convert hyphens
            return dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(|s| s.to_string());
        }
        current = dir.parent();
    }
    None
}

fn range_contains(outer: &Range, inner: &Range) -> bool {
    let outer_start = (outer.start_line, outer.start_char);
    let outer_end = (outer.end_line, outer.end_char);
    let inner_start = (inner.start_line, inner.start_char);
    let inner_end = (inner.end_line, inner.end_char);

    outer_start <= inner_start && inner_end <= outer_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_file(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, "// test file\n").unwrap();
    }

    fn canonical_string(path: &Path) -> String {
        std::fs::canonicalize(path)
            .unwrap()
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn maps_inline_and_external_modules() {
        let temp = tempdir().unwrap();
        let crate_dir = temp.path();
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname=\"test_crate\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();

        let lib_path_buf = crate_dir.join("src/lib.rs");
        write_file(&lib_path_buf);
        let inline_range = Range::with_file(1, 0, 5, 0, canonical_string(&lib_path_buf));

        let external_path_buf = crate_dir.join("src/external_mod.rs");
        write_file(&external_path_buf);

        let lib_path = canonical_string(&lib_path_buf);
        let external_path = canonical_string(&external_path_buf);

        let mut syntax = SyntaxResults::new();
        syntax.add_mod_decl(ModDecl {
            name: "inline_mod".to_string(),
            source_file: lib_path.clone(),
            range: inline_range.clone(),
            kind: ModKind::Inline,
            resolved_file: Some(lib_path.clone()),
        });
        syntax.add_mod_decl(ModDecl {
            name: "external_mod".to_string(),
            source_file: lib_path.clone(),
            range: Range::with_file(6, 0, 6, 12, lib_path.clone()),
            kind: ModKind::External,
            resolved_file: Some(external_path.clone()),
        });

        let graph = ModuleGraph::build(&syntax);

        let crate_name = crate_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('-', "_");

        let lib_module = graph
            .module_path_for_file(&lib_path)
            .expect("lib module path");
        assert_eq!(lib_module.segments(), std::slice::from_ref(&crate_name));

        let inline_module = graph
            .module_path_for_range(&lib_path, &inline_range)
            .expect("inline module path");
        assert_eq!(
            inline_module.segments(),
            &[crate_name.clone(), "inline_mod".into()]
        );

        let external_module = graph
            .module_path_for_file(&external_path)
            .expect("external module path");
        assert_eq!(
            external_module.segments(),
            &[crate_name.clone(), "external_mod".into()]
        );
    }
}
