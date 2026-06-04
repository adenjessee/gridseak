//! Containment relationship builder for Phase 1, 2 & 3
//!
//! This module creates Project, Crate, File, and Folder nodes, and establishes containment edges:
//!
//! Phase 1:
//!   - Module → Function (from FQN hierarchy)
//!   - Module → Interface (from FQN hierarchy)
//!   - Module → Type (type aliases, from FQN hierarchy)
//!   - Module → Enum (from FQN hierarchy)
//!   - Module → Struct (from FQN hierarchy)
//!   - Module → Module (submodule hierarchy)
//!
//! Phase 2:
//!   - Project → Crate / File / Folder (top-level containment)
//!   - Crate → File (all files in crate)
//!   - Crate → Module (root modules)
//!   - File → Module (modules declared in each file)
//!
//! Phase 3:
//!   - Folder → File (files in each folder)
//!   - Folder → Folder (nested directory hierarchy)
//!
//! Language-agnostic design: Works with project roots, file paths, and FQNs.

use crate::domain::*;
use crate::syntax::utils::path_utils;
use std::collections::{HashMap, HashSet};
use tracing::{debug, info, warn};

/// Build containment relationships from parsed nodes
///
/// Creates:
/// Phase 1:
///   1. Project nodes (one per workspace root)
///   2. Crate nodes (one per unique crate name)
///   3. File nodes (one per unique file path)
///   3. Contains edges: Module → Function
///   4. Contains edges: Module → Interface
///   5. Contains edges: Module → Type (type aliases)
///   6. Contains edges: Module → Enum
///   7. Contains edges: Module → Struct
///   8. Contains edges: Module → Module
///
/// Phase 2:
///   9. Contains edges: Project → Crate / File / Folder
///   10. Contains edges: Crate → File
///   11. Contains edges: Crate → Module (root modules)
///   12. Contains edges: File → Module
///
/// Phase 3:
///   12. Folder nodes (one per unique directory path)
///   13. Contains edges: Folder → File
///   14. Contains edges: Folder → Folder (nested directories)
///
/// Language-agnostic: Works with FQN hierarchy (crate::module::item) and file paths
pub struct ContainmentBuilder;

impl ContainmentBuilder {
    /// Build containment nodes and edges from existing nodes
    ///
    /// # Arguments
    /// * `nodes` - All existing nodes (Functions, Modules, etc.)
    /// * `syntax_results` - Syntax extraction results (for mod_decls)
    ///
    /// # Returns
    /// * `(new_nodes, new_edges)` - Crate/File nodes and Contains edges to add
    pub fn build_containment(
        nodes: &[Node],
        syntax_results: &crate::application::ports::SyntaxResults,
    ) -> (Vec<Node>, Vec<Edge>) {
        let mut new_nodes = Vec::new();
        let mut new_edges = Vec::new();

        let workspace_root_path: Option<std::path::PathBuf> = syntax_results
            .workspace_root
            .as_ref()
            .map(std::path::PathBuf::from)
            .map(|p| p.canonicalize().unwrap_or(p));
        let language = syntax_results.language.as_deref();

        // Step 1: Extract unique crate names, file paths, and folder paths
        let mut crate_names: HashSet<String> = HashSet::new();
        let mut crate_manifest_paths: HashMap<String, String> = HashMap::new();
        let mut file_paths: HashSet<String> = HashSet::new();
        let mut file_to_crate: HashMap<String, String> = HashMap::new();
        let mut folder_paths: HashSet<String> = HashSet::new();
        let mut nodes_by_id: HashMap<String, &Node> = HashMap::new();
        let rust_like = syntax_results
            .language
            .as_deref()
            .map(|lang| lang.eq_ignore_ascii_case("rust"))
            .unwrap_or(true);
        // Languages with no true nested-module hierarchy (Apex today —
        // TypeScript/JavaScript/Go eventually; tracked separately) must
        // NOT synthesize ancestor Module nodes from the FQN walk. Their
        // FQN prefix only reflects filesystem structure, which is
        // already represented by the Folder node hierarchy. Each file's
        // `__file_module__` Module node is the correct container for
        // its symbols, and every symbol in that file attaches directly
        // to it. See docs/workstreams/apex/VALIDATION_RESULTS.md (PagedResult.cls
        // anomaly) for the motivating case.
        let use_file_module_as_container = syntax_results
            .language
            .as_deref()
            .map(|lang| lang.eq_ignore_ascii_case("apex"))
            .unwrap_or(false);

        for node in nodes {
            nodes_by_id.insert(node.id.clone(), node);

            // Extract file path from location
            let file_path = node.location.file.clone();
            file_paths.insert(file_path.clone());
            if rust_like {
                if let Some(crate_name) = Self::extract_crate_name(&node.fqn) {
                    crate_names.insert(crate_name.clone());
                    file_to_crate.entry(file_path.clone()).or_insert(crate_name);
                }
                if let Some(manifest_path) = path_utils::find_cargo_toml_path(&node.location.file) {
                    if let Some(crate_name) = path_utils::extract_crate_name(&node.location.file) {
                        crate_names.insert(crate_name.clone());
                        crate_manifest_paths
                            .entry(crate_name)
                            .or_insert(manifest_path);
                    }
                }
                if let Some(crate_name) = path_utils::extract_crate_name(&file_path) {
                    file_to_crate.entry(file_path.clone()).or_insert(crate_name);
                }
            }

            // Extract folder paths from file paths (Phase 3)
            // Skip empty folder paths (files in root directory)
            if let Some(folder_path) = Self::extract_folder_path(&file_path) {
                if !folder_path.is_empty() {
                    folder_paths.insert(folder_path);
                }
            }
        }

        // Include all discovered source files, even if no symbols were extracted
        for file_path in &syntax_results.source_files {
            file_paths.insert(file_path.clone());

            if rust_like {
                if let Some(crate_name) = path_utils::extract_crate_name(file_path) {
                    crate_names.insert(crate_name.clone());
                    if let Some(manifest_path) = path_utils::find_cargo_toml_path(file_path) {
                        crate_manifest_paths
                            .entry(crate_name.clone())
                            .or_insert(manifest_path);
                    }
                    file_to_crate.entry(file_path.clone()).or_insert(crate_name);
                }
            }

            if let Some(folder_path) = Self::extract_folder_path(file_path) {
                if !folder_path.is_empty() {
                    folder_paths.insert(folder_path);
                }
            }
        }

        info!(
            "Building containment: {} crates, {} files, {} folders, {} nodes",
            crate_names.len(),
            file_paths.len(),
            folder_paths.len(),
            nodes.len()
        );

        // Step 2: Create Project node (workspace root)
        let project_node = syntax_results.workspace_root.as_ref().map(|root| {
            let mut node = Self::create_project_node(root.clone());
            if let Some(root_path) = workspace_root_path.as_deref() {
                // Recommended view roots are stored on the Project node properties.
                let candidates = crate::domain::compute_recommended_view_roots(
                    root_path,
                    language,
                    &syntax_results.source_files,
                );
                node.set_property(
                    "recommended_view_roots",
                    serde_json::to_value(candidates).unwrap_or(serde_json::Value::Array(vec![])),
                );
                node.set_property(
                    "path_abs",
                    serde_json::Value::String(root_path.to_string_lossy().to_string()),
                );
            }
            // NOTE: We intentionally do NOT set `properties.language` on
            // the Project node here.
            //
            // The polyglot parser orchestrator runs one pipeline pass per
            // language and persists each pass's graph with
            // `INSERT OR REPLACE INTO nodes …`
            // ([graphengine-parsing/src/infrastructure/storage/sqlite_repository.rs::upsert_nodes]),
            // which replaces the whole row including the `properties`
            // JSON. So whatever language we wrote on the first pass gets
            // clobbered by the last pass's language — typically picking
            // up a minor file type (e.g. a vendored JS file) and
            // mis-labelling a Rust-majority repo as `javascript`.
            //
            // The canonical project language is a file-majority signal
            // owned by `graphengine-analysis::health::graph::detect_ecosystem`
            // (rule 1 of its priority order). The analyzer writes the
            // resolved value back into the Project node and surfaces it
            // as `HealthReport.primary_language` so the local store and
            // CLI can keep `scan_runs.primary_language` in sync. Leaving
            // it unset here means the analyzer's file-majority answer is
            // the single source of truth across the system.
            //
            // The unused-binding suppression below is intentional: the
            // value is still useful for File/Folder classification
            // calls earlier in this function (see `language` deref
            // captures of `syntax_results.language`).
            let _ = language;
            node
        });
        if let Some(project_node) = project_node.as_ref() {
            new_nodes.push(project_node.clone());
            debug!(
                "Created project node: {} ({})",
                project_node.fqn, project_node.id
            );
        }

        // Step 3: Create Crate nodes
        let mut crate_nodes: HashMap<String, Node> = HashMap::new();
        for crate_name in &crate_names {
            let crate_node = Self::create_crate_node(
                crate_name.clone(),
                crate_manifest_paths.get(crate_name).cloned(),
            );
            let crate_id = crate_node.id.clone();
            crate_nodes.insert(crate_name.clone(), crate_node);
            new_nodes.push(crate_nodes[crate_name].clone());
            debug!("Created crate node: {} ({})", crate_name, crate_id);
        }

        // Step 4: Create File nodes
        //
        // Collect per-file `entry_points` tags from the already-extracted
        // symbol nodes first. The parsing extractors stamp tags like
        // `"rest_resource"`, `"aura_enabled"`, `"tdtm_runnable"` on each
        // symbol's `properties.entry_points`; we need them here so the
        // per-file framework list can be augmented at File-node
        // creation time (Wave 2.2 of the truthful-scans simplification
        // plan).
        let mut file_symbol_tags: HashMap<String, Vec<String>> = HashMap::new();
        for node in nodes {
            if let Some(tags) = node
                .properties
                .get("entry_points")
                .and_then(|v| v.as_array())
            {
                let file_path = &node.location.file;
                let bucket = file_symbol_tags.entry(file_path.clone()).or_default();
                for tag in tags {
                    if let Some(s) = tag.as_str() {
                        bucket.push(s.to_string());
                    }
                }
            }
        }

        let mut file_nodes: HashMap<String, Node> = HashMap::new();
        for file_path in &file_paths {
            let mut file_node = Self::create_file_node(file_path.clone());
            let abs = std::path::Path::new(file_path);
            let rel = workspace_root_path
                .as_deref()
                .and_then(|root| crate::domain::repo_relative_path(root, abs));
            let classification =
                crate::domain::classify_path(abs, workspace_root_path.as_deref(), language);
            file_node.properties =
                classification.to_properties(file_path, rel.as_deref(), language);

            // Path-based framework tags (fast, deterministic,
            // content-free).
            let path_frameworks = crate::domain::frameworks::detect_frameworks_by_path(
                abs,
                workspace_root_path.as_deref(),
                language,
            );

            // Promote file-scope frameworks discovered via symbol
            // `entry_points` tags (e.g. `@RestResource` →
            // `restresource`). Tags that are per-symbol concerns
            // (`aura_enabled`, `invocable_method`) are left for the
            // classifier to consume via `GraphNode.entry_point_tags`.
            let empty_tags: Vec<String> = Vec::new();
            let symbol_tags = file_symbol_tags.get(file_path).unwrap_or(&empty_tags);
            let augmented_frameworks =
                crate::domain::frameworks::augment_frameworks_from_symbol_tags(
                    path_frameworks,
                    symbol_tags.iter().map(String::as_str),
                );

            file_node.properties.insert(
                "frameworks".to_string(),
                serde_json::Value::Array(
                    augmented_frameworks
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect(),
                ),
            );

            let file_id = file_node.id.clone();
            file_nodes.insert(file_path.clone(), file_node);
            new_nodes.push(file_nodes[file_path].clone());
            debug!("Created file node: {} ({})", file_path, file_id);
        }

        // Step 4b: Create Folder nodes (Phase 3)
        let mut folder_nodes: HashMap<String, Node> = HashMap::new();
        for folder_path in &folder_paths {
            let mut folder_node = Self::create_folder_node(folder_path.clone());
            let abs = std::path::Path::new(folder_path);
            let rel = workspace_root_path
                .as_deref()
                .and_then(|root| crate::domain::repo_relative_path(root, abs));
            let classification =
                crate::domain::classify_path(abs, workspace_root_path.as_deref(), language);
            folder_node.properties =
                classification.to_properties(folder_path, rel.as_deref(), language);
            let folder_id = folder_node.id.clone();
            folder_nodes.insert(folder_path.clone(), folder_node);
            new_nodes.push(folder_nodes[folder_path].clone());
            debug!("Created folder node: {} ({})", folder_path, folder_id);
        }

        // Step 4: Build module hierarchy from FQNs
        // Track all symbol types that need containment relationships.
        // Modules are keyed by (FQN, file_path) to handle FQN collisions across
        // directories (e.g., runtime-tests/deno/hono.test.ts and src/hono.test.ts
        // both produce "hono.test::__file_module__").
        let mut module_fqns: HashMap<String, String> = HashMap::new(); // FQN -> node_id (first seen)
        let mut module_by_fqn_file: HashMap<(String, String), String> = HashMap::new(); // (FQN, file) -> node_id
                                                                                        // For path-based-FQN languages: map file → its `__file_module__`
                                                                                        // Module node id, so symbols in that file can skip the FQN
                                                                                        // ancestor walk and attach directly.
        let mut file_module_by_file: HashMap<String, String> = HashMap::new();
        let mut symbol_nodes: Vec<(&Node, &str)> = Vec::new(); // (node, kind_name) for all symbols

        for node in nodes {
            match node.kind {
                NodeKind::Module => {
                    let file_key = node.location.file.clone();
                    module_by_fqn_file
                        .insert((node.fqn.clone(), file_key.clone()), node.id.clone());
                    module_fqns
                        .entry(node.fqn.clone())
                        .or_insert_with(|| node.id.clone());
                    if node.properties.get("file_module").and_then(|v| v.as_bool()) == Some(true) {
                        file_module_by_file.insert(file_key, node.id.clone());
                    }
                }
                NodeKind::Function => symbol_nodes.push((node, "Function")),
                NodeKind::Interface => symbol_nodes.push((node, "Interface")),
                NodeKind::Type => symbol_nodes.push((node, "Type")),
                NodeKind::Enum => symbol_nodes.push((node, "Enum")),
                NodeKind::Struct => symbol_nodes.push((node, "Struct")),
                NodeKind::Variable => symbol_nodes.push((node, "Variable")),
                _ => {}
            }
        }

        // Step 4b: Auto-create missing Module nodes from all symbol FQNs
        // This ensures every symbol has a parent module, even if no explicit `mod` declaration was parsed.
        // In Rust, modules are defined by file paths - a file at src/foo.rs IS module crate::foo.
        // We mark these as Medium confidence to distinguish from explicitly declared modules.
        //
        // This also handles nested modules: if we have crate::foo::bar::func, we create both
        // crate::foo::bar AND crate::foo if they don't exist.
        //
        // Process ALL symbol nodes (including duplicates with same FQN).
        // Languages that use path-based FQNs (currently Apex) MUST skip
        // this walk: their FQN ancestors are filesystem noise, not real
        // modules, and materialising them creates dangling synthetic
        // Module nodes that end up File→Module-parented to arbitrary
        // files (see PagedResult.cls anomaly). For those languages
        // every symbol is contained by its file's `__file_module__`.
        let ancestor_walk_iter: &[(&Node, &str)] = if use_file_module_as_container {
            &[]
        } else {
            &symbol_nodes
        };
        for (symbol_node, symbol_kind) in ancestor_walk_iter {
            let mut current_fqn = symbol_node.fqn.clone();
            let symbol_file = symbol_node.location.file.clone();
            while let Some(parent_module_fqn) = Self::extract_parent_module_fqn(&current_fqn) {
                if crate_names.contains(&parent_module_fqn) {
                    break;
                }
                // Don't create a duplicate if this module already exists in ANY file.
                // A module from a different file with the same FQN is the SAME logical module
                // (e.g., `test_crate::parent` declared in parent.rs but referenced from child.rs).
                let file_key = (parent_module_fqn.clone(), symbol_file.clone());
                let already_exists = module_by_fqn_file.contains_key(&file_key)
                    || module_fqns.contains_key(&parent_module_fqn);
                if !already_exists {
                    let module_node = Node::new(
                        NodeKind::Module,
                        parent_module_fqn.clone(),
                        symbol_node.location.clone(),
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
                    );
                    debug!(
                        "Auto-created inferred module: {} (from {} {})",
                        parent_module_fqn, symbol_kind, symbol_node.fqn
                    );
                    module_by_fqn_file.insert(file_key, module_node.id.clone());
                    module_fqns
                        .entry(parent_module_fqn.clone())
                        .or_insert_with(|| module_node.id.clone());
                    new_nodes.push(module_node);
                }
                current_fqn = parent_module_fqn;
            }
        }

        // Step 5: Create Module → Symbol containment edges
        // Now that all parent modules exist (either declared or inferred), create edges
        // for ALL symbol nodes (including those with duplicate FQNs).
        // Prefer the module from the same file when FQN collisions exist.
        for (symbol_node, symbol_kind) in &symbol_nodes {
            let parent_fqn_opt = Self::extract_parent_module_fqn(&symbol_node.fqn);

            // Resolution order:
            //   1. Same-file Module matching the FQN-derived parent.
            //   2. Any Module matching the FQN-derived parent.
            //   3. Crate node matching the FQN-derived parent.
            //   4. (Path-based-FQN languages, e.g. Apex) the file's
            //      synthesized `__file_module__` Module node. This
            //      fallback is what prevents Apex symbols from being
            //      orphaned once the ancestor-walk auto-creation is
            //      skipped.
            let mut resolved_parent_id: Option<String> = None;
            let mut resolved_parent_label: Option<String> = None;

            if let Some(parent_fqn) = &parent_fqn_opt {
                let same_file_key = (parent_fqn.clone(), symbol_node.location.file.clone());
                if let Some(pid) = module_by_fqn_file.get(&same_file_key) {
                    resolved_parent_id = Some(pid.clone());
                    resolved_parent_label = Some(format!("Module {}", parent_fqn));
                } else if let Some(pid) = module_fqns.get(parent_fqn) {
                    resolved_parent_id = Some(pid.clone());
                    resolved_parent_label = Some(format!("Module {}", parent_fqn));
                } else if let Some(crate_node) = crate_nodes.get(parent_fqn) {
                    resolved_parent_id = Some(crate_node.id.clone());
                    resolved_parent_label = Some(format!("Crate {}", parent_fqn));
                }
            }

            if resolved_parent_id.is_none() && use_file_module_as_container {
                if let Some(fm_id) = file_module_by_file.get(&symbol_node.location.file) {
                    resolved_parent_id = Some(fm_id.clone());
                    resolved_parent_label =
                        Some(format!("__file_module__ {}", symbol_node.location.file));
                }
            }

            if let Some(parent_id) = resolved_parent_id {
                let edge = Edge::contains(
                    parent_id,
                    symbol_node.id.clone(),
                    Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                );
                new_edges.push(edge);
                debug!(
                    "Created containment edge: {} → {} {}",
                    resolved_parent_label.as_deref().unwrap_or("<unknown>"),
                    symbol_kind,
                    symbol_node.fqn
                );
            }
        }

        // Step 6: Create Module → Module containment edges (submodule hierarchy)
        // Use the (FQN, file) map so colliding modules in different files get correct parents.
        for ((child_fqn, child_file), child_id) in &module_by_fqn_file {
            if let Some(parent_fqn) = Self::extract_parent_module_fqn(child_fqn) {
                // Prefer same-file parent, then fall back to any module with that FQN
                let parent_key = (parent_fqn.clone(), child_file.clone());
                let parent_id = module_by_fqn_file
                    .get(&parent_key)
                    .or_else(|| module_fqns.get(&parent_fqn));

                if let Some(parent_id) = parent_id {
                    if parent_id != child_id {
                        let edge = Edge::contains(
                            parent_id.clone(),
                            child_id.clone(),
                            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                        );
                        new_edges.push(edge);
                        debug!(
                            "Created Module → Module edge: {} → {}",
                            parent_fqn, child_fqn
                        );
                    }
                } else {
                    debug!(
                        "Skipping Module → Module edge: parent '{}' is not a module (likely crate root)",
                        parent_fqn
                    );
                }
            }
        }

        // Phase 2: Create Project → Crate / root-level File / root-level Folder edges
        // Only connect Project to items that have no other folder parent, preventing
        // duplicate containment (Project→File AND Folder→File).
        if let Some(project_node) = project_node.as_ref() {
            for crate_node in crate_nodes.values() {
                new_edges.push(Edge::contains(
                    project_node.id.clone(),
                    crate_node.id.clone(),
                    Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                ));
            }
            let workspace_root_str = workspace_root_path
                .as_deref()
                .map(|p| p.to_string_lossy().to_string());
            for (file_path, file_node) in &file_nodes {
                let parent_folder = Self::extract_folder_path(file_path);
                let is_root_level = match (&parent_folder, &workspace_root_str) {
                    (Some(folder), Some(root)) => folder == root,
                    (None, _) => true,
                    _ => !folder_nodes.contains_key(parent_folder.as_deref().unwrap_or("")),
                };
                if is_root_level {
                    new_edges.push(Edge::contains(
                        project_node.id.clone(),
                        file_node.id.clone(),
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                    ));
                }
            }
            for (folder_path, folder_node) in &folder_nodes {
                let parent_folder = Self::extract_parent_folder_path(folder_path);
                let is_root_level = match &parent_folder {
                    Some(pf) => !folder_nodes.contains_key(pf),
                    None => true,
                };
                if is_root_level {
                    new_edges.push(Edge::contains(
                        project_node.id.clone(),
                        folder_node.id.clone(),
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                    ));
                }
            }
        }

        // Phase 2b: Create Crate → File, Crate → Module (root), and File → Module edges

        // Step 7: Create Crate → File edges (all files in the crate)
        for (crate_name, crate_node) in &crate_nodes {
            for (file_path, file_node) in &file_nodes {
                // Check if file belongs to this crate based on file discovery
                let file_belongs_to_crate = file_to_crate
                    .get(file_path)
                    .map(|file_crate| file_crate == crate_name)
                    .unwrap_or(false);

                if file_belongs_to_crate {
                    let edge = Edge::contains(
                        crate_node.id.clone(),
                        file_node.id.clone(),
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                    );
                    new_edges.push(edge);
                    debug!("Created Crate → File edge: {} → {}", crate_name, file_path);
                }
            }
        }

        // Step 8: Create Crate → Module edges (root modules of the crate)
        // Root modules are those that are directly under the crate (e.g., "crate::mod_a")
        for (crate_name, crate_node) in &crate_nodes {
            for ((module_fqn, _file), module_id) in &module_by_fqn_file {
                let parts: Vec<&str> = module_fqn.split("::").collect();
                if parts.len() == 2 && parts[0] == crate_name {
                    let edge = Edge::contains(
                        crate_node.id.clone(),
                        module_id.clone(),
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                    );
                    new_edges.push(edge);
                    debug!(
                        "Created Crate → Module edge: {} → {}",
                        crate_name, module_fqn
                    );
                }
            }
        }

        // Step 9: Create File → Module edges (modules declared in each file)
        // CRITICAL: A module should be linked to the file where it's DECLARED, not where it's implemented
        // For external modules (mod mod_a;), use source_file (where it's declared, e.g., lib.rs)
        // For inline modules (mod mod_a { ... }), use source_file (where it's declared)
        // This shows the module hierarchy as declared in the source code, not where code is implemented

        // Build a map of (module FQN, module file) -> file where it's declared
        let mut module_to_file: HashMap<(String, String), String> = HashMap::new();

        // First, check mod_decls to find where modules are declared
        for decl in &syntax_results.mod_decls {
            // Always use source_file (where the module is declared)
            // For external modules: mod mod_a; in lib.rs -> source_file = lib.rs
            // For inline modules: mod mod_a { ... } in lib.rs -> source_file = lib.rs
            let declaration_file = &decl.source_file;

            // Build module FQN from the file path and module name
            // For external modules: mod mod_a; in lib.rs -> mod_a.rs -> crate::mod_a
            // For inline modules: mod mod_a { ... } in lib.rs -> crate::mod_a
            // We need to construct the FQN based on where the module is declared and its name
            let module_name = &decl.name;

            // Find the crate this file belongs to
            let crate_name = file_to_crate.get(declaration_file).or_else(|| {
                crate_nodes.keys().find(|crate_name| {
                    // Fallback: Check if any node from this file belongs to this crate
                    nodes.iter().any(|node| {
                        if let Some(node_crate) = Self::extract_crate_name(&node.fqn) {
                            node_crate == **crate_name && node.location.file == *declaration_file
                        } else {
                            false
                        }
                    })
                })
            });

            if let Some(crate_name) = crate_name {
                let module_fqn = format!("{}::{}", crate_name, module_name);

                // Find all module instances with this FQN and associate them with the declaration file
                let matching_keys: Vec<(String, String)> = module_by_fqn_file
                    .keys()
                    .filter(|(fqn, _)| fqn == &module_fqn)
                    .cloned()
                    .collect();

                if matching_keys.is_empty() {
                    debug!(
                        "Module FQN {} not found in module_by_fqn_file, skipping",
                        module_fqn
                    );
                } else {
                    for key in matching_keys {
                        module_to_file.insert(key, declaration_file.clone());
                    }
                    debug!(
                        "Module {} is declared in file {} (implemented in {:?})",
                        module_fqn, declaration_file, decl.resolved_file
                    );
                }
            }
        }

        // Fallback: If we don't have mod_decl info, use the module's own location file.
        // The (FQN, file) map ensures we handle collisions correctly — each module instance
        // maps to its own file rather than picking a "best" file across collisions.
        for (module_fqn, module_file) in module_by_fqn_file.keys() {
            let key = (module_fqn.clone(), module_file.clone());
            if module_to_file.contains_key(&key) {
                continue;
            }
            module_to_file.insert(key, module_file.clone());
            debug!(
                "Fallback: Module {} assigned to file {} (from location)",
                module_fqn, module_file
            );
        }

        // Collect module IDs that already have a parent from FQN hierarchy (Steps 5-6 and 8).
        // These should NOT also get a File → Module edge, which would create a DAG instead of a tree.
        let modules_with_fqn_parent: HashSet<String> = new_edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Contains)
            .map(|e| e.to_id.clone())
            .collect();

        // Now create File → Module edges only for modules that lack a parent
        for ((module_fqn, module_file), file_path) in &module_to_file {
            let module_key = (module_fqn.clone(), module_file.clone());
            if let Some(module_id) = module_by_fqn_file.get(&module_key) {
                if modules_with_fqn_parent.contains(module_id) {
                    debug!(
                        "Skipping File → Module edge: {} → {} (already has FQN parent)",
                        file_path, module_fqn
                    );
                    continue;
                }
                if let Some(file_node) = file_nodes.get(file_path) {
                    let edge = Edge::contains(
                        file_node.id.clone(),
                        module_id.clone(),
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                    );
                    new_edges.push(edge);
                    debug!("Created File → Module edge: {} → {}", file_path, module_fqn);
                }
            }
        }

        // Phase 3: Create Folder → File and Folder → Folder edges

        // Step 10: Create Folder → File edges (files in each folder)
        for (folder_path, folder_node) in &folder_nodes {
            for (file_path, file_node) in &file_nodes {
                // Check if file is in this folder
                if Self::file_in_folder(file_path, folder_path) {
                    let edge = Edge::contains(
                        folder_node.id.clone(),
                        file_node.id.clone(),
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                    );
                    new_edges.push(edge);
                    debug!(
                        "Created Folder → File edge: {} → {}",
                        folder_path, file_path
                    );
                }
            }
        }

        // Step 11: Create Folder → Folder edges (nested directory hierarchy)
        for (child_folder_path, child_folder_node) in &folder_nodes {
            // Extract parent folder path
            if let Some(parent_folder_path) = Self::extract_parent_folder_path(child_folder_path) {
                if let Some(parent_folder_node) = folder_nodes.get(&parent_folder_path) {
                    // Don't create self-loops
                    if parent_folder_node.id != child_folder_node.id {
                        let edge = Edge::contains(
                            parent_folder_node.id.clone(),
                            child_folder_node.id.clone(),
                            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                        );
                        new_edges.push(edge);
                        debug!(
                            "Created Folder → Folder edge: {} → {}",
                            parent_folder_path, child_folder_path
                        );
                    }
                }
            }
        }

        info!(
            "Built containment: {} new nodes, {} new edges",
            new_nodes.len(),
            new_edges.len()
        );

        // Validate containment: warn about orphaned nodes
        Self::validate_containment(nodes, &new_nodes, &new_edges);

        (new_nodes, new_edges)
    }

    /// Validate that all nodes have proper containment relationships.
    ///
    /// Warns about:
    /// - Functions without a parent module
    /// - Modules without a parent crate or file
    /// - Any other orphaned nodes
    ///
    /// This helps catch parser bugs or data inconsistencies that would
    /// result in disconnected nodes in the visualization.
    fn validate_containment(existing_nodes: &[Node], new_nodes: &[Node], edges: &[Edge]) {
        use std::collections::HashSet;

        // Collect all node IDs that are targets of containment edges (have a parent)
        let nodes_with_parents: HashSet<&str> = edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Contains)
            .map(|e| e.to_id.as_str())
            .collect();

        // Suppress unused warning - new_nodes included for potential future use
        let _ = new_nodes;

        // Check each existing node for containment
        let mut orphan_count = 0;
        for node in existing_nodes {
            // Skip container nodes (Project, Crate, File, Folder) - they are allowed to be roots
            match node.kind {
                NodeKind::Project | NodeKind::Crate | NodeKind::File | NodeKind::Folder => continue,
                _ => {}
            }

            // Check if this node has a parent
            if !nodes_with_parents.contains(node.id.as_str()) {
                orphan_count += 1;
                // Only warn for the first few to avoid log spam
                if orphan_count <= 10 {
                    warn!(
                        "[CONTAINMENT] Orphaned {:?} node without parent: '{}' ({})",
                        node.kind, node.fqn, node.id
                    );
                }
            }
        }

        if orphan_count > 0 {
            warn!(
                "[CONTAINMENT] Found {} orphaned nodes without proper containment. \
                This may indicate parser bugs or FQN construction issues.",
                orphan_count
            );
        }
    }

    /// Extract crate name from FQN
    ///
    /// FQN format: "crate_name::module::item"
    /// Returns the first segment (crate name)
    fn extract_crate_name(fqn: &str) -> Option<String> {
        fqn.split("::").next().map(|s| s.to_string())
    }

    /// Extract parent module FQN from item FQN
    ///
    /// Examples:
    /// - "crate::mod_a::a1_base" -> Some("crate::mod_a")
    /// - "crate::nested::alpha::func" -> Some("crate::nested::alpha")
    /// - "crate::func" -> None (no parent module, directly in crate root)
    fn extract_parent_module_fqn(item_fqn: &str) -> Option<String> {
        let parts: Vec<&str> = item_fqn.split("::").collect();
        if parts.len() < 2 {
            // No parent possible (just a single name)
            return None;
        }
        // Remove last segment (the item itself) to get parent module
        // e.g., "hero_logo::HeroLogo" → "hero_logo"
        // e.g., "a::b::c" → "a::b"
        let parent_parts = &parts[..parts.len() - 1];
        Some(parent_parts.join("::"))
    }

    /// Create a Crate node
    fn create_crate_node(crate_name: String, manifest_path: Option<String>) -> Node {
        // Use crate name as FQN
        let fqn = crate_name.clone();

        // Create a location for the crate (Cargo.toml path when available)
        let location = Range::with_file(
            1,
            0,
            1,
            0,
            manifest_path.unwrap_or_else(|| format!("{}/Cargo.toml", crate_name)),
        );

        Node::new(
            NodeKind::Crate,
            fqn,
            location,
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        )
    }

    /// Create a Project node
    fn create_project_node(root_path: String) -> Node {
        let canonical_root = path_utils::canonical_string(std::path::Path::new(&root_path));
        let fqn = canonical_root.clone();
        let location = Range::with_file(1, 0, 1, 0, canonical_root);
        Node::new(
            NodeKind::Project,
            fqn,
            location,
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        )
    }

    /// Create a File node
    fn create_file_node(file_path: String) -> Node {
        // Use file path as FQN (normalized)
        let fqn = file_path.clone();

        // Create a synthetic location covering the entire file
        let location = Range::with_file(1, 0, 1, 0, file_path.clone());

        Node::new(
            NodeKind::File,
            fqn,
            location,
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        )
    }

    /// Create a Folder node (Phase 3)
    fn create_folder_node(folder_path: String) -> Node {
        // Use folder path as FQN (normalized)
        let fqn = folder_path.clone();

        // Create a synthetic location for the folder
        let location = Range::with_file(1, 0, 1, 0, folder_path.clone());

        Node::new(
            NodeKind::Folder,
            fqn,
            location,
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        )
    }

    /// Extract folder path from file path (Phase 3)
    ///
    /// Examples:
    /// - "/path/to/src/mod_a.rs" -> Some("/path/to/src")
    /// - "/path/to/src/nested/alpha.rs" -> Some("/path/to/src/nested")
    pub(crate) fn extract_folder_path(file_path: &str) -> Option<String> {
        std::path::Path::new(file_path)
            .parent()
            .and_then(|p| p.to_str())
            .map(|s| s.to_string())
    }

    /// Check if a file is in a folder (Phase 3)
    ///
    /// Returns true if the file's directory is the folder or a subdirectory of it
    pub(crate) fn file_in_folder(file_path: &str, folder_path: &str) -> bool {
        let file_dir = std::path::Path::new(file_path)
            .parent()
            .and_then(|p| p.to_str());

        let folder_path_normalized = std::path::Path::new(folder_path);

        if let Some(file_dir_str) = file_dir {
            let file_dir_path = std::path::Path::new(file_dir_str);
            // Check if file is DIRECTLY in this folder (not in a subdirectory)
            // Use canonical comparison to handle path normalization
            file_dir_path == folder_path_normalized
        } else {
            // File has no parent directory - check if folder_path is empty/root
            folder_path.is_empty() || folder_path == "/"
        }
    }

    /// Extract parent folder path from child folder path (Phase 3)
    ///
    /// Examples:
    /// - "/path/to/src/nested" -> Some("/path/to/src")
    /// - "/path/to/src" -> Some("/path/to")
    /// - "/path/to" -> Some("/path")
    /// - "/path" -> None (root)
    pub(crate) fn extract_parent_folder_path(folder_path: &str) -> Option<String> {
        std::path::Path::new(folder_path).parent().and_then(|p| {
            // Don't return root paths (e.g., "/")
            let parent_str = p.to_str()?;
            if parent_str == "/" || parent_str.is_empty() {
                None
            } else {
                Some(parent_str.to_string())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::utils::path_utils::canonical_string;
    use tempfile::tempdir;

    #[test]
    fn test_extract_crate_name() {
        assert_eq!(
            ContainmentBuilder::extract_crate_name("function-relationship-test::mod_a::a1_base"),
            Some("function-relationship-test".to_string())
        );
        assert_eq!(
            ContainmentBuilder::extract_crate_name("my_crate::func"),
            Some("my_crate".to_string())
        );
    }

    #[test]
    fn test_extract_parent_module_fqn() {
        assert_eq!(
            ContainmentBuilder::extract_parent_module_fqn("crate::mod_a::a1_base"),
            Some("crate::mod_a".to_string())
        );
        assert_eq!(
            ContainmentBuilder::extract_parent_module_fqn("crate::nested::alpha::func"),
            Some("crate::nested::alpha".to_string())
        );
        // "crate::func" has 2 parts, parent is "crate"
        assert_eq!(
            ContainmentBuilder::extract_parent_module_fqn("crate::func"),
            Some("crate".to_string())
        );
        // "crate::mod_a" has 2 parts, parent is "crate"
        assert_eq!(
            ContainmentBuilder::extract_parent_module_fqn("crate::mod_a"),
            Some("crate".to_string())
        );
        // Single name has no parent
        assert_eq!(
            ContainmentBuilder::extract_parent_module_fqn("standalone"),
            None
        );
    }

    #[test]
    fn test_build_containment() {
        let nodes = vec![
            Node::module(
                "test_crate::mod_a".to_string(),
                Range::with_file(1, 0, 10, 0, "src/mod_a.rs".to_string()),
            ),
            Node::function(
                "test_crate::mod_a::func1".to_string(),
                Range::with_file(5, 0, 10, 0, "src/mod_a.rs".to_string()),
            ),
            Node::module(
                "test_crate::mod_b".to_string(),
                Range::with_file(1, 0, 10, 0, "src/mod_b.rs".to_string()),
            ),
        ];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (new_nodes, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        // Should create 1 crate node, 2 file nodes, and folder nodes (Phase 3)
        // Folder nodes depend on file paths, so we check for at least 3 nodes
        assert!(
            new_nodes.len() >= 3,
            "Expected at least 1 crate + 2 files, got {}",
            new_nodes.len()
        );

        // Should create:
        // - 1 Module → Function edge (Phase 1)
        // - 1 Crate → File edge for each file (Phase 2) = 2 edges
        // - 1 Crate → Module edge for root module (Phase 2) = 1 edge
        // - 1 File → Module edge for each file's module (Phase 2) = 2 edges
        // Total: 1 + 2 + 1 + 2 = 6 edges
        assert!(
            !new_edges.is_empty(),
            "Expected at least 1 edge (Module → Function)"
        );

        // Verify Module → Function edge exists
        let mod_node = nodes.iter().find(|n| n.fqn == "test_crate::mod_a").unwrap();
        let func_node = nodes
            .iter()
            .find(|n| n.fqn == "test_crate::mod_a::func1")
            .unwrap();

        let mod_to_func_edge = new_edges
            .iter()
            .find(|e| e.from_id == mod_node.id && e.to_id == func_node.id);
        assert!(
            mod_to_func_edge.is_some(),
            "Expected Module → Function edge"
        );

        // Verify Phase 2 edges exist
        let _crate_node = new_nodes
            .iter()
            .find(|n| n.kind == NodeKind::Crate)
            .unwrap();
        let _file_nodes: Vec<_> = new_nodes
            .iter()
            .filter(|n| n.kind == NodeKind::File)
            .collect();

        // Should have Crate → File edges
        let crate_to_file_count = new_edges.iter()
            .filter(|e| {
                let from = new_nodes.iter().find(|n| n.id == e.from_id);
                let to = new_nodes.iter().find(|n| n.id == e.to_id);
                matches!((from, to), (Some(f), Some(t)) if f.kind == NodeKind::Crate && t.kind == NodeKind::File)
            })
            .count();
        assert_eq!(crate_to_file_count, 2, "Expected 2 Crate → File edges");

        // Should have Crate → Module edge
        let crate_to_mod_count = new_edges.iter()
            .filter(|e| {
                let from = new_nodes.iter().find(|n| n.id == e.from_id);
                let to = nodes.iter().find(|n| n.id == e.to_id);
                matches!((from, to), (Some(f), Some(t)) if f.kind == NodeKind::Crate && t.kind == NodeKind::Module)
            })
            .count();
        assert!(
            crate_to_mod_count >= 1,
            "Expected at least 1 Crate → Module edge"
        );

        // Modules with a Crate parent should NOT also get a File parent (tree, not DAG).
        // Verify each module has exactly one parent containment edge.
        for node in &nodes {
            if node.kind != NodeKind::Module {
                continue;
            }
            let parent_count = new_edges
                .iter()
                .filter(|e| e.to_id == node.id && e.kind == EdgeKind::Contains)
                .count();
            assert!(
                parent_count >= 1,
                "Module {} should have at least 1 parent",
                node.fqn
            );
            assert!(
                parent_count <= 1,
                "Module {} should have at most 1 parent, got {}",
                node.fqn,
                parent_count
            );
        }
    }

    #[test]
    fn test_project_node_created_from_workspace_root() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_string_lossy().to_string();

        let mut syntax_results = crate::application::ports::SyntaxResults::new();
        syntax_results.set_workspace_root(root.clone());
        syntax_results.set_language("typescript".to_string());

        let (new_nodes, _new_edges) = ContainmentBuilder::build_containment(&[], &syntax_results);

        let project_node = new_nodes.iter().find(|n| n.kind == NodeKind::Project);
        assert!(
            project_node.is_some(),
            "Expected Project node to be created"
        );

        let expected_root = canonical_string(std::path::Path::new(&root));
        assert_eq!(project_node.unwrap().fqn, expected_root);
    }

    /// A3 — the Project node must NOT carry a `language` property
    /// straight out of the parser. The polyglot orchestrator calls
    /// `build_containment` once per language pass, and the sqlite
    /// repository persists with `INSERT OR REPLACE`. If we wrote the
    /// pass's language onto the Project node, the last pass would
    /// win and a Rust-majority repo would end up labelled with
    /// whatever the final tail-pass language happened to be (often a
    /// minor JS/JSON pass).
    ///
    /// The canonical primary language is now owned by
    /// `graphengine-analysis::health::graph::detect_primary_language`
    /// (File-majority) and written back to the Project node by
    /// `run_analysis`. This test pins that contract so a future
    /// refactor cannot quietly re-introduce the parser-side write.
    #[test]
    fn test_project_node_has_no_language_property() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_string_lossy().to_string();

        let mut syntax_results = crate::application::ports::SyntaxResults::new();
        syntax_results.set_workspace_root(root.clone());
        syntax_results.set_language("typescript".to_string());

        let (new_nodes, _new_edges) = ContainmentBuilder::build_containment(&[], &syntax_results);

        let project_node = new_nodes
            .iter()
            .find(|n| n.kind == NodeKind::Project)
            .expect("Project node must be created");
        assert!(
            !project_node.properties.contains_key("language"),
            "Project node MUST NOT carry `properties.language` from the parser; the \
             analyzer is the source of truth for primary language (A3 fix). Found {:?}",
            project_node.properties.get("language")
        );
    }

    #[test]
    fn test_source_files_create_file_nodes_without_symbols() {
        let temp = tempdir().unwrap();
        let file_a = temp.path().join("a.ts");
        let file_b = temp.path().join("b.ts");
        std::fs::write(&file_a, "export const A = 1;").unwrap();
        std::fs::write(&file_b, "export const B = 2;").unwrap();

        let mut syntax_results = crate::application::ports::SyntaxResults::new();
        syntax_results.set_language("typescript".to_string());
        syntax_results.set_source_files(vec![
            file_a.to_string_lossy().to_string(),
            file_b.to_string_lossy().to_string(),
        ]);

        let (new_nodes, _new_edges) = ContainmentBuilder::build_containment(&[], &syntax_results);

        let file_nodes: Vec<_> = new_nodes
            .iter()
            .filter(|n| n.kind == NodeKind::File)
            .collect();
        assert_eq!(
            file_nodes.len(),
            2,
            "Expected 2 File nodes from source_files"
        );
        assert!(file_nodes.iter().any(|n| n.fqn == file_a.to_string_lossy()));
        assert!(file_nodes.iter().any(|n| n.fqn == file_b.to_string_lossy()));
    }

    #[test]
    fn test_crate_nodes_rust_only_and_file_edges_from_source_files() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname=\"crate_test\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        let src_dir = temp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let lib_path = src_dir.join("lib.rs");
        std::fs::write(&lib_path, "pub fn demo() {}").unwrap();

        let file_path = lib_path.to_string_lossy().to_string();

        let mut ts_results = crate::application::ports::SyntaxResults::new();
        ts_results.set_language("typescript".to_string());
        ts_results.set_source_files(vec![file_path.clone()]);
        let (ts_nodes, _ts_edges) = ContainmentBuilder::build_containment(&[], &ts_results);
        assert!(
            ts_nodes.iter().all(|n| n.kind != NodeKind::Crate),
            "Expected no Crate nodes for non-Rust language"
        );

        let mut rust_results = crate::application::ports::SyntaxResults::new();
        rust_results.set_language("rust".to_string());
        rust_results.set_source_files(vec![file_path.clone()]);
        let (rust_nodes, rust_edges) = ContainmentBuilder::build_containment(&[], &rust_results);

        let crate_node = rust_nodes
            .iter()
            .find(|n| n.kind == NodeKind::Crate)
            .unwrap();
        let file_node = rust_nodes
            .iter()
            .find(|n| n.kind == NodeKind::File && n.fqn == file_path)
            .unwrap();

        let has_crate_to_file = rust_edges.iter().any(|e| {
            e.kind == EdgeKind::Contains && e.from_id == crate_node.id && e.to_id == file_node.id
        });
        assert!(
            has_crate_to_file,
            "Expected Crate → File edge even when no symbols were extracted"
        );
    }

    #[test]
    fn test_extract_folder_path() {
        assert_eq!(
            ContainmentBuilder::extract_folder_path("/path/to/src/mod_a.rs"),
            Some("/path/to/src".to_string())
        );
        assert_eq!(
            ContainmentBuilder::extract_folder_path("/path/to/src/nested/alpha.rs"),
            Some("/path/to/src/nested".to_string())
        );
        // When file has no directory, parent() returns Some("") (empty string)
        // This is expected behavior - we'll filter these out during folder node creation
        let result = ContainmentBuilder::extract_folder_path("mod_a.rs");
        assert!(result.is_some() && result.unwrap().is_empty());
    }

    #[test]
    fn test_file_in_folder() {
        assert!(ContainmentBuilder::file_in_folder(
            "/path/to/src/mod_a.rs",
            "/path/to/src"
        ));
        // File in subdirectory should NOT be in parent folder
        assert!(!ContainmentBuilder::file_in_folder(
            "/path/to/src/nested/alpha.rs",
            "/path/to/src"
        ));
        // File should be in its direct parent folder
        assert!(ContainmentBuilder::file_in_folder(
            "/path/to/src/nested/alpha.rs",
            "/path/to/src/nested"
        ));
        assert!(!ContainmentBuilder::file_in_folder(
            "/path/to/src/mod_a.rs",
            "/path/to/other"
        ));
    }

    #[test]
    fn test_extract_parent_folder_path() {
        assert_eq!(
            ContainmentBuilder::extract_parent_folder_path("/path/to/src/nested"),
            Some("/path/to/src".to_string())
        );
        assert_eq!(
            ContainmentBuilder::extract_parent_folder_path("/path/to/src"),
            Some("/path/to".to_string())
        );
        assert_eq!(
            ContainmentBuilder::extract_parent_folder_path("/path"),
            None // Root path
        );
    }

    #[test]
    fn test_auto_create_missing_module_for_orphan_function() {
        // Test: Function exists but no Module node was parsed (no explicit mod declaration)
        // Expected: ContainmentBuilder should auto-create the missing module with Medium confidence
        let nodes = vec![
            // Only a function - NO module node provided
            Node::function(
                "test_crate::orphan_mod::orphan_func".to_string(),
                Range::with_file(5, 0, 10, 0, "src/orphan_mod.rs".to_string()),
            ),
        ];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (new_nodes, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        // Should auto-create the missing module "test_crate::orphan_mod"
        let auto_created_module = new_nodes
            .iter()
            .find(|n| n.kind == NodeKind::Module && n.fqn == "test_crate::orphan_mod");

        assert!(
            auto_created_module.is_some(),
            "Expected auto-created module 'test_crate::orphan_mod' but got nodes: {:?}",
            new_nodes
                .iter()
                .map(|n| (&n.kind, &n.fqn))
                .collect::<Vec<_>>()
        );

        let module = auto_created_module.unwrap();

        // Verify it has Medium confidence (inferred, not declared)
        assert_eq!(
            module.provenance.confidence,
            Confidence::Medium,
            "Auto-created module should have Medium confidence"
        );

        // Verify Module → Function edge was created
        let func_node = &nodes[0];
        let mod_to_func_edge = new_edges
            .iter()
            .find(|e| e.from_id == module.id && e.to_id == func_node.id);

        assert!(
            mod_to_func_edge.is_some(),
            "Expected Module → Function edge from auto-created module"
        );
    }

    #[test]
    fn test_auto_create_nested_modules_for_deep_function() {
        // Test: Function at crate::foo::bar::baz::deep_func
        // Expected: Should create foo, bar, AND baz modules if none exist
        let nodes = vec![Node::function(
            "test_crate::foo::bar::baz::deep_func".to_string(),
            Range::with_file(10, 0, 20, 0, "src/foo/bar/baz.rs".to_string()),
        )];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (new_nodes, _new_edges) =
            ContainmentBuilder::build_containment(&nodes, &syntax_results);

        // Should auto-create all three nested modules
        let module_fqns: Vec<_> = new_nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module)
            .map(|n| n.fqn.as_str())
            .collect();

        assert!(
            module_fqns.contains(&"test_crate::foo::bar::baz"),
            "Expected module test_crate::foo::bar::baz, got: {:?}",
            module_fqns
        );
        assert!(
            module_fqns.contains(&"test_crate::foo::bar"),
            "Expected module test_crate::foo::bar, got: {:?}",
            module_fqns
        );
        assert!(
            module_fqns.contains(&"test_crate::foo"),
            "Expected module test_crate::foo, got: {:?}",
            module_fqns
        );

        // All should have Medium confidence
        for module in new_nodes.iter().filter(|n| n.kind == NodeKind::Module) {
            assert_eq!(
                module.provenance.confidence,
                Confidence::Medium,
                "Auto-created module {} should have Medium confidence",
                module.fqn
            );
        }
    }

    #[test]
    fn test_no_duplicate_module_when_already_exists() {
        // Test: Module exists AND function exists
        // Expected: Should NOT create duplicate module; existing module retains High confidence
        let nodes = vec![
            Node::module(
                "test_crate::existing_mod".to_string(),
                Range::with_file(1, 0, 50, 0, "src/existing_mod.rs".to_string()),
            ),
            Node::function(
                "test_crate::existing_mod::my_func".to_string(),
                Range::with_file(10, 0, 20, 0, "src/existing_mod.rs".to_string()),
            ),
        ];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (new_nodes, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        // Should NOT create any new Module nodes (only Crate, File, Folder)
        let new_module_nodes: Vec<_> = new_nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module)
            .collect();

        assert!(
            new_module_nodes.is_empty(),
            "Should not create duplicate module; got: {:?}",
            new_module_nodes.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );

        // Original module should still be used in the edge (High confidence preserved)
        let original_mod = &nodes[0];
        let func_node = &nodes[1];

        let mod_to_func_edge = new_edges
            .iter()
            .find(|e| e.from_id == original_mod.id && e.to_id == func_node.id);

        assert!(
            mod_to_func_edge.is_some(),
            "Should create edge using original module node"
        );

        // Original module retains its High confidence (provenance unchanged)
        assert_eq!(
            original_mod.provenance.confidence,
            Confidence::High,
            "Original module should retain High confidence"
        );
    }

    #[test]
    fn test_module_to_module_containment_with_auto_created() {
        // Test: Nested function triggers auto-creation; Module→Module edges should form
        let nodes = vec![
            // Explicit parent module
            Node::module(
                "test_crate::parent".to_string(),
                Range::with_file(1, 0, 100, 0, "src/parent.rs".to_string()),
            ),
            // Function in nested module (child module NOT declared)
            Node::function(
                "test_crate::parent::child::nested_func".to_string(),
                Range::with_file(10, 0, 20, 0, "src/parent/child.rs".to_string()),
            ),
        ];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (new_nodes, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        // Should auto-create "test_crate::parent::child" module
        let child_module = new_nodes
            .iter()
            .find(|n| n.kind == NodeKind::Module && n.fqn == "test_crate::parent::child");

        assert!(child_module.is_some(), "Expected auto-created child module");

        let child_mod = child_module.unwrap();
        let parent_mod = &nodes[0];

        // Should have Module → Module edge (parent → child)
        let mod_to_mod_edge = new_edges
            .iter()
            .find(|e| e.from_id == parent_mod.id && e.to_id == child_mod.id);

        assert!(
            mod_to_mod_edge.is_some(),
            "Expected Module → Module edge from parent to auto-created child"
        );

        // Should have Module → Function edge (child → func)
        let func_node = &nodes[1];
        let child_to_func_edge = new_edges
            .iter()
            .find(|e| e.from_id == child_mod.id && e.to_id == func_node.id);

        assert!(
            child_to_func_edge.is_some(),
            "Expected Module → Function edge from auto-created child to function"
        );
    }

    #[test]
    fn test_interface_containment() {
        // Test: Interface nodes should have Module → Interface containment edges
        let nodes = vec![
            Node::module(
                "test_crate::schemas".to_string(),
                Range::with_file(1, 0, 100, 0, "src/schemas.ts".to_string()),
            ),
            Node::new(
                NodeKind::Interface,
                "test_crate::schemas::UserSchema".to_string(),
                Range::with_file(10, 0, 20, 0, "src/schemas.ts".to_string()),
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            ),
            Node::new(
                NodeKind::Interface,
                "test_crate::schemas::ProductSchema".to_string(),
                Range::with_file(25, 0, 35, 0, "src/schemas.ts".to_string()),
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            ),
        ];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (_, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        // Find Module → Interface edges
        let mod_node = &nodes[0];
        let interface1 = &nodes[1];
        let interface2 = &nodes[2];

        let mod_to_interface1 = new_edges.iter().find(|e| {
            e.from_id == mod_node.id && e.to_id == interface1.id && e.kind == EdgeKind::Contains
        });
        let mod_to_interface2 = new_edges.iter().find(|e| {
            e.from_id == mod_node.id && e.to_id == interface2.id && e.kind == EdgeKind::Contains
        });

        assert!(
            mod_to_interface1.is_some(),
            "Expected Module → Interface edge for UserSchema"
        );
        assert!(
            mod_to_interface2.is_some(),
            "Expected Module → Interface edge for ProductSchema"
        );
    }

    #[test]
    fn test_type_alias_containment() {
        // Test: Type alias nodes should have Module → Type containment edges
        let nodes = vec![
            Node::module(
                "test_crate::types".to_string(),
                Range::with_file(1, 0, 100, 0, "src/types.ts".to_string()),
            ),
            Node::new(
                NodeKind::Type,
                "test_crate::types::UserID".to_string(),
                Range::with_file(10, 0, 10, 30, "src/types.ts".to_string()),
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            ),
        ];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (_, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        let mod_node = &nodes[0];
        let type_node = &nodes[1];

        let mod_to_type = new_edges.iter().find(|e| {
            e.from_id == mod_node.id && e.to_id == type_node.id && e.kind == EdgeKind::Contains
        });

        assert!(
            mod_to_type.is_some(),
            "Expected Module → Type edge for UserID type alias"
        );
    }

    #[test]
    fn test_enum_containment() {
        // Test: Enum nodes should have Module → Enum containment edges
        let nodes = vec![
            Node::module(
                "test_crate::enums".to_string(),
                Range::with_file(1, 0, 100, 0, "src/enums.ts".to_string()),
            ),
            Node::new(
                NodeKind::Enum,
                "test_crate::enums::Status".to_string(),
                Range::with_file(10, 0, 15, 0, "src/enums.ts".to_string()),
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            ),
        ];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (_, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        let mod_node = &nodes[0];
        let enum_node = &nodes[1];

        let mod_to_enum = new_edges.iter().find(|e| {
            e.from_id == mod_node.id && e.to_id == enum_node.id && e.kind == EdgeKind::Contains
        });

        assert!(
            mod_to_enum.is_some(),
            "Expected Module → Enum edge for Status enum"
        );
    }

    #[test]
    fn test_struct_containment() {
        // Test: Struct/Class nodes should have Module → Struct containment edges
        let nodes = vec![
            Node::module(
                "test_crate::models".to_string(),
                Range::with_file(1, 0, 100, 0, "src/models.ts".to_string()),
            ),
            Node::new(
                NodeKind::Struct,
                "test_crate::models::UserModel".to_string(),
                Range::with_file(10, 0, 50, 0, "src/models.ts".to_string()),
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            ),
        ];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (_, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        let mod_node = &nodes[0];
        let struct_node = &nodes[1];

        let mod_to_struct = new_edges.iter().find(|e| {
            e.from_id == mod_node.id && e.to_id == struct_node.id && e.kind == EdgeKind::Contains
        });

        assert!(
            mod_to_struct.is_some(),
            "Expected Module → Struct edge for UserModel class"
        );
    }

    #[test]
    fn test_all_symbol_types_get_containment() {
        // Test: All symbol types (Function, Interface, Type, Enum, Struct) should have containment
        // when they exist in the same module
        let nodes = vec![
            Node::module(
                "test_crate::mixed".to_string(),
                Range::with_file(1, 0, 200, 0, "src/mixed.ts".to_string()),
            ),
            Node::function(
                "test_crate::mixed::myFunction".to_string(),
                Range::with_file(10, 0, 20, 0, "src/mixed.ts".to_string()),
            ),
            Node::new(
                NodeKind::Interface,
                "test_crate::mixed::MyInterface".to_string(),
                Range::with_file(25, 0, 35, 0, "src/mixed.ts".to_string()),
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            ),
            Node::new(
                NodeKind::Type,
                "test_crate::mixed::MyType".to_string(),
                Range::with_file(40, 0, 40, 30, "src/mixed.ts".to_string()),
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            ),
            Node::new(
                NodeKind::Enum,
                "test_crate::mixed::MyEnum".to_string(),
                Range::with_file(45, 0, 55, 0, "src/mixed.ts".to_string()),
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            ),
            Node::new(
                NodeKind::Struct,
                "test_crate::mixed::MyClass".to_string(),
                Range::with_file(60, 0, 100, 0, "src/mixed.ts".to_string()),
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            ),
        ];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (_, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        let mod_node = &nodes[0];

        // Count containment edges from the module
        let containment_edges: Vec<_> = new_edges
            .iter()
            .filter(|e| e.from_id == mod_node.id && e.kind == EdgeKind::Contains)
            .collect();

        // Should have 5 containment edges: one for each symbol type
        assert_eq!(
            containment_edges.len(), 5,
            "Expected 5 Module → Symbol containment edges (Function, Interface, Type, Enum, Struct), got {}",
            containment_edges.len()
        );

        // Verify each symbol has a containment edge
        for node in &nodes[1..] {
            let has_containment = containment_edges.iter().any(|e| e.to_id == node.id);
            assert!(
                has_containment,
                "Expected containment edge for {:?} node '{}'",
                node.kind, node.fqn
            );
        }
    }

    #[test]
    fn test_auto_create_module_for_orphan_interface() {
        // Test: Interface without explicit module should trigger auto-creation
        let nodes = vec![Node::new(
            NodeKind::Interface,
            "test_crate::orphan_mod::OrphanInterface".to_string(),
            Range::with_file(5, 0, 15, 0, "src/orphan_mod.ts".to_string()),
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        )];

        let syntax_results = crate::application::ports::SyntaxResults::new();
        let (new_nodes, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        // Should auto-create the missing module "test_crate::orphan_mod"
        let auto_created_module = new_nodes
            .iter()
            .find(|n| n.kind == NodeKind::Module && n.fqn == "test_crate::orphan_mod");

        assert!(
            auto_created_module.is_some(),
            "Expected auto-created module 'test_crate::orphan_mod' for orphan interface"
        );

        // Verify Module → Interface edge was created
        let module = auto_created_module.unwrap();
        let interface_node = &nodes[0];
        let mod_to_interface = new_edges
            .iter()
            .find(|e| e.from_id == module.id && e.to_id == interface_node.id);

        assert!(
            mod_to_interface.is_some(),
            "Expected Module → Interface edge from auto-created module"
        );
    }

    /// Sprint E.6 regression.
    ///
    /// Before the fix, for Apex the containment builder would walk up
    /// every `::`-separated segment of the symbol FQN and synthesize
    /// ancestor Module nodes (`force_app`, `force_app::main`, ...).
    /// The topmost ancestor (`force_app`) had no FQN parent, so the
    /// File→Module fallback pinned it to whichever file was scanned
    /// first alphabetically — producing the infamous
    /// `File(PagedResult.cls) → Module(force_app)` edge reported in
    /// `docs/workstreams/apex/VALIDATION_RESULTS.md`.
    ///
    /// After the fix, for path-based-FQN Apex:
    ///   * No synthetic ancestor Modules are created.
    ///   * Every symbol attaches directly to its file's
    ///     `__file_module__` Module.
    ///   * The only File→Module edges are File → `__file_module__`,
    ///     one per file.
    #[test]
    fn apex_file_module_is_the_container_not_synthetic_ancestors() {
        // Simulate treesitter.rs output for two Apex files in a
        // dreamhouse-lwc-shaped layout. We set up the nodes by hand
        // because ContainmentBuilder runs after parsing.
        let paged_file = "/repo/force-app/main/default/classes/PagedResult.cls".to_string();
        let prop_file = "/repo/force-app/main/default/classes/PropertyController.cls".to_string();

        // Synthetic `__file_module__` nodes produced by treesitter.rs
        // (one per file, marked with the `file_module` property).
        let mut paged_fm = Node::new(
            NodeKind::Module,
            "force_app::main::default::classes::PagedResult::__file_module__".to_string(),
            Range::with_file(1, 0, 20, 0, paged_file.clone()),
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::Low),
        );
        paged_fm.set_property("file_module", serde_json::Value::Bool(true));

        let mut prop_fm = Node::new(
            NodeKind::Module,
            "force_app::main::default::classes::PropertyController::__file_module__".to_string(),
            Range::with_file(1, 0, 80, 0, prop_file.clone()),
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::Low),
        );
        prop_fm.set_property("file_module", serde_json::Value::Bool(true));

        // The two class Structs with path-based FQNs that exercise the
        // old-walk bug.
        let paged_class = Node::new(
            NodeKind::Struct,
            "force_app::main::default::classes::PagedResult::PagedResult".to_string(),
            Range::with_file(3, 0, 20, 0, paged_file.clone()),
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        );
        let prop_class = Node::new(
            NodeKind::Struct,
            "force_app::main::default::classes::PropertyController::PropertyController".to_string(),
            Range::with_file(3, 0, 80, 0, prop_file.clone()),
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        );

        let nodes = vec![
            paged_fm.clone(),
            prop_fm.clone(),
            paged_class.clone(),
            prop_class.clone(),
        ];

        let mut syntax_results = crate::application::ports::SyntaxResults::new();
        syntax_results.set_language("apex".to_string());
        syntax_results.set_workspace_root("/repo".to_string());
        syntax_results.set_source_files(vec![paged_file.clone(), prop_file.clone()]);

        let (new_nodes, new_edges) = ContainmentBuilder::build_containment(&nodes, &syntax_results);

        // Invariant 1: no synthetic ancestor Module nodes like
        // `force_app`, `force_app::main`, etc.
        let offending: Vec<_> = new_nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module)
            .filter(|n| !n.fqn.ends_with("__file_module__"))
            .collect();
        assert!(
            offending.is_empty(),
            "Apex must not emit synthetic ancestor Module nodes, got: {:?}",
            offending.iter().map(|n| &n.fqn).collect::<Vec<_>>()
        );

        // Invariant 2: every struct is contained by its own file's
        // `__file_module__`, not by some arbitrary ancestor.
        for (cls, fm) in [(&paged_class, &paged_fm), (&prop_class, &prop_fm)] {
            let edge = new_edges
                .iter()
                .find(|e| e.kind == EdgeKind::Contains && e.from_id == fm.id && e.to_id == cls.id);
            assert!(
                edge.is_some(),
                "Expected {} → {} Contains edge; edges: {:?}",
                fm.fqn,
                cls.fqn,
                new_edges
                    .iter()
                    .filter(|e| e.to_id == cls.id)
                    .map(|e| (e.from_id.clone(), e.kind))
                    .collect::<Vec<_>>()
            );
        }

        // Invariant 3: no File → Module edge targets anything other
        // than a `__file_module__` Module. This is the PagedResult.cls
        // anomaly repro.
        let file_ids: std::collections::HashSet<String> = new_nodes
            .iter()
            .filter(|n| n.kind == NodeKind::File)
            .map(|n| n.id.clone())
            .collect();
        let module_fqn_by_id: std::collections::HashMap<String, String> = new_nodes
            .iter()
            .chain(nodes.iter())
            .filter(|n| n.kind == NodeKind::Module)
            .map(|n| (n.id.clone(), n.fqn.clone()))
            .collect();
        for edge in &new_edges {
            if edge.kind != EdgeKind::Contains {
                continue;
            }
            if !file_ids.contains(&edge.from_id) {
                continue;
            }
            if let Some(fqn) = module_fqn_by_id.get(&edge.to_id) {
                assert!(
                    fqn.ends_with("__file_module__"),
                    "File→Module target must be a __file_module__ for Apex, got {}",
                    fqn
                );
            }
        }
    }
}
