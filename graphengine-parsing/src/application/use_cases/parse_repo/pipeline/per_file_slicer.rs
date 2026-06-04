//! Per-file slicing + reconstitution for `SyntaxResults`.
//!
//! The S1 incremental cache stores one row per discovered source file
//! containing the slice of [`SyntaxResults`] attributable to that file.
//! On rescan the orchestrator hashes every file, looks up the matching
//! cache row, and skips re-extraction for unchanged files. To make this
//! work the aggregate [`SyntaxResults`] returned by the extractor has
//! to be sliceable by `file_path` and re-mergeable without losing
//! information.
//!
//! This module provides the two halves of that contract:
//!
//! * [`slice_per_file`] — split an aggregate [`SyntaxResults`] into
//!   one [`PerFileSlice`] per source file (plus a scan-level
//!   [`ScanMetadata`] for items that don't belong to any one file).
//! * [`reconstitute_from_slices`] — given a map of per-file slices
//!   and a scan-level metadata block, return a [`SyntaxResults`]
//!   logically equivalent to the original.
//!
//! Round-trip correctness: slicing then reconstituting yields a
//! [`SyntaxResults`] with the **same set of items** as the original.
//! Per-vec order within each file may differ between the original and
//! the reconstituted aggregate because slicing groups by file (the
//! original extractor groups by file too, but the order of files
//! depends on the parallel-extraction schedule, which is not stable).
//! Downstream resolution + graph-building do not depend on per-vec
//! order — they hash by node id and sort by stable criteria.
//!
//! See `docs/02-strategy/S1_INCREMENTAL_SCANNING_DESIGN.md` §4.4–§4.6.

use std::collections::{BTreeMap, HashMap};

use super::super::super::super::ports::{
    FileExtractionCoverage, IdentifierUse, ImportSpec, LocalVarScope, ModDecl, SyntaxResults,
    TypeReference, UnresolvedReference,
};
use crate::domain::{Edge, Node, Range};

/// A bucket-name for items that cannot be attributed to a specific
/// file. Synthesized edges whose `from_id` doesn't match any node in
/// `symbols`, and class-symbols whose `api_name` doesn't match any
/// Class node, end up here. In practice this should always be empty;
/// the bucket exists so slicing is total (every input item ends up in
/// some slice) and the round-trip preserves them.
pub const ORPHAN_FILE_KEY: &str = "<orphan>";

/// One file's slice of the aggregate [`SyntaxResults`]. The fields
/// mirror [`SyntaxResults`] one-for-one except for `source_files`,
/// `workspace_root`, and `language`, which are scan-level and live
/// on [`ScanMetadata`] instead.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PerFileSlice {
    pub symbols: Vec<Node>,
    pub identifier_uses: Vec<IdentifierUse>,
    pub references: Vec<UnresolvedReference>,
    pub imports: Vec<Range>,
    pub type_refs: Vec<Range>,
    pub type_references: Vec<TypeReference>,
    pub import_specs: Vec<ImportSpec>,
    pub mod_decls: Vec<ModDecl>,
    pub synthesized_edges: Vec<Edge>,
    pub class_symbols: Vec<(String, String)>,
    pub local_var_scopes: Vec<LocalVarScope>,
    pub extraction_coverage: Option<FileExtractionCoverage>,
}

impl PerFileSlice {
    fn is_empty(&self) -> bool {
        self.symbols.is_empty()
            && self.identifier_uses.is_empty()
            && self.references.is_empty()
            && self.imports.is_empty()
            && self.type_refs.is_empty()
            && self.type_references.is_empty()
            && self.import_specs.is_empty()
            && self.mod_decls.is_empty()
            && self.synthesized_edges.is_empty()
            && self.class_symbols.is_empty()
            && self.local_var_scopes.is_empty()
            && self.extraction_coverage.is_none()
    }
}

/// Scan-level metadata that doesn't belong to any single file. Lives
/// alongside the slices in the cache so the reconstituted
/// [`SyntaxResults`] preserves the original aggregate's
/// `workspace_root` / `language` / `source_files`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ScanMetadata {
    pub source_files: Vec<String>,
    pub workspace_root: Option<String>,
    pub language: Option<String>,
}

/// Slice an aggregate [`SyntaxResults`] into one [`PerFileSlice`] per
/// source file plus a [`ScanMetadata`] block. Returns a
/// `BTreeMap<file_path, PerFileSlice>` for deterministic iteration
/// order.
///
/// Synthesized edges and class-symbols entries are attributed by
/// looking up the related node in `symbols`:
///
/// * `synthesized_edges`: keyed by `from_id`. If no node in
///   `symbols` has that id, the edge falls into the
///   [`ORPHAN_FILE_KEY`] bucket.
/// * `class_symbols`: keyed by case-insensitive match on the dotted
///   api name against any `Node` whose `kind` is `Class` (Apex) and
///   whose FQN matches. Orphans go to the [`ORPHAN_FILE_KEY`] bucket.
pub fn slice_per_file(results: &SyntaxResults) -> (BTreeMap<String, PerFileSlice>, ScanMetadata) {
    let mut slices: BTreeMap<String, PerFileSlice> = BTreeMap::new();

    // Build two lookups in a single pass:
    //   * node_id -> file_path  for synthesized-edge attribution.
    //   * lowercased FQN -> file_path  for class-symbol attribution.
    // The class_symbols vec is keyed by dotted api name (`Outer.Inner`)
    // which matches the corresponding type-declaration node's FQN
    // case-insensitively. Apex (the only producer today) emits class
    // declarations as `NodeKind::Struct` (and inner enums/interfaces
    // as the matching kinds), but we don't filter by kind here — any
    // node whose FQN equals the api_name owns the symbol entry, and
    // the lookup is robust to whatever kind the extractor picks.
    let mut node_file: HashMap<&str, &str> = HashMap::new();
    let mut fqn_file: HashMap<String, &str> = HashMap::new();
    for node in &results.symbols {
        node_file.insert(node.id.as_str(), node.location.file.as_str());
        fqn_file.insert(node.fqn.to_ascii_lowercase(), node.location.file.as_str());
    }

    for node in &results.symbols {
        slices
            .entry(node.location.file.clone())
            .or_default()
            .symbols
            .push(node.clone());
    }
    for item in &results.identifier_uses {
        slices
            .entry(item.location.file.clone())
            .or_default()
            .identifier_uses
            .push(item.clone());
    }
    for item in &results.references {
        let file = item.call_site().location.file.clone();
        slices
            .entry(file)
            .or_default()
            .references
            .push(item.clone());
    }
    for range in &results.imports {
        slices
            .entry(range.file.clone())
            .or_default()
            .imports
            .push(range.clone());
    }
    for range in &results.type_refs {
        slices
            .entry(range.file.clone())
            .or_default()
            .type_refs
            .push(range.clone());
    }
    for item in &results.type_references {
        slices
            .entry(item.location.file.clone())
            .or_default()
            .type_references
            .push(item.clone());
    }
    for item in &results.import_specs {
        slices
            .entry(item.source_file.clone())
            .or_default()
            .import_specs
            .push(item.clone());
    }
    for item in &results.mod_decls {
        slices
            .entry(item.source_file.clone())
            .or_default()
            .mod_decls
            .push(item.clone());
    }
    // Index symbols by id so we can copy endpoint nodes into consumer
    // slices. Apex managed-package `Import` edges are attributed to the
    // consumer `.cls` via `from_id`, but the virtual external `Module`
    // lives at `location.file = <external:managed_package>`. That
    // sentinel path is not a discovered source file, so its slice is
    // never written to `file_cache` — warm rescans reconstitute consumer
    // slices without the external module and graph validation fails with
    // a dangling `to_id`. Co-locate both endpoints in the consumer slice.
    let symbols_by_id: HashMap<&str, &Node> =
        results.symbols.iter().map(|n| (n.id.as_str(), n)).collect();
    for edge in &results.synthesized_edges {
        let bucket = node_file
            .get(edge.from_id.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| ORPHAN_FILE_KEY.to_string());
        let slice = slices.entry(bucket).or_default();
        slice.synthesized_edges.push(edge.clone());
        for endpoint_id in [&edge.from_id, &edge.to_id] {
            if slice.symbols.iter().any(|n| n.id == *endpoint_id) {
                continue;
            }
            if let Some(node) = symbols_by_id.get(endpoint_id.as_str()) {
                slice.symbols.push((*node).clone());
            }
        }
    }
    for (api_name, payload) in &results.class_symbols {
        let key = api_name.to_ascii_lowercase();
        let bucket = fqn_file
            .get(&key)
            .map(|s| s.to_string())
            .unwrap_or_else(|| ORPHAN_FILE_KEY.to_string());
        slices
            .entry(bucket)
            .or_default()
            .class_symbols
            .push((api_name.clone(), payload.clone()));
    }
    for scope in &results.local_var_scopes {
        slices
            .entry(scope.body.file.clone())
            .or_default()
            .local_var_scopes
            .push(scope.clone());
    }
    for record in &results.extraction_coverage {
        let key = record.file_path.to_string_lossy().into_owned();
        let slot = &mut slices.entry(key).or_default().extraction_coverage;
        if slot.is_some() {
            tracing::warn!(
                "multiple extraction_coverage records for the same file_path; keeping the first"
            );
        } else {
            *slot = Some(record.clone());
        }
    }

    let metadata = ScanMetadata {
        source_files: results.source_files.clone(),
        workspace_root: results.workspace_root.clone(),
        language: results.language.clone(),
    };

    slices.retain(|_, slice| !slice.is_empty());

    (slices, metadata)
}

/// Reconstitute an aggregate [`SyntaxResults`] from a per-file slice
/// map and scan-level metadata. The reconstituted aggregate contains
/// the same set of items as the original (vec order within each
/// component may differ — see module-level docs).
pub fn reconstitute_from_slices(
    slices: &BTreeMap<String, PerFileSlice>,
    metadata: &ScanMetadata,
) -> SyntaxResults {
    let mut out = SyntaxResults::new();
    out.source_files = metadata.source_files.clone();
    out.workspace_root = metadata.workspace_root.clone();
    out.language = metadata.language.clone();

    for slice in slices.values() {
        out.symbols.extend(slice.symbols.iter().cloned());
        out.identifier_uses
            .extend(slice.identifier_uses.iter().cloned());
        out.references.extend(slice.references.iter().cloned());
        out.imports.extend(slice.imports.iter().cloned());
        out.type_refs.extend(slice.type_refs.iter().cloned());
        out.type_references
            .extend(slice.type_references.iter().cloned());
        out.import_specs.extend(slice.import_specs.iter().cloned());
        out.mod_decls.extend(slice.mod_decls.iter().cloned());
        out.synthesized_edges
            .extend(slice.synthesized_edges.iter().cloned());
        out.class_symbols
            .extend(slice.class_symbols.iter().cloned());
        out.local_var_scopes
            .extend(slice.local_var_scopes.iter().cloned());
        if let Some(coverage) = &slice.extraction_coverage {
            out.extraction_coverage.push(coverage.clone());
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::{CallSite, TypeUsageKind};
    use crate::domain::node::NodeKind;
    use crate::domain::provenance::Provenance;
    use crate::domain::{Confidence, EdgeKind, ProvenanceSource};

    fn node(id: &str, kind: NodeKind, fqn: &str, file: &str) -> Node {
        Node {
            id: id.to_string(),
            kind,
            fqn: fqn.to_string(),
            location: Range::with_file(1, 0, 1, 10, file),
            provenance: Provenance::new(ProvenanceSource::Heuristic, Confidence::High),
            properties: Default::default(),
            trait_metadata: None,
        }
    }

    fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge::new(
            from.to_string(),
            to.to_string(),
            kind,
            Provenance::new(ProvenanceSource::Heuristic, Confidence::High),
        )
    }

    fn ident_use(name: &str, file: &str) -> IdentifierUse {
        IdentifierUse {
            location: Range::with_file(2, 0, 2, 8, file),
            name: name.to_string(),
        }
    }

    fn call_ref(name: &str, file: &str) -> UnresolvedReference {
        UnresolvedReference::Call(CallSite {
            location: Range::with_file(3, 0, 3, 12, file),
            function_name: name.to_string(),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        })
    }

    fn import_spec(file: &str, name: &str) -> ImportSpec {
        use crate::application::ports::{ImportKind, ImportPath, ImportVisibility, PathRoot};
        ImportSpec {
            range: Range::with_file(4, 0, 4, 10, file),
            path: ImportPath::new(PathRoot::Crate, vec![name.to_string()]),
            alias: None,
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: false,
            source_file: file.to_string(),
        }
    }

    fn type_ref(name: &str, file: &str) -> TypeReference {
        TypeReference {
            location: Range::with_file(5, 0, 5, 8, file),
            type_name: name.to_string(),
            usage_kind: TypeUsageKind::Parameter,
        }
    }

    #[test]
    fn slice_per_file_empty_input_returns_empty_map_and_empty_metadata() {
        let results = SyntaxResults::new();
        let (slices, meta) = slice_per_file(&results);
        assert!(slices.is_empty());
        assert!(meta.source_files.is_empty());
        assert!(meta.workspace_root.is_none());
        assert!(meta.language.is_none());
    }

    #[test]
    fn slice_per_file_attributes_nodes_by_location_file() {
        let mut results = SyntaxResults::new();
        results.symbols = vec![
            node("a-id", NodeKind::Function, "a::foo", "/a.rs"),
            node("b-id", NodeKind::Function, "b::bar", "/b.rs"),
            node("a-id-2", NodeKind::Function, "a::baz", "/a.rs"),
        ];

        let (slices, _) = slice_per_file(&results);
        assert_eq!(slices["/a.rs"].symbols.len(), 2);
        assert_eq!(slices["/b.rs"].symbols.len(), 1);
    }

    #[test]
    fn slice_per_file_attributes_synthesized_edges_via_from_id() {
        let mut results = SyntaxResults::new();
        let src = node("from-id", NodeKind::Module, "/a.rs", "/a.rs");
        results.symbols.push(src);
        results
            .synthesized_edges
            .push(edge("from-id", "target-module", EdgeKind::Import));

        let (slices, _) = slice_per_file(&results);
        assert_eq!(slices["/a.rs"].synthesized_edges.len(), 1);
    }

    #[test]
    fn slice_per_file_orphan_synthesized_edge_lands_in_orphan_bucket() {
        let mut results = SyntaxResults::new();
        results
            .synthesized_edges
            .push(edge("unknown-from", "target", EdgeKind::Import));

        let (slices, _) = slice_per_file(&results);
        assert_eq!(slices[ORPHAN_FILE_KEY].synthesized_edges.len(), 1);
    }

    #[test]
    fn slice_per_file_copies_external_module_into_consumer_slice_for_warm_cache() {
        use crate::syntax::language::apex::managed_packages::{
            synthesize_import_edge, synthesize_module_node, VIRTUAL_MANAGED_MODULE_FILE_SENTINEL,
        };

        let consumer = node(
            "consumer-module",
            NodeKind::Module,
            "fixture::Consumer::__file_module__",
            "/repo/Consumer.cls",
        );
        let external = synthesize_module_node("npsp");
        assert_eq!(external.location.file, VIRTUAL_MANAGED_MODULE_FILE_SENTINEL);

        let mut results = SyntaxResults::new();
        results.symbols.push(consumer.clone());
        results.symbols.push(external.clone());
        results.synthesized_edges.push(synthesize_import_edge(
            consumer.id.clone(),
            external.id.clone(),
        ));

        let (slices, _) = slice_per_file(&results);
        let consumer_slice = &slices["/repo/Consumer.cls"];
        assert_eq!(consumer_slice.synthesized_edges.len(), 1);
        assert!(
            consumer_slice
                .symbols
                .iter()
                .any(|n| n.id == external.id),
            "external module must be cached with the consumer file so warm reconstitution is self-contained"
        );
    }

    #[test]
    fn slice_round_trip_preserves_synthesized_edge_endpoints() {
        use crate::syntax::language::apex::managed_packages::{
            synthesize_import_edge, synthesize_module_node,
        };

        let consumer = node(
            "consumer-module",
            NodeKind::Module,
            "fixture::Consumer::__file_module__",
            "/repo/Consumer.cls",
        );
        let external = synthesize_module_node("npsp");

        let mut results = SyntaxResults::new();
        results.symbols.push(consumer.clone());
        results.symbols.push(external.clone());
        results.synthesized_edges.push(synthesize_import_edge(
            consumer.id.clone(),
            external.id.clone(),
        ));

        let (slices, meta) = slice_per_file(&results);
        let reconstituted = reconstitute_from_slices(&slices, &meta);
        let node_ids: std::collections::HashSet<&str> = reconstituted
            .symbols
            .iter()
            .map(|n| n.id.as_str())
            .collect();
        for edge in &reconstituted.synthesized_edges {
            assert!(node_ids.contains(edge.from_id.as_str()));
            assert!(node_ids.contains(edge.to_id.as_str()));
        }
    }

    #[test]
    fn slice_per_file_attributes_class_symbols_via_node_fqn() {
        // Apex emits class declarations as NodeKind::Struct with the
        // dotted api name as FQN; the class_symbols vec is keyed by
        // that same api name. Attribution joins on the FQN.
        let mut results = SyntaxResults::new();
        results
            .symbols
            .push(node("class-a", NodeKind::Struct, "MyClass", "/cls.cls"));
        results
            .class_symbols
            .push(("MyClass".to_string(), r#"{"k":1}"#.to_string()));

        let (slices, _) = slice_per_file(&results);
        assert_eq!(slices["/cls.cls"].class_symbols.len(), 1);
    }

    #[test]
    fn slice_per_file_class_symbol_lookup_is_case_insensitive() {
        // Apex identifiers are case-insensitive; the class node may
        // record "MyClass" while the symbols entry is keyed
        // "myclass" or vice versa. Lookup must succeed either way.
        let mut results = SyntaxResults::new();
        results
            .symbols
            .push(node("class-a", NodeKind::Struct, "MyClass", "/cls.cls"));
        results
            .class_symbols
            .push(("MYCLASS".to_string(), r#"{}"#.to_string()));

        let (slices, _) = slice_per_file(&results);
        assert_eq!(slices["/cls.cls"].class_symbols.len(), 1);
    }

    #[test]
    fn slice_per_file_attributes_each_collection_type_correctly() {
        let mut results = SyntaxResults::new();
        results
            .symbols
            .push(node("n", NodeKind::Function, "f", "/a.rs"));
        results.identifier_uses.push(ident_use("x", "/a.rs"));
        results.references.push(call_ref("call", "/a.rs"));
        results.import_specs.push(import_spec("/a.rs", "dep"));
        results.type_references.push(type_ref("MyType", "/a.rs"));
        results.imports.push(Range::with_file(1, 0, 1, 8, "/a.rs"));
        results
            .type_refs
            .push(Range::with_file(2, 0, 2, 8, "/a.rs"));

        let (slices, _) = slice_per_file(&results);
        let s = &slices["/a.rs"];
        assert_eq!(s.symbols.len(), 1);
        assert_eq!(s.identifier_uses.len(), 1);
        assert_eq!(s.references.len(), 1);
        assert_eq!(s.import_specs.len(), 1);
        assert_eq!(s.type_references.len(), 1);
        assert_eq!(s.imports.len(), 1);
        assert_eq!(s.type_refs.len(), 1);
    }

    #[test]
    fn slice_then_reconstitute_preserves_item_counts() {
        let mut results = SyntaxResults::new();
        results.symbols = vec![
            node("a", NodeKind::Function, "f1", "/a.rs"),
            node("b", NodeKind::Function, "f2", "/b.rs"),
        ];
        results.identifier_uses = vec![ident_use("x", "/a.rs"), ident_use("y", "/b.rs")];
        results.references = vec![call_ref("c1", "/a.rs"), call_ref("c2", "/b.rs")];
        results.workspace_root = Some("/workspace".to_string());
        results.language = Some("rust".to_string());
        results.source_files = vec!["/a.rs".to_string(), "/b.rs".to_string()];

        let (slices, meta) = slice_per_file(&results);
        let reconstituted = reconstitute_from_slices(&slices, &meta);

        assert_eq!(reconstituted.symbols.len(), results.symbols.len());
        assert_eq!(
            reconstituted.identifier_uses.len(),
            results.identifier_uses.len()
        );
        assert_eq!(reconstituted.references.len(), results.references.len());
        assert_eq!(reconstituted.workspace_root, results.workspace_root);
        assert_eq!(reconstituted.language, results.language);
        assert_eq!(reconstituted.source_files, results.source_files);
    }

    #[test]
    fn slice_then_reconstitute_preserves_node_ids_as_a_set() {
        let mut results = SyntaxResults::new();
        results.symbols = vec![
            node("id-1", NodeKind::Function, "f1", "/a.rs"),
            node("id-2", NodeKind::Function, "f2", "/b.rs"),
            node("id-3", NodeKind::Function, "f3", "/a.rs"),
        ];

        let (slices, meta) = slice_per_file(&results);
        let reconstituted = reconstitute_from_slices(&slices, &meta);

        let mut ids_in: Vec<&str> = results.symbols.iter().map(|n| n.id.as_str()).collect();
        let mut ids_out: Vec<&str> = reconstituted
            .symbols
            .iter()
            .map(|n| n.id.as_str())
            .collect();
        ids_in.sort();
        ids_out.sort();
        assert_eq!(ids_in, ids_out);
    }

    #[test]
    fn slice_per_file_skips_empty_slices() {
        // A SyntaxResults with no per-file items must produce an
        // empty slice map (only scan-level metadata in the
        // ScanMetadata block). This is the cheap-path case for a
        // pure scan-config change with no source files yet parsed.
        let mut results = SyntaxResults::new();
        results.workspace_root = Some("/ws".to_string());
        results.source_files = vec!["/a.rs".to_string()];

        let (slices, meta) = slice_per_file(&results);
        assert!(slices.is_empty());
        assert_eq!(meta.workspace_root, Some("/ws".to_string()));
    }

    #[test]
    fn slice_per_file_returns_btreemap_with_deterministic_order() {
        let mut results = SyntaxResults::new();
        results.symbols = vec![
            node("a", NodeKind::Function, "f", "/zzz.rs"),
            node("b", NodeKind::Function, "f", "/aaa.rs"),
            node("c", NodeKind::Function, "f", "/mmm.rs"),
        ];

        let (slices, _) = slice_per_file(&results);
        let keys: Vec<&String> = slices.keys().collect();
        assert_eq!(keys, vec!["/aaa.rs", "/mmm.rs", "/zzz.rs"]);
    }

    #[test]
    fn reconstitute_empty_slices_returns_empty_syntax_results_modulo_scan_meta() {
        let slices = BTreeMap::new();
        let meta = ScanMetadata {
            source_files: vec!["/a.rs".to_string()],
            workspace_root: Some("/ws".to_string()),
            language: Some("rust".to_string()),
        };
        let out = reconstitute_from_slices(&slices, &meta);
        assert!(out.symbols.is_empty());
        assert!(out.references.is_empty());
        assert_eq!(out.source_files, meta.source_files);
        assert_eq!(out.workspace_root, meta.workspace_root);
        assert_eq!(out.language, meta.language);
    }

    #[test]
    fn per_file_slice_round_trips_through_serde_json() {
        // The cache stores PerFileSlice as JSON in `file_cache.payload_json`.
        // Round-trip via serde_json must preserve every field.
        let slice = PerFileSlice {
            symbols: vec![node("a", NodeKind::Function, "f", "/a.rs")],
            identifier_uses: vec![ident_use("x", "/a.rs")],
            references: vec![call_ref("c", "/a.rs")],
            imports: vec![],
            type_refs: vec![],
            type_references: vec![type_ref("T", "/a.rs")],
            import_specs: vec![import_spec("/a.rs", "dep")],
            mod_decls: vec![],
            synthesized_edges: vec![],
            class_symbols: vec![("MyClass".to_string(), "{}".to_string())],
            local_var_scopes: vec![],
            extraction_coverage: None,
        };

        let json = serde_json::to_string(&slice).expect("serialise");
        let back: PerFileSlice = serde_json::from_str(&json).expect("deserialise");

        assert_eq!(back.symbols.len(), slice.symbols.len());
        assert_eq!(back.identifier_uses.len(), slice.identifier_uses.len());
        assert_eq!(back.references.len(), slice.references.len());
        assert_eq!(back.type_references.len(), slice.type_references.len());
        assert_eq!(back.import_specs.len(), slice.import_specs.len());
        assert_eq!(back.class_symbols.len(), slice.class_symbols.len());
    }
}
