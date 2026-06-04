//! [`AnalysisGraph`] — the assembled in-memory graph + the methods
//! that traverse it.
//!
//! Split out of `health/graph.rs` by R1. The struct holds adjacency
//! lists, containment tree, precomputed subsets (clean / production /
//! high-only-production edge index sets), and module-membership
//! caches. Construction is two-phase:
//!
//! 1. [`AnalysisGraph::load`] / `load_with_*` — pull the raw node
//!    and edge collections from a parse DB (delegated to
//!    [`super::loader`]) and call [`AnalysisGraph::build`] to
//!    materialise this struct.
//! 2. [`AnalysisGraph::finalize_production_edges`] — must run AFTER
//!    `structural_classification::propagate` has stamped `is_test`
//!    on File nodes; otherwise the production edge set will be
//!    empty and every Layer-3 metric will degrade silently.
//!    [`AnalysisGraph::validate_invariants`] exists specifically to
//!    catch that ordering bug.
//!
//! `is_synthetic_node`, `classification_of`, and `is_non_production_node`
//! live in [`super::classification`] — they're predicates rather than
//! graph mechanics. Per-node language / framework propagation lives in
//! [`super::language`] — it's a load-time pass over `nodes`, not a
//! property of the graph as a runtime object.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use super::classification::is_synthetic_node;
use super::language::propagate_file_metadata_to_descendants;
use super::loader::{load_edges, load_nodes};
use super::types::{Confidence, EdgeKind, GraphEdge, GraphNode, NodeKind};

pub struct AnalysisGraph {
    pub nodes: BTreeMap<String, GraphNode>,
    pub edges: Vec<GraphEdge>,

    // Adjacency: non-containment edges only
    pub outgoing: HashMap<String, Vec<usize>>,
    pub incoming: HashMap<String, Vec<usize>>,

    // Containment tree
    pub containment_parent: HashMap<String, String>,
    pub containment_children: HashMap<String, Vec<String>>,

    // Precomputed subsets
    pub structural_edge_indices: Vec<usize>,
    /// Structural edges excluding any edge that touches a synthetic node.
    pub clean_structural_edge_indices: Vec<usize>,
    /// Structural edges restricted to production code only (excludes edges where
    /// either endpoint is a test/example/benchmark/vendor/generated node).
    /// Built lazily via `finalize_production_edges()` after classification propagation.
    pub production_structural_edge_indices: Vec<usize>,
    /// Subset of `production_structural_edge_indices` whose edges carry
    /// `Confidence::High`. This is the "authoritative graph" used by T3
    /// dual-metric emission: every Layer-3 metric is computed once over
    /// the full production edge set and once over this high-only subset,
    /// and the report surfaces the difference as the metric's
    /// `fidelity_gap`. Built alongside production edges in
    /// `finalize_production_edges()`.
    pub high_only_production_structural_edge_indices: Vec<usize>,
    pub function_node_ids: Vec<String>,
    pub module_node_ids: Vec<String>,
    /// Analysis-level module keys (path prefixes at the configured depth).
    /// For TypeScript depth=2: `src/middleware`, `src/adapter`, etc.
    pub folder_module_ids: Vec<String>,

    // Module membership cache: node_id -> nearest module ancestor id
    module_membership: HashMap<String, String>,
    /// node_id -> analysis-level module key (path prefix at configured depth)
    folder_membership: HashMap<String, String>,
    /// module_key -> set of member node IDs (pre-computed for coupling analysis)
    analysis_module_members: HashMap<String, HashSet<String>>,
    /// file_path -> File node ID (for classification lookup by location).
    /// `pub(super)` so [`super::classification`] can resolve a node's
    /// owning File without an extra getter — the classification impl
    /// is one half of this graph's behaviour, just kept in a sibling
    /// file for readability.
    pub(super) file_path_index: HashMap<String, String>,

    /// Maximum path depth for analysis-level modules.
    /// Folders deeper than this are collapsed into their ancestor at this depth.
    analysis_module_depth: usize,

    /// When true, strip Maven/Gradle build-convention directories
    /// (`src/main/java/`, `src/test/java/`, etc.) from paths before computing
    /// analysis module boundaries.
    strip_build_dirs: bool,

    /// Count of SQLite edge rows whose `kind` column did not
    /// deserialise into a known `EdgeKind`. Populated at load time
    /// from `load_edges`; consumed by the health pipeline to emit
    /// `CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1` when non-zero. Always zero
    /// for a DB written by the current engine.
    unknown_edge_kind_count: usize,
}

impl AnalysisGraph {
    /// Placeholder graph for pipeline bootstrap before `GraphPrep` loads the DB.
    pub fn empty() -> Self {
        Self {
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
            containment_parent: HashMap::new(),
            containment_children: HashMap::new(),
            structural_edge_indices: Vec::new(),
            clean_structural_edge_indices: Vec::new(),
            production_structural_edge_indices: Vec::new(),
            high_only_production_structural_edge_indices: Vec::new(),
            function_node_ids: Vec::new(),
            module_node_ids: Vec::new(),
            folder_module_ids: Vec::new(),
            module_membership: HashMap::new(),
            folder_membership: HashMap::new(),
            analysis_module_members: HashMap::new(),
            file_path_index: HashMap::new(),
            analysis_module_depth: 0,
            strip_build_dirs: false,
            unknown_edge_kind_count: 0,
        }
    }

    /// Load a graph from an open SQLite connection.
    pub fn load(conn: &Connection) -> Result<Self> {
        let nodes = load_nodes(conn)?;
        let mut unknown_kind_count = 0usize;
        let edges = load_edges(conn, &nodes, &mut unknown_kind_count)?;

        let mut graph = Self::build(nodes, edges);
        graph.unknown_edge_kind_count = unknown_kind_count;
        graph.compute_module_membership();
        Ok(graph)
    }

    /// Load with a custom analysis module depth and optional build-dir stripping.
    pub fn load_with_depth(conn: &Connection, depth: usize) -> Result<Self> {
        Self::load_with_module_config(conn, depth, false)
    }

    /// Load with full module config: depth + build-convention directory stripping.
    pub fn load_with_module_config(
        conn: &Connection,
        depth: usize,
        strip_build_dirs: bool,
    ) -> Result<Self> {
        let nodes = load_nodes(conn)?;
        let mut unknown_kind_count = 0usize;
        let edges = load_edges(conn, &nodes, &mut unknown_kind_count)?;

        let mut graph = Self::build(nodes, edges);
        graph.unknown_edge_kind_count = unknown_kind_count;
        graph.analysis_module_depth = depth;
        graph.strip_build_dirs = strip_build_dirs;
        graph.compute_module_membership();
        Ok(graph)
    }

    /// Counts edges whose `edges.kind` column value did not match any
    /// known `EdgeKind` variant (serde tagged form). Populated by
    /// `load_edges`; surfaced by the pipeline as
    /// `CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1` in `integrity_status`
    /// whenever non-zero. A parse DB written by the current engine
    /// always leaves this at zero; non-zero values mean the DB was
    /// written by a newer engine version and the analysis is
    /// measuring a strict subset of the intended graph.
    pub fn unknown_edge_kind_count(&self) -> usize {
        self.unknown_edge_kind_count
    }

    pub(crate) fn build(nodes: BTreeMap<String, GraphNode>, edges: Vec<GraphEdge>) -> Self {
        let mut outgoing: HashMap<String, Vec<usize>> = HashMap::new();
        let mut incoming: HashMap<String, Vec<usize>> = HashMap::new();
        let mut containment_parent: HashMap<String, String> = HashMap::new();
        let mut containment_children: HashMap<String, Vec<String>> = HashMap::new();
        let mut structural_edge_indices: Vec<usize> = Vec::new();

        for (idx, edge) in edges.iter().enumerate() {
            if edge.kind.is_containment() {
                containment_parent.insert(edge.to_id.clone(), edge.from_id.clone());
                containment_children
                    .entry(edge.from_id.clone())
                    .or_default()
                    .push(edge.to_id.clone());
            } else {
                outgoing.entry(edge.from_id.clone()).or_default().push(idx);
                incoming.entry(edge.to_id.clone()).or_default().push(idx);
                structural_edge_indices.push(idx);
            }
        }

        // Filter: exclude __file_scope__ / __file_module__ synthetic nodes from function list.
        // These are parser artifacts representing file-level code, not meaningful functions.
        let function_node_ids: Vec<String> = nodes
            .iter()
            .filter(|(_, n)| n.kind.is_function_like() && !is_synthetic_node(n))
            .map(|(id, _)| id.clone())
            .collect();

        let module_node_ids: Vec<String> = nodes
            .iter()
            .filter(|(_, n)| n.kind.is_module_like())
            .map(|(id, _)| id.clone())
            .collect();

        // All folder IDs — analysis-level filtering happens in compute_module_membership()
        // after analysis_module_depth is set.
        let folder_module_ids: Vec<String> = nodes
            .iter()
            .filter(|(_, n)| n.kind == NodeKind::Folder)
            .map(|(id, _)| id.clone())
            .collect();

        // Structural edges that don't touch any synthetic node — used for cycle detection
        let synthetic_ids: HashSet<&str> = nodes
            .iter()
            .filter(|(_, n)| is_synthetic_node(n))
            .map(|(id, _)| id.as_str())
            .collect();

        let clean_structural_edge_indices: Vec<usize> = structural_edge_indices
            .iter()
            .copied()
            .filter(|&ei| {
                let e = &edges[ei];
                !synthetic_ids.contains(e.from_id.as_str())
                    && !synthetic_ids.contains(e.to_id.as_str())
            })
            .collect();

        // Build file_path -> File node ID index for O(1) classification lookup
        let mut file_path_index: HashMap<String, String> = HashMap::new();
        for (id, node) in &nodes {
            if node.kind == NodeKind::File {
                if let Some(ref fp) = node.file_path {
                    file_path_index.insert(fp.clone(), id.clone());
                }
                if let Some(ref prr) = node.path_repo_rel {
                    file_path_index.insert(prr.clone(), id.clone());
                }
                file_path_index.insert(node.fqn.clone(), id.clone());
            }
        }

        let mut nodes = nodes;
        propagate_file_metadata_to_descendants(&mut nodes, &file_path_index);

        Self {
            nodes,
            edges,
            outgoing,
            incoming,
            containment_parent,
            containment_children,
            structural_edge_indices,
            clean_structural_edge_indices,
            production_structural_edge_indices: Vec::new(),
            high_only_production_structural_edge_indices: Vec::new(),
            function_node_ids,
            module_node_ids,
            folder_module_ids,
            module_membership: HashMap::new(),
            folder_membership: HashMap::new(),
            analysis_module_members: HashMap::new(),
            file_path_index,
            analysis_module_depth: 2,
            strip_build_dirs: false,
            unknown_edge_kind_count: 0,
        }
    }

    /// For each node, compute module membership.
    /// Analysis modules are defined by path prefixes at the configured depth,
    /// NOT by the containment tree (which may skip intermediate directories).
    pub(crate) fn compute_module_membership(&mut self) {
        let node_ids: Vec<String> = self.nodes.keys().cloned().collect();
        let mut cache: HashMap<String, String> = HashMap::new();

        for id in &node_ids {
            let module_id = self.resolve_module_for(id, &mut cache);
            if let Some(mid) = module_id {
                self.module_membership.insert(id.clone(), mid);
            }
        }

        // Path-prefix-based analysis module membership.
        let depth = self.analysis_module_depth;
        let do_strip = self.strip_build_dirs;
        let mut module_members: HashMap<String, HashSet<String>> = HashMap::new();

        for id in &node_ids {
            if let Some(raw_path) = self.resolve_path_for(id) {
                let path = if do_strip {
                    strip_build_convention_dirs(&raw_path)
                } else {
                    raw_path
                };
                let key = path_prefix(Some(&path), depth);
                if !key.is_empty() {
                    self.folder_membership.insert(id.clone(), key.clone());
                    module_members.entry(key).or_default().insert(id.clone());
                }
            }
        }

        let mut module_keys: Vec<String> = module_members.keys().cloned().collect();
        module_keys.sort();
        self.folder_module_ids = module_keys;
        self.analysis_module_members = module_members;
    }

    /// Resolve the repo-relative path for a node.
    /// File/Folder nodes have path_repo_rel directly.
    /// Other nodes inherit from their parent File via the file_path_index.
    pub fn resolve_path_for(&self, node_id: &str) -> Option<String> {
        let node = self.nodes.get(node_id)?;

        if let Some(ref prr) = node.path_repo_rel {
            if !prr.is_empty() {
                return Some(prr.clone());
            }
        }

        if let Some(ref fp) = node.file_path {
            if let Some(file_id) = self.file_path_index.get(fp) {
                if let Some(file_node) = self.nodes.get(file_id) {
                    if let Some(ref prr) = file_node.path_repo_rel {
                        return Some(prr.clone());
                    }
                }
            }
        }

        let mut current = node_id.to_string();
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current.clone()) {
                return None;
            }
            if let Some(parent_id) = self.containment_parent.get(&current) {
                if let Some(parent) = self.nodes.get(parent_id) {
                    if let Some(ref prr) = parent.path_repo_rel {
                        if !prr.is_empty() {
                            return Some(prr.clone());
                        }
                    }
                }
                current = parent_id.clone();
            } else {
                return None;
            }
        }
    }

    fn resolve_module_for(
        &self,
        node_id: &str,
        cache: &mut HashMap<String, String>,
    ) -> Option<String> {
        if let Some(cached) = cache.get(node_id) {
            return Some(cached.clone());
        }

        if let Some(node) = self.nodes.get(node_id) {
            if node.kind.is_module_like() {
                cache.insert(node_id.to_string(), node_id.to_string());
                return Some(node_id.to_string());
            }
        }

        let mut visited = HashSet::new();
        let mut current = node_id.to_string();
        loop {
            if !visited.insert(current.clone()) {
                return None;
            }
            if let Some(parent_id) = self.containment_parent.get(&current) {
                if let Some(parent) = self.nodes.get(parent_id) {
                    if parent.kind.is_module_like() {
                        cache.insert(node_id.to_string(), parent_id.clone());
                        return Some(parent_id.clone());
                    }
                }
                current = parent_id.clone();
            } else {
                return None;
            }
        }
    }

    /// Build the production-only edge index. Must be called AFTER structural
    /// classification propagation has set `is_test` on File nodes, so that
    /// `is_non_production_node()` returns accurate results.
    pub fn finalize_production_edges(&mut self) {
        self.production_structural_edge_indices = self
            .clean_structural_edge_indices
            .iter()
            .copied()
            .filter(|&ei| {
                let e = &self.edges[ei];
                !self.is_non_production_node(&e.from_id) && !self.is_non_production_node(&e.to_id)
            })
            .collect();

        // T3 dual-metric emission: `High`-confidence subset of the
        // production edge set. This is the graph that metrics are
        // re-computed on so the report can surface a fidelity gap.
        // `Medium` / `Low` / `Unknown` all drop out here — a heuristic
        // edge is by definition not an authoritative one, even if it
        // turns out to be correct.
        self.high_only_production_structural_edge_indices = self
            .production_structural_edge_indices
            .iter()
            .copied()
            .filter(|&ei| self.edges[ei].confidence.is_high())
            .collect();
    }

    /// Validate post-preparation invariants on the graph. Returns Err with a
    /// list of human-readable violation strings if any invariant is broken.
    ///
    /// These invariants exist to catch the class of silent-regression bugs
    /// that shipped a version of this pipeline emitting `cycles_found = 0`
    /// for every codebase because `production_structural_edge_indices` was
    /// consulted before `finalize_production_edges()` was called. A correct
    /// pipeline cannot land in a state where the clean edge set is non-empty
    /// but the production edge set is empty.
    ///
    /// The caller should run this AFTER `finalize_production_edges()`. In
    /// debug builds, a caller may `debug_assert!` on the result; in release
    /// builds, violations should be surfaced as analysis errors in the
    /// HealthReport rather than panicking.
    pub fn validate_invariants(&self) -> Result<(), Vec<String>> {
        let mut violations: Vec<String> = Vec::new();

        if !self.clean_structural_edge_indices.is_empty()
            && self.production_structural_edge_indices.is_empty()
        {
            violations.push(
                "production_structural_edge_indices is empty but clean structural edges exist \
                 — did finalize_production_edges() run before metric computation?"
                    .into(),
            );
        }

        if !self.function_node_ids.is_empty() && self.nodes.is_empty() {
            violations.push(
                "function_node_ids is non-empty but nodes map is empty — graph is in an \
                 inconsistent state."
                    .into(),
            );
        }

        let edge_count = self.edges.len();
        let edge_buckets: [(&str, &[usize]); 4] = [
            ("structural_edge_indices", &self.structural_edge_indices),
            (
                "clean_structural_edge_indices",
                &self.clean_structural_edge_indices,
            ),
            (
                "production_structural_edge_indices",
                &self.production_structural_edge_indices,
            ),
            (
                "high_only_production_structural_edge_indices",
                &self.high_only_production_structural_edge_indices,
            ),
        ];
        for (name, indices) in edge_buckets {
            if let Some(&bad) = indices.iter().find(|&&i| i >= edge_count) {
                violations.push(format!(
                    "{} contains out-of-range edge index {} (edges.len() = {})",
                    name, bad, edge_count
                ));
                break;
            }
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }

    /// Get the **language-level** module ID a node belongs to.
    ///
    /// "Language-level" here means the module identity the parser
    /// assigned at extraction time — for Rust, this is the Cargo
    /// crate + `mod` path; for TypeScript, the package + file
    /// boundary; for Python, the importable package path; for Apex,
    /// the namespace + class. It is the unit a programmer would
    /// reach for when they say "this function lives in module X."
    ///
    /// Pairs with [`Self::folder_of`] which returns the
    /// **analysis-level** module key (folder-grouped, used by
    /// coupling / hub analysis and the `top_coupled` query). Both
    /// pulls are O(1) HashMap lookups on indexes the constructor
    /// builds eagerly.
    ///
    /// # Why this exists despite no production callers (yet)
    ///
    /// R5 dogfood (P3 cross-check) found this method has zero
    /// production callers as of v0.1.0-rc1 — `folder_of` is the
    /// one the priority engine reaches for today. We deliberately
    /// keep `module_of` rather than delete because:
    ///
    /// 1. It pairs with `folder_of` and
    ///    [`Self::analysis_module_members_of`] as the public
    ///    membership-query API on `AnalysisGraph`. Removing one
    ///    leg of that trio is a DRY violation in the API surface
    ///    — callers reading the type ask "where's the
    ///    language-level lookup?" and don't find it.
    /// 2. The integration test
    ///    `graphengine-analysis/tests/integration_test.rs` pins
    ///    its contract, so its behaviour is locked even without
    ///    a production caller.
    /// 3. The natural callers for v0.1.0 final are sitting in the
    ///    backlog: per-module risk roll-up (Q? not yet filed) and
    ///    "is this dead because its module is dead?" detection
    ///    would both reach for `module_of` over `folder_of` (the
    ///    user-facing narrative wants "lives in module X", not
    ///    "lives in folder X").
    ///
    /// If a future audit re-confirms zero production callers AND
    /// the planned consumers above are descoped, this is safe to
    /// delete together with `module_membership`, the constructor
    /// path that populates it, and the test assertion.
    pub fn module_of(&self, node_id: &str) -> Option<&String> {
        self.module_membership.get(node_id)
    }

    /// Get the analysis-level module key for coupling/hub analysis.
    ///
    /// Returns the folder-grouped module key the analysis layer
    /// uses for cross-cutting metrics (coupling, hubs, hotspots).
    /// This is **distinct** from [`Self::module_of`], which returns
    /// the language-level module identity. For most user-facing
    /// "where does this function live" narratives, prefer
    /// `module_of`; for "which group is hot", prefer `folder_of`.
    pub fn folder_of(&self, node_id: &str) -> Option<&String> {
        self.folder_membership.get(node_id)
    }

    /// Get all node IDs belonging to an analysis module (by module key).
    pub fn analysis_module_members_of(&self, module_key: &str) -> Option<&HashSet<String>> {
        self.analysis_module_members.get(module_key)
    }

    /// Get outgoing structural (non-containment) edges for a node.
    pub fn structural_outgoing(&self, node_id: &str) -> Vec<&GraphEdge> {
        self.outgoing
            .get(node_id)
            .map(|indices| indices.iter().map(|&i| &self.edges[i]).collect())
            .unwrap_or_default()
    }

    /// Get incoming structural (non-containment) edges for a node.
    pub fn structural_incoming(&self, node_id: &str) -> Vec<&GraphEdge> {
        self.incoming
            .get(node_id)
            .map(|indices| indices.iter().map(|&i| &self.edges[i]).collect())
            .unwrap_or_default()
    }

    /// Fan-in for a node (count of incoming structural edges).
    pub fn fan_in(&self, node_id: &str) -> usize {
        self.incoming.get(node_id).map(|v| v.len()).unwrap_or(0)
    }

    /// Fan-out for a node (count of outgoing structural edges).
    pub fn fan_out(&self, node_id: &str) -> usize {
        self.outgoing.get(node_id).map(|v| v.len()).unwrap_or(0)
    }

    /// Fan-in counting only `Confidence::High` edges. Used by T3 dual-
    /// metric emission for dead-code detection: the high-only view of
    /// dead code asks "how many functions have zero callers *if we
    /// trust only authoritative call edges*?". Almost always larger
    /// than `fan_in()` because heuristic calls drop out.
    pub fn fan_in_high_only(&self, node_id: &str) -> usize {
        self.incoming
            .get(node_id)
            .map(|v| {
                v.iter()
                    .filter(|&&ei| self.edges[ei].confidence.is_high())
                    .count()
            })
            .unwrap_or(0)
    }

    /// Fan-out counting only `Confidence::High` edges.
    pub fn fan_out_high_only(&self, node_id: &str) -> usize {
        self.outgoing
            .get(node_id)
            .map(|v| {
                v.iter()
                    .filter(|&&ei| self.edges[ei].confidence.is_high())
                    .count()
            })
            .unwrap_or(0)
    }

    /// Total non-containment edges.
    pub fn total_structural_edges(&self) -> usize {
        self.structural_edge_indices.len()
    }

    /// Confidence breakdown across every loaded edge, regardless of kind
    /// or production status. This is the denominator for whole-graph
    /// reporting. Prefer `call_edges_by_confidence` when the question
    /// is specifically "how trustworthy is my call graph" — containment
    /// edges always carry `High` and will inflate the whole-graph ratio.
    pub fn all_edges_by_confidence(&self) -> crate::health::report::EdgesByConfidence {
        breakdown_for_edges(&self.edges, self.edges.iter().enumerate().map(|(i, _)| i))
    }

    /// Confidence breakdown restricted to call-like edges
    /// (`Call`, `Framework(_)`, `Declarative(_)`). This is the
    /// denominator T4's `MeasuredFidelityTier` is computed from:
    /// "is the *call graph* authoritative?" is the honest question.
    /// Non-production edges are included so the measurement reflects
    /// resolver behaviour on the full corpus the parser emitted.
    pub fn call_edges_by_confidence(&self) -> crate::health::report::EdgesByConfidence {
        let iter = self
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind.is_call_like())
            .map(|(i, _)| i);
        breakdown_for_edges(&self.edges, iter)
    }

    /// Containment depth from root for a node.
    pub fn containment_depth(&self, node_id: &str) -> usize {
        let mut depth = 0;
        let mut current = node_id.to_string();
        let mut visited = HashSet::new();
        while let Some(parent) = self.containment_parent.get(&current) {
            if !visited.insert(current.clone()) {
                break;
            }
            depth += 1;
            current = parent.clone();
        }
        depth
    }

    /// Get all node IDs that are contained (transitively) within a module.
    pub fn descendants_of(&self, module_id: &str) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut stack = vec![module_id.to_string()];
        while let Some(id) = stack.pop() {
            if let Some(children) = self.containment_children.get(&id) {
                for child in children {
                    if result.insert(child.clone()) {
                        stack.push(child.clone());
                    }
                }
            }
        }
        result
    }

    /// Summary counts for the report.
    pub fn total_nodes(&self) -> usize {
        self.nodes.len()
    }

    pub fn total_edges(&self) -> usize {
        self.edges.len()
    }

    pub fn total_functions(&self) -> usize {
        self.function_node_ids.len()
    }

    pub fn total_modules(&self) -> usize {
        self.module_node_ids.len()
    }

    pub fn total_folder_modules(&self) -> usize {
        self.folder_module_ids.len()
    }

    /// Count of Import edges in the graph. Zero means cross-file resolution
    /// is missing, which degrades every metric that depends on cross-file
    /// connectivity (dead code, coupling, fan-in, blast radius).
    pub fn import_edge_count(&self) -> usize {
        self.structural_edge_indices
            .iter()
            .filter(|&&idx| self.edges[idx].kind == EdgeKind::Import)
            .count()
    }

    /// `true` when the graph lacks Import edges entirely — a reliable signal
    /// that cross-file resolution failed and many metrics will be degraded.
    pub fn lacks_cross_file_edges(&self) -> bool {
        self.import_edge_count() == 0 && self.total_structural_edges() > 0
    }
}

/// Strip Maven/Gradle/Kotlin build-convention directory segments from a path.
///
/// Java/Kotlin projects nest source files under `src/{main,test}/{java,kotlin,resources}/`,
/// which inflates directory depth without representing real module structure.
/// Stripping these segments lets the analysis_depth parameter map to actual
/// package boundaries instead of build scaffolding.
///
/// Examples:
///   `gson/src/main/java/com/google/gson/Gson.java`  → `gson/com/google/gson/Gson.java`
///   `src/test/java/com/google/gson/GsonTest.java`    → `com/google/gson/GsonTest.java`
///   `app/src/main/kotlin/com/example/App.kt`         → `app/com/example/App.kt`
///   `src/main/resources/config.xml`                   → `config.xml`
///   `lib/util/Helper.java`                            → `lib/util/Helper.java` (unchanged)
fn strip_build_convention_dirs(path: &str) -> String {
    const PATTERNS: &[&[&str]] = &[
        &["src", "main", "java"],
        &["src", "test", "java"],
        &["src", "main", "kotlin"],
        &["src", "test", "kotlin"],
        &["src", "main", "resources"],
        &["src", "test", "resources"],
        &["src", "main", "scala"],
        &["src", "test", "scala"],
    ];

    let segments: Vec<&str> = path.split('/').collect();

    for pattern in PATTERNS {
        let plen = pattern.len();
        if let Some(pos) = segments.windows(plen).position(|w| w == *pattern) {
            let mut result: Vec<&str> = Vec::with_capacity(segments.len() - plen);
            result.extend_from_slice(&segments[..pos]);
            result.extend_from_slice(&segments[pos + plen..]);
            return result.join("/");
        }
    }

    path.to_string()
}

/// Count edges by confidence level for the given iterator of edge
/// indices. Pulled out of the inherent `impl AnalysisGraph` block so
/// both `all_edges_by_confidence` and `call_edges_by_confidence`
/// share one implementation.
fn breakdown_for_edges(
    edges: &[GraphEdge],
    indices: impl Iterator<Item = usize>,
) -> crate::health::report::EdgesByConfidence {
    use crate::health::report::EdgesByConfidence;
    let mut out = EdgesByConfidence::default();
    for i in indices {
        match edges[i].confidence {
            Confidence::High => out.high += 1,
            Confidence::Medium => out.medium += 1,
            Confidence::Low => out.low += 1,
            Confidence::Unknown => out.unknown += 1,
        }
    }
    out
}

/// Extract the directory-based module key at the given depth.
/// Strips the filename (last segment if it contains '.') before computing the prefix.
///
/// "src/middleware/cors/index.ts" depth=2 → "src/middleware"
/// "src/hono.ts"                 depth=2 → "src"
/// "src/router/linear-router/router.ts" depth=2 → "src/router"
/// "src/middleware" (directory)   depth=2 → "src/middleware"
/// "chi.go" (root-level file)    depth=2 → "."
/// Returns empty string for None/empty paths.
fn path_prefix(path: Option<&str>, depth: usize) -> String {
    match path {
        None | Some("") => String::new(),
        Some(p) => {
            let segments: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
            if segments.is_empty() {
                return String::new();
            }

            let dir_segments = if segments.last().map(|s| s.contains('.')).unwrap_or(false) {
                &segments[..segments.len() - 1]
            } else {
                &segments[..]
            };

            if dir_segments.is_empty() {
                return ".".to_string();
            }

            let take = dir_segments.len().min(depth);
            dir_segments[..take].join("/")
        }
    }
}
