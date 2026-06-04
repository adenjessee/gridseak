# ge-analyze: Full Specification for GraphEngine Team

**Date:** 2026-02-24
**Status:** Definitive specification. This is the instruction set for building the structural analysis engine.
**Owner:** GraphEngine (Rust)
**Consumer:** GridSeak UE (visualization), MCP Server (agent API), Audit Service (manual delivery)
**Priority:** This is the single gating dependency for all downstream work. Nothing else matters until this exists.

**Related documents:**
- `PHASE_4_STRUCTURAL_HEALTH.md` — Phase 4 overview (P4-1 through P4-6)
- `PHASE_6_DEEP_HEALTH_INTELLIGENCE.md` — Future extensions (complexity, temporal coupling, coverage, SARIF)
- `ARCHITECTURE.md` — System diagrams and data flow
- `docs/01-status/CURRENT_STATE.md` — Ground truth about what exists today

---

## 1. Purpose

`ge-analyze` computes structural health metrics from a parsed codebase graph stored in SQLite. It produces a JSON health report containing:
- A composite health score (0-100)
- Individual findings (cycles, hotspots, coupling problems, dead code)
- Per-node annotations (fan-in, fan-out, blast radius, risk level)
- Per-module annotations (coupling score, internal/external edge counts)

The analysis reveals emergent structural properties of the system that are:
- Invisible in text editors (you can't see circular dependencies by reading files)
- Invisible to AI chat tools (LLMs process text file-by-file, cannot compute graph metrics)
- Invisible until they cause incidents (coupling, blast radius, boundary violations are silent until a refactor goes sideways)

These are properties of the *system graph*, not any individual file.

---

## 2. Interface Contract

### 2.1 Command-Line Interface

```
ge-analyze --db <path-to-sqlite> --output <path-to-health-json>
```

**Required arguments:**

| Argument | Type | Description |
|----------|------|-------------|
| `--db` | File path | Path to SQLite database created by `graphengine-parsing`. Must exist and contain `nodes` and `edges` tables. |
| `--output` | File path | Path where the health JSON report will be written. Parent directory must exist. Overwrites if file exists. |

**Optional arguments (Phase 6 — design for extensibility, do not implement yet):**

| Argument | Type | Description | Phase |
|----------|------|-------------|-------|
| `--git-dir` | Directory path | Path to `.git` directory for temporal coupling analysis | P6-2 |
| `--coverage` | File path | Path to lcov coverage file for coverage import | P6-3 |
| `--sarif` | File path(s) | Path(s) to SARIF static analysis results | P6-4 |
| `--config` | File path | Path to analysis configuration TOML (thresholds, weights, exclusions) | Future |
| `--format` | `json` \| `json-pretty` | Output format. Default: `json-pretty` | P4-1 |

**Exit codes:**

| Code | Meaning |
|------|---------|
| 0 | Analysis completed successfully. Health JSON written. |
| 1 | Database not found or unreadable. |
| 2 | Database schema invalid (missing required tables/columns). |
| 3 | Output path not writable. |
| 4 | Analysis error (internal). Stderr contains details. |

**Stderr output:** Progress messages during analysis. Format: one line per phase.
```
[ge-analyze] Reading graph from database...
[ge-analyze] 847 nodes, 2341 edges loaded.
[ge-analyze] Running cycle detection...
[ge-analyze] Running fan-in/fan-out computation...
[ge-analyze] Running module coupling analysis...
[ge-analyze] Running dead code detection...
[ge-analyze] Running blast radius computation...
[ge-analyze] Running depth analysis...
[ge-analyze] Computing health score...
[ge-analyze] Health report written to /path/to/output.json
[ge-analyze] Health score: 61/100 | 9 findings | completed in 1.2s
```

### 2.2 Performance Requirements

| Metric | Requirement | Rationale |
|--------|-------------|-----------|
| 500-node graph | < 2 seconds | MVP demo responsiveness |
| 2,000-node graph | < 10 seconds | Typical medium TypeScript project |
| 10,000-node graph | < 60 seconds | Large TypeScript monorepo |
| Memory | < 500MB for 10,000 nodes | Desktop app constraint |

Cycle detection (Tarjan's SCC) is O(V + E). Fan-in/fan-out is O(V + E). Blast radius (reverse BFS per node) is O(V * (V + E)) worst case but should be optimized with caching. These bounds are well within the requirements for expected graph sizes.

---

## 3. SQLite Input Schema

`ge-analyze` reads from the SQLite database produced by `graphengine-parsing`. The relevant schema:

### 3.1 Actual Database Schema

**`nodes` table (direct columns):**

| Column | Type | Used For |
|--------|------|----------|
| `id` | TEXT (primary key) | Node identity (SHA-256 hash of FQN + location) |
| `kind` | TEXT NOT NULL | Node type (see complete enum below) |
| `fqn` | TEXT NOT NULL | Fully qualified name (e.g., `crate::module::function`) |
| `location` | TEXT NOT NULL | **JSON object**: `{"file": "...", "start_line": 10, "start_char": 0, "end_line": 20, "end_char": 1}` |
| `provenance` | TEXT NOT NULL | **JSON object**: `{"source": "Lsp", "confidence": "High"}` |
| `properties` | TEXT NOT NULL DEFAULT '{}' | **JSON object**: schemaless bag for classification data (see below) |
| `trait_metadata` | TEXT (nullable) | **JSON object** or NULL: trait method metadata |

**`properties` JSON keys (set by classification system during parsing):**

| Key | Type | Used For |
|-----|------|----------|
| `path_abs` | string | Absolute file path |
| `path_repo_rel` | string | Repository-relative file path (e.g., `src/auth/login.ts`) |
| `role` | string | Classification: `source`, `test`, `vendor`, `build_output`, `generated`, `docs`, `tooling`, `unknown` |
| `role_confidence` | string | `high`, `medium`, `low` |
| `role_reason` | string | Human-readable reason for classification |
| `is_test` | boolean | Whether this is a test file/function |
| `is_vendor` | boolean | Whether this is vendor/third-party code |
| `is_build_output` | boolean | Whether this is build output |
| `is_generated` | boolean | Whether this is generated code |
| `language` | string (optional) | Source language |

**Important:**
- There is no `name` column. Use `fqn` and extract a short name via `fqn.split("::").last()`.
- There is no `is_exported` column or property — dead code detection must rely on the containment-in-index-file heuristic.
- **Classification properties (`role`, `is_test`, `is_vendor`, `path_repo_rel`, etc.) are only set on File/Folder/Project nodes, NOT on Function nodes.** Function nodes have `properties: {}`. To determine a function's classification, walk up the containment tree to its parent File node and read classification from there. This is the same containment walk needed for module resolution (Section 3.3).

**`location` JSON keys:**

| Key | Type |
|-----|------|
| `file` | string (absolute path) |
| `start_line` | integer (1-based) |
| `start_char` | integer (0-based) |
| `end_line` | integer (1-based) |
| `end_char` | integer (0-based) |

**Complete `kind` enum (from `NodeKind` in `graphengine-parsing/src/domain/node.rs`):**

`Function`, `Struct`, `Module`, `Interface`, `Enum`, `Variable`, `Type`, `Import`, `Project`, `Crate`, `File`, `Folder`

Note: `Namespace` does not exist in the current domain model. `Project` and `Crate` are top-level container types. `Import` is a node representing an import statement.

**`edges` table:**

| Column | Type | Used For |
|--------|------|----------|
| `from_id` | TEXT NOT NULL (FK → nodes.id) | Edge source node |
| `to_id` | TEXT NOT NULL (FK → nodes.id) | Edge target node |
| `kind` | TEXT NOT NULL | Edge type: `Call`, `Import`, `Uses`, `Type`, `Contains` |
| `provenance` | TEXT NOT NULL | JSON provenance (same as nodes) |

Primary key: `(from_id, to_id, kind)` — prevents duplicate edges.

**Indexes:** `idx_nodes_fqn(fqn)`, `idx_nodes_kind(kind)`, `idx_edges_from(from_id)`, `idx_edges_to(to_id)`, `idx_edges_kind(kind)`

### 3.1.1 Graph Loading Strategy

Because classification data lives in the `properties` JSON blob and source locations live in the `location` JSON blob, the graph loader must:

1. Query `SELECT id, kind, fqn, location, properties FROM nodes`
2. For each row, deserialize `properties` as `serde_json::Value` and extract `path_repo_rel`, `role`, `is_test`, `is_vendor`, `is_build_output`, `is_generated`
3. Deserialize `location` as `serde_json::Value` and extract `start_line`, `end_line`
4. Extract a short `name` from `fqn` via `fqn.rsplit("::").next()`
5. Query `SELECT from_id, to_id, kind FROM edges`
6. Build the in-memory graph from these rows

### 3.2 Edge Type Semantics for Analysis

| Edge Type | Direction | Analysis Role |
|-----------|-----------|---------------|
| `Contains` | Parent → Child | Defines module hierarchy. **Excluded** from cycle detection, fan-in/out, coupling, dead code. Used only for module membership resolution. |
| `Call` | Caller → Callee | Primary structural edge. Used in cycle detection, fan-in/out, blast radius, dead code, depth. |
| `Import` | Importer → Imported | Structural dependency. Used in cycle detection, coupling, dead code. |
| `Uses` | User → Used | Type/value reference. Used in coupling, fan-in/out. |
| `Type` | Node → Type | Type annotation reference. Used in coupling. Lower weight than Call/Import. |

**Critical constraint:** All analysis algorithms operate on **non-containment edges only** unless explicitly stated otherwise. Containment edges define the hierarchy (which functions belong to which files, which files belong to which folders) — they are not structural dependencies.

### 3.3 Module Resolution

#### 3.3.1 Analysis Module Strategy

> **Empirical finding (2026-02-24):** Containment-tree-based module resolution produces pathological results for TypeScript projects. The parser creates Folder nodes for leaf directories but NOT for intermediate directories. A project like Hono has 88 leaf folders — many with 1-2 files — but no Folder nodes for `src/middleware`, `src/adapter`, etc. Using every leaf folder as a module boundary produces: avg coupling >0.56 (structurally inevitable), 174 hub findings (noise), and coupling scores that measure "is a file" not "has a boundary problem." The fix is NOT to soften the coupling formula — it's to define modules at the right granularity.

**Analysis modules** are defined by **path-prefix grouping at a configurable depth**, not by containment tree walking. This works regardless of whether intermediate directories exist as Folder nodes in the graph.

**Algorithm:**
1. For each node, resolve its `path_repo_rel` (from itself, its parent File node, or containment tree walk)
2. Strip the filename (last segment containing `.`) to get the directory path
3. Take the first `N` path segments as the **module key** (default N=2)
4. Group all nodes by their module key

**Examples at depth=2:**
| Path | Module Key |
|------|-----------|
| `src/middleware/cors/index.ts` | `src/middleware` |
| `src/hono.ts` | `src` |
| `src/router/linear-router/router.ts` | `src/router` |
| `runtime-tests/deno/hono.test.ts` | `runtime-tests/deno` |
| `benchmarks/routers/src/bench.ts` | `benchmarks/routers` |

**Why depth=2:** For TypeScript projects, this produces modules that match architectural intent: `src/middleware` (all middleware), `src/adapter` (all adapters), `src/jsx` (all JSX). Depth=1 is too coarse (everything under `src/` collapses). Depth=3 returns to the leaf-folder problem. For Rust/Go projects, depth=2 similarly maps to `crate/module` or `pkg/service`. This default is configurable via the `--config` TOML.

**Validated result:** Hono: 88 folders → 29 analysis modules. `src/middleware` (1,938 nodes, coupling 0.61), `src/jsx` (2,187 nodes, coupling 0.29). These numbers match architectural intuition.

#### 3.3.2 Internal/External Edge Classification

For each non-containment edge (A → B), determine:
- Module of A: the module key resolved for A via path-prefix grouping
- Module of B: same for B
- If module(A) == module(B): this is an **internal edge**
- If module(A) != module(B): this is an **external edge** (crosses module boundary)

#### 3.3.3 Legacy: Containment-Tree Module Resolution

The original containment-tree walk (nearest Folder/File/Module ancestor) is retained for module membership used in cycle descriptions and node display names. It is NOT used for coupling, hub score, instability, or API surface analysis — those use path-prefix analysis modules exclusively.

---

## 4. Analysis Algorithms

### 4.1 Cycle Detection

**Algorithm:** Tarjan's Strongly Connected Components (SCC)

**Input:** Directed graph of all non-containment edges (Call, Import, Uses, Type)

**Output:** List of strongly connected components with size > 1. Each SCC represents a cycle.

**Implementation requirements:**
- Use Tarjan's SCC algorithm (not DFS cycle detection) because it finds ALL cycles in a single O(V + E) pass
- For each SCC of size > 1, also extract the specific edges that form the cycle
- Sort SCCs by size (largest first) — larger cycles are more severe
- Compute cycle severity: `severity = sum(nodes_in_cycle * edges_in_cycle)` normalized to 0-100

**Cycle output per SCC:**

```json
{
  "id": "cycle-1",
  "type": "circular_dependency",
  "severity": "high",
  "description": "auth → database → config → auth",
  "node_ids": ["node-12", "node-45", "node-78"],
  "edge_ids": ["edge-23", "edge-56", "edge-89"],
  "cycle_length": 3,
  "impact": "14 downstream callers affected",
  "blast_radius": 14
}
```

**Description generation:** Build the description from the FQN or name of each node in the cycle, joined by ` → `, with the first node repeated at the end to show the loop closure. Use the shortest unambiguous name (module name if all nodes are in different modules, otherwise function name).

**Severity classification:**

| Condition | Severity |
|-----------|----------|
| Cycle length >= 4 OR cycle involves nodes with blast_radius > median | `critical` |
| Cycle length 3 OR any node in cycle has fan_in > 5 | `high` |
| Cycle length 2 | `warning` |

### 4.2 Fan-In / Fan-Out

**Algorithm:** Count incoming and outgoing non-containment edges per node.

**Input:** All non-containment edges.

**Output per node:**

| Field | Definition |
|-------|------------|
| `fan_in` | Count of unique non-containment edges where this node is the target |
| `fan_out` | Count of unique non-containment edges where this node is the source |
| `is_hotspot` | `true` if this node's `fan_in` is in the top 5% of all function-type nodes |

**Hotspot threshold:** Compute the 95th percentile of fan_in across all nodes of type `Function`. Any function node with fan_in >= this value is flagged as a hotspot. If the graph has fewer than 20 function nodes, use a fixed threshold of fan_in >= 8.

**Hotspot finding output:**

```json
{
  "id": "hotspot-1",
  "type": "blast_radius_hotspot",
  "severity": "high",
  "description": "connectPool: called by 14 paths, affects 27 nodes",
  "node_ids": ["node-45"],
  "fan_in": 14,
  "blast_radius": 27
}
```

### 4.3 Module Coupling

**Algorithm:** For each module, compute the ratio of external edges to total edges touching module members.

**Formula:**
```
coupling_score = external_edges / (internal_edges + external_edges)
```

Where:
- `external_edges` = count of non-containment edges where exactly one endpoint is inside the module
- `internal_edges` = count of non-containment edges where both endpoints are inside the module

**Scale:**
- 0.0 = perfectly isolated (all edges are internal)
- 1.0 = fully coupled (all edges cross module boundary)

**Output per module:**

```json
{
  "node_id": "node-5",
  "coupling_score": 0.73,
  "internal_edges": 12,
  "external_edges": 34,
  "risk_level": "high"
}
```

**Coupling finding output:**

```json
{
  "id": "coupling-1",
  "type": "high_coupling",
  "severity": "warning",
  "description": "auth/ module: coupling 0.73 (34 external vs 12 internal edges)",
  "node_ids": ["node-5"],
  "coupling_score": 0.73,
  "internal_edges": 12,
  "external_edges": 34
}
```

**Severity classification:**

| Coupling Score | Severity |
|----------------|----------|
| > 0.7 | `high` |
| 0.5 - 0.7 | `warning` |
| 0.3 - 0.5 | `info` |
| < 0.3 | Not reported as finding |

**Note on fixed thresholds:** Unlike IFC and hub (which use percentile-based thresholds), coupling uses fixed thresholds because the ratio is already normalized to [0, 1]. A coupling of 0.7 has the same structural meaning regardless of project size — 70% of a module's edges cross its boundary. Percentile-based thresholds would hide coupling problems in projects where every module is poorly encapsulated.

**Module eligibility:** Only compute coupling for modules with >= 3 internal nodes. Modules with 1-2 nodes produce noisy coupling scores.

### 4.4 Dead Code Detection

**Algorithm:** Identify function nodes with zero incoming non-containment edges, excluding entry points.

**Steps:**
1. Collect all nodes of type `Function` (or callable types: `Function`, `Method`)
2. For each, count incoming non-containment edges (fan_in)
3. If fan_in == 0, check entry point heuristics (see below)
4. If not an entry point, flag as potentially dead

**Entry point heuristics (exemptions):**

> **Empirical validation (2026-02-24):** Running against Hono (1,226 functions), the original heuristic set flagged 59 dead functions at a 76% false positive rate. After adding the heuristics below, dead code dropped to 12 functions at ~50% FP (remaining FPs are parser limitations: callback references, `this`-method calls, property access patterns that don't generate Call edges). The expanded heuristic set is documented below.

**Layer 1: Universal heuristics (apply to any language/graph):**

| Heuristic | Exemption Rule | Implementation |
|-----------|---------------|----------------|
| Root-level modules | Nodes of type `Folder`/`Module` at containment depth 0-1 | Walk containment tree, check depth |
| Exported from index/barrel files | Functions contained in files named `index.ts`, `index.js`, `mod.rs`, `lib.rs`, `__init__.py` | Check `path_repo_rel` of parent file |
| Test files | Any node where `is_test == true` | Walk to parent File, check `is_test` |
| Vendor/build/generated files | Any node where parent file has `is_vendor`, `is_build_output`, or `is_generated` | Walk to parent File, check properties |
| Main/entrypoint files | Functions in files named `main.ts`, `main.rs`, `app.ts`, `server.ts` | Check `path_repo_rel` and `name` |
| Constructor/lifecycle | Functions named `constructor`, `init`, `setup`, `teardown`, `mount`, `unmount`, `drop`, `new`, `default`, `from`, `__init__`, `__del__`, etc. | Name match against known lifecycle patterns |
| Exported functions | Functions with `is_exported == true` (if this field exists in schema) | Direct column check |

**Layer 2: Ecosystem-aware heuristics (configurable per language — see Section 5.0):**

| Heuristic | Exemption Rule | Ecosystems |
|-----------|---------------|------------|
| Framework handlers | Name/FQN ends with: `Handler`, `Controller`, `Middleware`, `Route`, `Resolver`, `Loader`, `Action`, `Endpoint`, `Servlet`, `Listener`, `Subscriber`, `Consumer`, `Interceptor`, `Guard`, `Filter` (case-insensitive) | All web frameworks |
| Framework handler prefixes | Name starts with: `handle`, `on_`, `do_`, `process_` | All event-driven systems |
| JSX/React components | PascalCase function names (uppercase first letter, 2+ chars). JSX compiler requires component names start with uppercase. | TypeScript, JavaScript |
| JSX intrinsic elements | Functions under `intrinsic_element::components::*` or `jsx::dom::*::components::*` paths. HTML element handlers invoked via `<element />` syntax. | TypeScript, JavaScript |
| JSX runtime functions | Names: `jsx`, `jsxs`, `jsxDEV`, `jsxAttr`, `jsxEscape`, `Fragment`, `createElement`, `createRef`, `h` | TypeScript, JavaScript |
| Class property accessors | Common getter names (`url`, `method`, `protocol`, `readyState`, `hostname`, `port`, `pathname`, `headers`, `status`, `body`, `length`, `size`, `name`, `value`) when FQN has 2+ segments | TypeScript, JavaScript, Python |
| Mock/stub functions | Names starting with `mock`, `stub`, `fake` | All (test infrastructure) |
| React/Angular/Vue lifecycle | `componentDidMount`, `ngOnInit`, `ngOnDestroy`, `mounted`, `created`, etc. | TypeScript, JavaScript |
| Spring Boot annotations | `@RequestMapping`, `@GetMapping`, etc. (requires annotation data in graph) | Java, Kotlin |
| Unity lifecycle | `Awake`, `Start`, `Update`, `FixedUpdate`, `OnEnable`, `OnDisable` | C# |

**These heuristics are intentionally conservative.** It is better to miss some dead code than to flag live entry points. False positives in dead code detection destroy user trust faster than any other metric.

**Irreducible parser limitations:** Some functions will always appear dead in static graph analysis regardless of heuristics:
- Functions passed as callbacks (`array.sort(compareKey)`) — no Call edge generated
- Methods called via `this` on dynamically-typed objects — no Call edge generated
- Functions invoked via reflection, dependency injection, or runtime dispatch
- Property accessors called via `obj.property` syntax — no Call edge generated

These are not heuristic failures; they are fundamental limits of static structural analysis. The dead code finding severity (`info`) and description (`potentially unreachable`) reflect this uncertainty.

**Dead code finding output:**

```json
{
  "id": "dead-1",
  "type": "potentially_unreachable",
  "severity": "info",
  "description": "3 potentially unreachable functions",
  "node_ids": ["node-101", "node-102", "node-103"],
  "count": 3
}
```

**Note:** Finding type is `potentially_unreachable`, not `dead_code`. Severity is `info`, not `warning`. The description uses "potentially" because static analysis cannot prove code is dead — only that it has no in-graph callers.

### 4.5 Blast Radius

**Algorithm:** Reverse BFS (breadth-first search) from each node, counting transitive dependents.

**Definition:** The blast radius of node N is the number of unique nodes that transitively depend on N. "Depend on" means: there exists a path of non-containment edges from the dependent node to N (following edges in reverse).

**Steps:**
1. Build the reverse graph (flip all non-containment edge directions)
2. For each node N: BFS from N in the reverse graph, counting all reachable nodes
3. The count (excluding N itself) is the blast radius

**Optimization:** This is the most expensive computation (O(V * (V + E)) worst case). Optimize with:
- Only compute blast radius for function-type nodes (skip folders/files — they inherit from their children)
- Cache BFS results — if during BFS from A you visit B, then blast_radius(B) includes everything reachable from B (which you've already explored)
- Consider computing only for nodes above a fan_in threshold (e.g., fan_in >= 2) and defaulting others to 0

**Output per node:**

```json
{
  "blast_radius": 27
}
```

### 4.6 Depth Complexity

**Algorithm:** Longest call chain via DFS from each root node.

**Definition:** The maximum number of non-containment edges in the longest directed path through the call graph. Measures how deep function call chains go.

**Steps:**
1. Identify root nodes: nodes with fan_in == 0 for Call edges specifically (entry points)
2. DFS from each root, tracking path length
3. Record `max_call_depth` (global) and `depth_from_root` per node (the longest path from any root to this node)
4. Handle cycles: if DFS encounters a cycle, do not recurse into it (use the cycle detection results to break cycles before depth computation, or track visited nodes)

**Output:**

```json
{
  "max_call_depth": 14
}
```

Per-node: `depth_from_root` stored in node annotations.

### 4.7 Additional Deterministic Metrics (Recommended)

The following metrics are deterministically computable from the existing graph data and provide additional architectural health dimensions. They are well-established in software engineering literature and add significant value to the health report.

#### 4.7.1 Instability (Robert C. Martin)

**Formula:**
```
I = Ce / (Ca + Ce)
```
Where:
- `Ca` (afferent coupling) = number of modules that depend on this module (fan-in at module level)
- `Ce` (efferent coupling) = number of modules this module depends on (fan-out at module level)

**Scale:** 0.0 = maximally stable (everyone depends on it, it depends on nothing). 1.0 = maximally unstable (depends on everything, nothing depends on it).

**Why it matters:** Instability combined with the existing coupling score gives a complete picture of module dependency health. A module that is both highly coupled AND highly stable is a dangerous bottleneck — changing it is expensive AND risky. A module that is highly unstable AND highly coupled has too many reasons to change from too many directions.

**Output per module:**
```json
{
  "instability": 0.65,
  "afferent_coupling": 7,
  "efferent_coupling": 13
}
```

#### 4.7.2 Package Tangle Index

**Formula:**
```
tangle_index = edges_in_cycles / total_non_containment_edges
```

**Scale:** 0.0 = no cycles. 1.0 = every edge participates in a cycle.

**Why it matters:** While cycle count tells you how many cycles exist, tangle index tells you what fraction of the total dependency graph is cyclical. A project with 2 cycles out of 2,000 edges is structurally sound. A project with 2 cycles out of 20 edges is severely tangled.

**Output:**
```json
{
  "tangle_index": 0.12
}
```

#### 4.7.3 Information Flow Complexity (Henry & Kafura)

**Formula per function:**
```
ifc = (fan_in × fan_out)²
```

**Why it matters:** A function with high fan-in AND high fan-out is an information flow bottleneck — it receives data from many sources and distributes it to many consumers. These are the hardest functions to refactor safely. Fan-in alone catches hotspots; fan-out alone catches orchestrators; the product catches the dangerous intersection.

**Finding threshold:** ~~Report as finding when `ifc > 100` (fan_in * fan_out > 10).~~ **REVISED (2026-02-24):** Use **project-relative threshold: 95th percentile** of all function IFC scores, with an absolute floor of 100. This makes the system self-calibrating: in a project with uniformly low IFC, 95th percentile catches the real outliers. In a project with many high-IFC functions, the threshold rises naturally.

> **Empirical finding:** Fixed threshold of 100 flagged 302/1,226 functions in Hono (24.6%). When >24% of all functions trigger a finding, the finding type measures "is a function" not "is a problem." At 95th percentile, ~61 functions are flagged — the actual statistical outliers.

**Severity thresholds (also project-relative):**
| Condition | Severity |
|-----------|----------|
| IFC > 99th percentile (floor: 400) | `critical` |
| IFC > 97th percentile (floor: 200) | `high` |
| IFC > 95th percentile (floor: 100) | `warning` |

**Output per node:**
```json
{
  "information_flow_complexity": 196
}
```

#### 4.7.4 API Surface Ratio

**Formula per module:**
```
api_surface = exported_functions / total_functions
```

Where `exported_functions` = functions with incoming edges from outside the module.

**Scale:** 0.0 = fully encapsulated (nothing is called from outside). 1.0 = everything is public (no encapsulation).

**Why it matters:** Modules with high API surface ratios are exposing too much internal implementation. This makes them hard to refactor because external consumers depend on internal details. This is a direct measure of encapsulation quality.

**Output per module:**
```json
{
  "api_surface_ratio": 0.45,
  "exported_functions": 9,
  "total_functions": 20
}
```

#### 4.7.5 Hub Score (Dependency Centrality)

**Algorithm:** Identify nodes that serve as bridges between otherwise disconnected graph regions.

**Simplified approach:** For each function node, compute:
```
hub_score = (distinct_source_modules × distinct_target_modules) / total_modules²
```

Where `distinct_source_modules` = number of unique modules that call this function, and `distinct_target_modules` = number of unique modules this function calls.

**Why it matters:** Hub nodes connect disparate parts of the system. They are the hardest to remove, the most dangerous to break, and the most important to understand. This is distinct from blast radius (which measures downstream impact) — hub score measures *cross-cutting connectivity*.

**Finding threshold:** ~~Flag as finding when a node bridges 4+ distinct modules.~~ **REVISED (2026-02-24):** Use **project-relative threshold: 95th percentile** of all function bridge counts, with an absolute floor of 4. In a project with 29 modules, a function bridging 4 is unremarkable. In a project with 8 modules, bridging 4 is significant.

> **Empirical finding:** Fixed threshold of 4 flagged 174/1,226 functions in Hono with 88 leaf folders. After module granularity fix (29 analysis modules) + percentile threshold, ~61 functions flagged — the true architectural bridges.

**Severity thresholds (project-relative):**
| Condition | Severity |
|-----------|----------|
| Bridges > 99th percentile (floor: 6) | `high` |
| Bridges > 97th percentile (floor: 5) | `warning` |
| Bridges > 95th percentile (floor: 4) | `warning` |

---

## 5. Health Score Composition

### 5.0 Three-Layer Framework: Universal Metrics → Ecosystem Thresholds → Scoring

> **Architectural principle (established 2026-02-24 from empirical calibration).** Every measurement in `ge-analyze` falls into one of three layers. Understanding which layer a measurement lives in determines whether it's universal or needs calibration.

**Layer 1 — Universal Graph Metrics (language-agnostic, always correct).**
These are mathematical properties of directed graphs. They apply identically to TypeScript, Rust, Java, Go, Python, C#, or any system represented as nodes and edges. They never need ecosystem configuration.

| Metric | Why Universal |
|--------|--------------|
| Cycle count and membership | A circular dependency is structurally identical in every language. A→B→A prevents independent reasoning/testing/deployment regardless of syntax. |
| Fan-in / Fan-out per node | Counting incoming and outgoing edges is a graph operation. No interpretation needed. |
| Blast radius per node | Transitive dependent count via reverse BFS. Pure graph reachability. |
| Call depth per node | Longest directed path length. Pure graph property. |
| Coupling ratio per module | `external_edges / (internal + external)`. Once module boundaries are defined, this is arithmetic. |
| IFC per node | `(fan_in × fan_out)²`. Pure arithmetic on graph properties. |
| Tangle index | `edges_in_cycles / total_edges`. Pure ratio. |
| Instability | `Ce / (Ca + Ce)`. Pure ratio on module-level fan-in/out. |

**These are always computed, always reported as raw numbers, and never adjusted.** They are the ground truth.

**Layer 2 — Ecosystem-Calibrated Thresholds (configurable, stable per ecosystem).**
"When does a metric value become a *finding*?" This is where ecosystem-specific knowledge enters. A coupling ratio of 0.7 means something different for a TypeScript file (which imports freely) vs a Rust crate (which has explicit `pub` boundaries). These thresholds are:
- Configured per ecosystem (TypeScript, Rust, Java, Go, Python, C#)
- Stable within an ecosystem (TypeScript's module conventions don't change quarterly)
- Project-relative where possible (95th percentile for IFC/hub catches outliers in ANY project)

| Threshold | What Varies | Why |
|-----------|------------|-----|
| Module boundary depth | TS: depth 2 (`src/middleware`). Rust: mod tree. Java: package. Go: package. | Languages have different natural encapsulation units |
| Hub finding threshold | Percentile-based, adapts to project's module count | 29-module project vs 8-module project have different baselines |
| IFC finding threshold | Percentile-based, adapts to project's function density | Dense library APIs vs sparse service layers have different norms |
| Dead code entry point heuristics | JSX components (TS only), Spring annotations (Java only), Unity lifecycle (C# only) | Frameworks define invocation patterns the parser can't resolve as Call edges |
| Depth expectations | Middleware chains (TS/Go) inherently deeper than service layers | Some architectures are deep by design |

**Layer 3 — Comparative Percentile Rank (population-based).**
Where does this project sit relative to the population of all analyzed projects? Computed per-metric and as a weighted composite. Percentile means "better than X% of projects in the population" — not an opinionated formula, but an actual rank against real data.

When a population (norms) database is available:
- Each metric value is ranked against the population using midpoint tie-handling
- Per-metric percentiles are reported alongside absolute values
- The composite health score equals the weighted-average percentile
- The weights (25% cycles, 25% coupling, 20% hotspots, 15% dead code, 15% depth) determine relative importance

When no population data is available:
- The `percentiles` block is omitted entirely
- A fallback formula-based score is provided for backward compatibility
- The system is honest — it doesn't fabricate a comparative ranking without data

**Design implication:** The system ALWAYS reports Layer 1 raw metrics (objective, unquestionable) in the `metrics` block. Layer 2 thresholds determine which findings appear (configurable, per-ecosystem). Layer 3 provides comparative context via the `percentiles` block when population data exists. A consumer who only wants facts uses `metrics`. A consumer who wants comparison uses `percentiles`. The raw metrics are never adjustable — they're facts about the graph.

### 5.0.1 Ecosystem Configuration Architecture

Ecosystem-dependent behavior (Layer 2) is encoded in **ecosystem profiles**. Each profile is a small configuration bundle:

```toml
[profile.typescript]
analysis_module_depth = 2
dead_code_heuristics = ["jsx_components", "jsx_runtime", "jsx_intrinsic", "property_accessors", "mock_functions", "framework_handlers", "lifecycle_methods"]
ifc_finding_percentile = 95
hub_finding_percentile = 95
exclude_test_modules_from_coupling_avg = true

[profile.rust]
analysis_module_depth = 2
dead_code_heuristics = ["lifecycle_methods", "trait_impls", "framework_handlers"]
ifc_finding_percentile = 95
hub_finding_percentile = 95
exclude_test_modules_from_coupling_avg = true

[profile.java]
analysis_module_depth = 2
dead_code_heuristics = ["framework_handlers", "lifecycle_methods", "spring_annotations"]
ifc_finding_percentile = 95
hub_finding_percentile = 95
exclude_test_modules_from_coupling_avg = true

[profile.go]
analysis_module_depth = 2    # maps to pkg/service, cmd/server, internal/auth
dead_code_heuristics = ["framework_handlers", "lifecycle_methods"]
ifc_finding_percentile = 95
hub_finding_percentile = 95
exclude_test_modules_from_coupling_avg = true
# Notes: Go's package system enforces boundaries via directories. test files
# (*_test.go) live alongside source files, so test-module detection must rely
# on is_test classification, not directory patterns. init() functions are
# covered by lifecycle_methods. Handler funcs (http.HandlerFunc signature) are
# covered by framework_handlers suffix/prefix matching.

[profile.python]
analysis_module_depth = 2    # maps to src/auth, app/models, tests/unit
dead_code_heuristics = ["framework_handlers", "lifecycle_methods", "mock_functions", "property_accessors"]
ifc_finding_percentile = 95
hub_finding_percentile = 95
exclude_test_modules_from_coupling_avg = true
# Notes: Python's __init__.py barrel files are already covered in barrel
# detection. Django views/urls, FastAPI route decorators, and Flask endpoints
# are invoked by the framework — covered by framework_handlers. __dunder__
# methods covered by lifecycle_methods. @property accessors covered by
# property_accessors. conftest.py fixtures appear dead (no Call edges) —
# mock_functions catches mock/stub/fake prefixes; pytest fixtures may need
# a future heuristic if FP rate is high.

[profile.csharp]
analysis_module_depth = 2    # maps to src/Services, src/Controllers
dead_code_heuristics = ["framework_handlers", "lifecycle_methods", "unity_lifecycle"]
ifc_finding_percentile = 95
hub_finding_percentile = 95
exclude_test_modules_from_coupling_avg = true
# Notes: C# namespaces map to directories by convention. Unity lifecycle
# (Awake, Start, Update, FixedUpdate, OnEnable, OnDisable, OnCollisionEnter,
# etc.) invoked by engine, not user code. ASP.NET controller actions invoked
# by routing — covered by framework_handlers suffix matching (Controller,
# Middleware, Filter). [Fact]/[Test] attributes identify test methods
# (requires attribute data in graph; fallback: is_test classification on file).
```

**How many ecosystem profiles are needed?** In practice, ~6-8 cover >95% of professional software:
1. **TypeScript/JavaScript** (React, Node, Deno, Bun)
2. **Rust**
3. **Java/Kotlin** (Spring, Android)
4. **Go**
5. **Python** (Django, FastAPI, Flask)
6. **C#** (.NET, Unity)
7. **C/C++** (future)
8. **Swift** (future)

**Do ecosystems change frequently?** No. TypeScript's module system, JSX component conventions, and import patterns have been stable since ~2018. Java's package conventions haven't changed since 1996. Rust's mod tree since 1.0. The framework-specific heuristics (Spring Boot, Unity) change slowly — maybe 1-2 new annotation patterns per major version, per year. This is a "build once, maintain annually" cost, not a "constantly update" cost.

**What about project-specific ecosystems?** The `--config` TOML allows per-project overrides. If a project uses an unusual framework that has unique entry point patterns, those can be added to the config without changing the engine. The percentile-based thresholds (IFC, hub) are already self-calibrating and don't need per-project tuning.

### 5.0.2 Finding Presentation: Caps and Tiering

> **Empirical finding (2026-02-24):** Hono produced 572 findings. 302 were IFC, 174 were hub nodes. The spec (Section 10.3) says findings should make a senior engineer say "I didn't know that." 572 findings is a data dump, not an analysis.

**Rule: Top N findings per type, overflow summarized.**

For each finding type, keep the top 10 findings sorted by severity (desc) then metric value (desc). Excess findings are summarized as a single aggregate finding:

```json
{
  "id": "informationflowbottleneck-overflow",
  "type": "information_flow_bottleneck",
  "severity": "info",
  "description": "51 additional InformationFlowBottleneck findings not shown (top 10 displayed above)",
  "count": 51
}
```

**Risk level computation uses ALL findings (before capping).** The cap affects presentation only. Node annotations, risk levels, and module annotations are computed from the full finding set to ensure correctness.

**Why 10?** Audits present 5-15 high-signal findings. Content marketing needs 3-5 dramatic ones. Self-serve product shows a findings panel. 10 per type gives enough depth without noise. The overflow summary communicates "we found more, but these are the most important."

### 5.0.3 Configuration TOML Schema

The `--config` TOML file allows per-project overrides of ecosystem defaults. The schema below defines every valid key. Any key not provided falls back to the ecosystem profile default, which falls back to the hardcoded engine default.

**Resolution order:** CLI `--config` TOML → ecosystem profile → engine defaults.

```toml
# --- Ecosystem selection ---
# If omitted, ge-analyze auto-detects from the graph's language property
# on File nodes (majority language wins).
ecosystem = "typescript"   # One of: typescript, rust, java, go, python, csharp

# --- Module boundary configuration (Layer 2) ---
[modules]
analysis_depth = 2                    # Path prefix depth for analysis modules
min_module_size = 3                   # Minimum nodes for coupling eligibility
exclude_test_modules_from_coupling = true  # Exclude test-only modules from avg coupling in health score

# --- Finding thresholds (Layer 2) ---
[thresholds]
coupling_high = 0.7                   # Coupling score above which to flag as finding
coupling_warning = 0.5
coupling_info = 0.3

ifc_percentile = 95                   # Percentile for IFC finding threshold
ifc_severity_critical_percentile = 99
ifc_severity_high_percentile = 97
ifc_floor = 100                       # Absolute minimum IFC to flag (regardless of percentile)

hub_percentile = 95                   # Percentile for hub finding threshold
hub_severity_high_percentile = 99
hub_severity_warning_percentile = 97
hub_floor = 4                         # Absolute minimum bridges to flag

hotspot_percentile = 95               # Fan-in percentile for hotspot detection
hotspot_small_graph_threshold = 8     # Fixed threshold when < 20 function nodes

depth_warning = 10
depth_high = 15
depth_critical = 20

api_surface_warning = 0.6
api_surface_high = 0.75
api_surface_critical = 0.9

max_findings_per_type = 10            # Cap findings per type (overflow summarized)

# --- Dead code heuristics (Layer 2) ---
[dead_code]
# Each key enables/disables a specific heuristic. All default to true for
# the active ecosystem profile; set to false to disable.
barrel_files = true
entrypoint_files = true
framework_handlers = true
lifecycle_methods = true
jsx_components = true          # TypeScript/JavaScript only
jsx_runtime = true             # TypeScript/JavaScript only
jsx_intrinsic = true           # TypeScript/JavaScript only
property_accessors = true      # TypeScript/JavaScript/Python
mock_functions = true
unity_lifecycle = true         # C# only
spring_annotations = true      # Java/Kotlin only
trait_impls = true             # Rust only

# Additional name patterns to exempt from dead code (regex, case-insensitive)
extra_entry_point_patterns = [
    "^handle",                 # Already built-in, shown as example
    "^on[A-Z]",               # Event handlers
]

# --- Health score weights (Layer 3) ---
[score_weights]
cycle_severity = 0.25
coupling_health = 0.25
hotspot_concentration = 0.20
dead_code_ratio = 0.15
depth_complexity = 0.15
# Must sum to 1.0. ge-analyze validates this at startup.
```

**Validation rules:** ge-analyze validates the config at startup before running analysis:
- `score_weights` values must sum to 1.0 (within floating point tolerance)
- Percentile values must be in [1, 99]
- Floor values must be non-negative
- `ecosystem` must be a recognized profile name
- Unknown keys produce a warning on stderr (forward compatibility)

**Phase 4-1 scope:** The config struct is defined and parsed, but `--config` itself is a future CLI arg. All values use hardcoded defaults matching the TypeScript profile. The struct exists so that when `--config` is wired in, the change is purely plumbing — no algorithm changes needed.

### 5.1 Two-Layer Output: Absolute Metrics + Percentile Rank

> **Updated 2026-02-25:** Replaces the previous quadratic-curve formula tables. The composite score is now derived from population percentiles rather than hand-tuned formulas.

**Layer A — Absolute Metrics (always present, `metrics` block in JSON).** Direct, transparent, no interpretation. Each metric reports the raw count, denominator, and ratio with a human-readable description. See Section 6.1 for the full JSON schema.

| Metric | Formula | What It Measures |
|--------|---------|-----------------|
| `cycle_ratio` | `nodes_in_cycles / total_nodes` | Fraction of graph participating in circular dependencies |
| `avg_coupling` | `avg(external / (internal + external))` across modules | Average module boundary leakage |
| `hotspot_concentration` | `sum_hotspot_fan_in / total_fan_in` | How much dependency traffic flows through the top 5% of functions |
| `dead_ratio` | `dead_functions / total_functions` | Fraction of functions with zero incoming non-containment edges |
| `max_call_depth` | Longest directed path in call graph | Depth of deepest call chain |
| `tangle_index` | `edges_in_cycles / total_structural_edges` | Fraction of structural edges participating in cycles |

**Layer B — Percentile Rank (optional, `percentiles` block in JSON).** Present only when `--norms <population.sqlite>` is provided. For each metric, reports the project's percentile rank against the population.

| Field | Meaning |
|-------|---------|
| `population_size` | Number of projects in the population database |
| `population_version` | Version/date string for the population snapshot |
| `composite_percentile` | Weighted average of per-metric percentiles (same weights as `score_weights`) |
| `per_metric.<name>.value` | The project's raw metric value |
| `per_metric.<name>.percentile` | "Better than X% of projects" |
| `per_metric.<name>.description` | Human-readable sentence |

**Percentile computation:** For metric value `v` against population: `percentile = (count where population_value >= v) / total * 100`. Ties handled by midpoint. No z-scores, no normal distribution assumption — just rank.

### 5.2 Composite Health Score Derivation

The `health_score` field (0-100) is derived as follows:

| Condition | Derivation |
|-----------|-----------|
| Norms DB provided | `health_score = composite_percentile` (weighted average of per-metric percentiles) |
| No norms DB | `health_score` = fallback formula-based score (simple linear penalties, backward compat) |

**Component weights** (configurable via `score_weights` in TOML):

| Component | Weight | Metric Used |
|-----------|--------|------------|
| Cycle severity | 25% | `cycle_ratio` percentile (or fallback formula) |
| Coupling health | 25% | `avg_coupling` percentile (or fallback formula) |
| Hotspot concentration | 20% | `hotspot_concentration` percentile (or fallback formula) |
| Dead code ratio | 15% | `dead_ratio` percentile (or fallback formula) |
| Depth complexity | 15% | `max_depth` percentile (or fallback formula) |

**Backward compatibility:** The `health_score_components` block is populated with per-metric percentiles (when norms available) or formula-based sub-scores (when not). The `score` field in each component is 0-100 either way. Consumers that read `health_score_components` do not need code changes.

### 5.3 Future Score Extension (Design for This, Do Not Implement Yet)

Phase 6 adds dimensions. The scoring system must be designed so new components can be added with weight redistribution. Recommended approach: store component weights in a config struct, not hardcoded. When Phase 6 dimensions arrive, weights redistribute proportionally (see `PHASE_6_DEEP_HEALTH_INTELLIGENCE.md` for the extended weight table).

### 5.4 Score Display

> **Updated 2026-02-25:** The percentile is shown directly. No more "76-100 = Healthy" color mapping on an arbitrary formula.

**When percentiles are available:**
- Display the composite percentile directly: "73rd percentile" or "Better than 73% of analyzed projects"
- Per-metric percentiles can be shown as coverage-style bars
- The `population_size` should be visible so users understand the sample basis
- Small populations (N < 30) should include a confidence caveat

**When percentiles are not available (no norms DB):**
- Display the Layer A absolute metrics as coverage-style bars (e.g., "Cycle ratio: 0.2%", "Dead code: 10.9%")
- The fallback `health_score` may be shown but should be labeled as "estimated" or "preliminary"
- The `percentiles` block is absent from the JSON — UI should detect this and adjust display

**Rounding:** All scores and percentiles are integers 0-100.

---

## 6. Output Format (JSON)

### 6.1 Top-Level Structure

> **Updated 2026-02-25:** Added `metrics` (always present) and `percentiles` (optional) blocks. `health_score` is now `Optional<u32>` (null when no data supports it).

```json
{
  "version": "1.0.0",
  "generated_at": "2026-02-25T14:30:00Z",
  "analysis_duration_ms": 1200,
  "db_path": "/path/to/graph.sqlite",

  "health_score": 73,
  "health_score_components": {
    "cycle_severity": { "score": 68, "weight": 0.25 },
    "coupling_health": { "score": 55, "weight": 0.25 },
    "hotspot_concentration": { "score": 41, "weight": 0.20 },
    "dead_code_ratio": { "score": 72, "weight": 0.15 },
    "depth_complexity": { "score": 81, "weight": 0.15 }
  },

  "metrics": {
    "cycles": {
      "count": 23,
      "total": 10331,
      "ratio": 0.0022,
      "description": "23 of 10,331 nodes participate in 5 dependency cycles"
    },
    "coupling": {
      "modules_measured": 8,
      "modules_above_070": 3,
      "modules_above_050": 5,
      "avg_coupling": 0.670,
      "description": "3 of 8 modules exceed 0.70 coupling (avg 0.670)"
    },
    "hotspot_concentration": {
      "count": 65,
      "total": 1314,
      "ratio": 0.465,
      "description": "65 hotspot functions (top 5%) absorb 46.5% of all incoming dependencies"
    },
    "dead_code": {
      "count": 143,
      "total": 1314,
      "ratio": 0.109,
      "description": "143 of 1,314 functions have zero incoming non-containment edges (10.9%)"
    },
    "depth": {
      "max_call_depth": 10,
      "description": "Longest call chain is 10 calls deep"
    },
    "tangle_index": {
      "count": 12,
      "total": 31109,
      "ratio": 0.000386,
      "description": "0.04% of structural edges participate in cycles"
    }
  },

  "percentiles": {
    "population_size": 500,
    "population_version": "2026-02-25",
    "composite_percentile": 73,
    "per_metric": {
      "cycle_ratio": { "value": 0.0022, "percentile": 68, "description": "Lower cycle ratio than 68% of 500 analyzed projects" },
      "avg_coupling": { "value": 0.670, "percentile": 55, "description": "Lower coupling than 55% of 500 analyzed projects" },
      "dead_ratio": { "value": 0.109, "percentile": 72, "description": "Lower dead code ratio than 72% of 500 analyzed projects" },
      "hotspot_concentration": { "value": 0.465, "percentile": 41, "description": "Lower hotspot concentration than 41% of 500 analyzed projects" },
      "max_depth": { "value": 10, "percentile": 81, "description": "Shallower call depth than 81% of 500 analyzed projects" },
      "tangle_index": { "value": 0.000386, "percentile": 65, "description": "Lower tangle index than 65% of 500 analyzed projects" }
    }
  },

  "summary": {
    "total_nodes": 847,
    "total_edges": 2341,
    "total_functions": 312,
    "total_modules": 23,
    "cycles_found": 9,
    "cycle_total_nodes": 27,
    "hotspot_count": 4,
    "hotspot_threshold_fan_in": 12,
    "high_coupling_modules": 3,
    "dead_functions": 31,
    "max_call_depth": 14,
    "tangle_index": 0.12,
    "avg_module_coupling": 0.42,
    "avg_fan_in": 3.2,
    "avg_fan_out": 2.8
  },

  "findings": [ ... ],

  "node_annotations": { ... },

  "module_annotations": { ... },

  "boundary_violations": [ ... ]
}
```

**Key changes from v1.0:**
- `health_score` is now `Optional<u32>` — omitted (null) when no data supports a meaningful number
- `metrics` block is always present — Layer A absolute metrics
- `percentiles` block is present only when `--norms` is provided — Layer B comparative rank
- `health_score_components` sub-scores are percentiles (when norms available) or fallback formulas
- The `summary` block is unchanged — it remains the Layer 1 aggregate statistics
```

### 6.2 Finding Object Schema

Each finding in the `findings` array:

```json
{
  "id": "cycle-1",
  "type": "circular_dependency",
  "severity": "critical",
  "description": "auth → database → config → auth",
  "detail": "3-node circular dependency chain involving authentication, database, and configuration modules",
  "node_ids": ["node-12", "node-45", "node-78"],
  "edge_ids": ["edge-23", "edge-56", "edge-89"],
  "primary_node_id": "node-12",
  "metric_name": "cycle_length",
  "metric_value": 3,
  "impact": "14 downstream callers affected",
  "blast_radius": 14,
  "recommendation": "Break the cycle by extracting the shared dependency into its own module, or invert the config → auth dependency using dependency injection"
}
```

**Finding type enum:**

| Type | Description | Generated By |
|------|-------------|-------------|
| `circular_dependency` | Nodes in a strongly connected component | Cycle detection (4.1) |
| `blast_radius_hotspot` | Function with exceptionally high fan-in | Fan-in/out (4.2) |
| `high_coupling` | Module with coupling score above threshold | Module coupling (4.3) |
| `potentially_unreachable` | Functions with zero incoming non-containment edges | Dead code (4.4) |
| `deep_call_chain` | Call chain exceeding depth threshold | Depth analysis (4.6) |
| `information_flow_bottleneck` | Function with high IFC score | IFC (4.7.3) |
| `hub_node` | Function bridging many distinct modules | Hub score (4.7.5) |
| `low_encapsulation` | Module with high API surface ratio | API surface (4.7.4) |
| `boundary_violation` | Edge that violates a defined boundary rule | Boundary rules (future) |

**Severity enum:** `critical`, `high`, `warning`, `info`

**Severity assignment rules:**

| Finding Type | Critical | High | Warning | Info |
|-------------|----------|------|---------|------|
| `circular_dependency` | cycle_length >= 4 OR involves blast_radius > median | cycle_length == 3 OR fan_in > 5 | cycle_length == 2 | — |
| `blast_radius_hotspot` | blast_radius > 90th percentile AND fan_in > 90th percentile | blast_radius > 90th percentile | fan_in > 95th percentile | — |
| `high_coupling` | coupling > 0.85 | coupling > 0.7 | coupling > 0.5 | coupling > 0.3 |
| `potentially_unreachable` | — | — | — | Always `info` |
| `deep_call_chain` | depth > 20 | depth > 15 | depth > 10 | — |
| `information_flow_bottleneck` | ifc > 99th pctl (floor 400) | ifc > 97th pctl (floor 200) | ifc > 95th pctl (floor 100) | — |
| `hub_node` | — | bridges > 99th pctl (floor 6) | bridges > 95th pctl (floor 4) | — |
| `low_encapsulation` | api_surface > 0.9 | api_surface > 0.75 | api_surface > 0.6 | — |

### 6.3 Node Annotation Schema

The `node_annotations` object is keyed by node ID:

```json
{
  "node-12": {
    "fan_in": 14,
    "fan_out": 3,
    "blast_radius": 27,
    "depth_from_root": 4,
    "information_flow_complexity": 196,
    "is_hotspot": true,
    "is_dead": false,
    "cycle_member": true,
    "cycle_ids": ["cycle-1"],
    "hub_score": 0.15,
    "risk_level": "high"
  }
}
```

**Include annotations for ALL function-type nodes**, not just those with findings. UE needs annotations for every node to apply overlays correctly (e.g., a node with risk_level "low" still needs the annotation so UE knows to color it green, not leave it uncolored).

**`risk_level` computation per node:**

| Level | Condition |
|-------|-----------|
| `critical` | Node appears in 2+ finding types at severity `critical` or `high` |
| `high` | Node appears in 1+ finding at severity `critical`, or 2+ findings at severity `high` or `warning` |
| `warning` | Node appears in 1+ finding at severity `high` or `warning` |
| `info` | Node appears in findings only at severity `info` |
| `healthy` | Node appears in no findings |

### 6.4 Module Annotation Schema

The `module_annotations` object is keyed by node ID of the module:

```json
{
  "node-5": {
    "coupling_score": 0.73,
    "internal_edges": 12,
    "external_edges": 34,
    "instability": 0.65,
    "afferent_coupling": 7,
    "efferent_coupling": 13,
    "api_surface_ratio": 0.45,
    "exported_functions": 9,
    "total_functions": 20,
    "total_nodes": 24,
    "risk_level": "high"
  }
}
```

### 6.5 Boundary Violations (Stub for Phase 4-6 / Future)

The `boundary_violations` array starts empty in P4-1. The field must be present in the JSON contract. Future: P4-6 (UE) passes boundary rules to `ge-analyze` via config, and violations are returned here.

```json
{
  "boundary_violations": []
}
```

---

## 7. Architectural Constraints

### 7.1 Read-Only Consumer

`ge-analyze` MUST NOT modify the SQLite database. It is a read-only consumer. Multiple processes (ge-analyze, ge-template, UE) may read the database concurrently.

### 7.2 No UE Dependencies

`ge-analyze` has zero dependency on Unreal Engine code, headers, or runtime. It is a standalone Rust binary. UE is a consumer of its JSON output.

### 7.3 Determinism

Given the same SQLite database, `ge-analyze` MUST produce byte-identical output every time (excluding `generated_at` timestamp and `analysis_duration_ms`). All algorithms must be deterministic. When ordering results, use stable sorts with consistent tiebreakers (e.g., sort findings by severity desc, then by node_id asc).

### 7.4 Graceful Degradation

If any single analysis algorithm fails (e.g., cycle detection encounters an unexpected graph shape), the remaining algorithms MUST still run. The health report should include the results of all successful analyses and note which algorithms failed.

```json
{
  "analysis_errors": [
    {
      "algorithm": "blast_radius",
      "error": "Exceeded memory limit during BFS",
      "nodes_affected": 12
    }
  ]
}
```

### 7.5 Extensibility

The analysis pipeline must be modular. Each algorithm (cycle detection, fan-in/out, coupling, dead code, blast radius, depth, instability, tangle, IFC, hub, API surface) should be an independent module that:
- Takes the graph as input
- Returns its specific annotations/findings
- Can be enabled/disabled independently
- Knows nothing about other algorithms (no cross-dependencies between analysis modules)

The orchestrator calls each module, collects results, merges annotations, computes the composite health score, and writes the JSON output.

**Crate and module structure:**

`ge-analyze` lives in the existing `graphengine-analysis` crate as a new binary target and `health/` module tree. The existing postprocessing/resolver code remains untouched.

```
graphengine-analysis/
├── Cargo.toml                — Add [[bin]] for ge-analyze, add rusqlite + clap deps
├── src/
│   ├── lib.rs                — Existing lib (postprocessing, resolver, io)
│   ├── bin/
│   │   └── ge_analyze.rs     — CLI entry point (clap, exit codes, orchestrator call)
│   ├── health/
│   │   ├── mod.rs            — Module entry, orchestrator
│   │   ├── graph.rs          — In-memory graph loaded from SQLite (nodes, edges, containment)
│   │   ├── cycles.rs         — Tarjan's SCC
│   │   ├── fan_metrics.rs    — Fan-in, fan-out, hotspot detection
│   │   ├── coupling.rs       — Module coupling scores
│   │   ├── dead_code.rs      — Dead code detection with entry point heuristics
│   │   ├── blast_radius.rs   — Reverse BFS blast radius
│   │   ├── depth.rs          — Call chain depth analysis
│   │   ├── instability.rs    — Martin's instability metric
│   │   ├── tangle.rs         — Package tangle index
│   │   ├── information_flow.rs — Henry & Kafura IFC
│   │   ├── hub_score.rs      — Cross-module bridge detection
│   │   ├── api_surface.rs    — Module encapsulation metric
│   │   ├── health_score.rs   — Composite score from all metrics
│   │   ├── report.rs         — JSON output builder (serde structs for Section 6 contract)
│   │   └── entry_points.rs   — Entry point heuristic rules (shared by dead_code, depth)
│   ├── postprocessing/       — Existing (derived edges, ghosts, enhancer)
│   ├── resolver/             — Existing (resolver trait, LSP enrichment, edge upgrade)
│   └── io/                   — Existing (JSON export, CLI utilities)
```

---

## 8. Definition of Done

### 8.1 Functional Requirements

- [ ] `ge-analyze --db path --output path` reads SQLite and produces a valid JSON health report
- [ ] Cycle detection finds all non-trivial strongly connected components in the non-containment graph
- [ ] Fan-in/fan-out computed for every node; hotspots flagged at 95th percentile
- [ ] Module coupling scores computed for every module with >= 3 internal nodes
- [ ] Dead code detection identifies functions with zero incoming non-containment edges, respecting all entry point heuristics
- [ ] Blast radius computed for every function node (transitive dependent count via reverse BFS)
- [ ] Depth complexity computed (max call depth, per-node depth from root)
- [ ] Instability metric computed for every module
- [ ] Package tangle index computed (global)
- [ ] Information flow complexity computed for every function node
- [ ] Hub score computed for function nodes bridging 3+ modules
- [ ] API surface ratio computed for every module
- [ ] Health score is a single integer 0-100 with per-component breakdown
- [ ] Per-node risk_level assigned based on cross-referencing all findings
- [ ] JSON output matches the contract in Section 6 exactly
- [ ] Output is deterministic (same input → same output, excluding timestamp)
- [ ] All severity classifications follow the rules in Section 6.2
- [ ] Finding descriptions are human-readable sentences (not raw IDs or metric values)

### 8.2 Performance Requirements

- [ ] 500-node graph completes in < 2 seconds
- [ ] 2,000-node graph completes in < 10 seconds
- [ ] 10,000-node graph completes in < 60 seconds
- [ ] Memory usage < 500MB for 10,000-node graph

### 8.3 Robustness Requirements

- [ ] Missing or empty database returns exit code 1 with helpful error message
- [ ] Database with 0 nodes returns valid JSON with health_score 100 and empty findings
- [ ] Database with 0 non-containment edges returns valid JSON (only dead code findings possible)
- [ ] Database with cycles that span 100+ nodes handles correctly (no stack overflow)
- [ ] Individual algorithm failure does not crash the process; other algorithms still run
- [ ] Analysis errors are reported in `analysis_errors` field

### 8.4 Integration Requirements

- [ ] Binary is discoverable by UE sidecar sync script (follows existing naming convention)
- [ ] Binary runs on Mac ARM64 (primary) with future cross-compilation to other platforms
- [ ] Binary has no runtime dependencies beyond the SQLite database file
- [ ] Binary can be invoked by UE's `FPlatformProcess::CreateProc` (same as graphengine-parsing and ge-template)
- [ ] Stderr output follows the progress format in Section 2.1 (UE may parse this for progress indication)

---

## 9. Test Strategy

### 9.1 Unit Tests (Per Algorithm)

Each analysis module must have unit tests using small, hand-constructed graphs.

**Cycle detection tests:**

| Test Case | Graph | Expected Output |
|-----------|-------|----------------|
| No cycles | A → B → C (linear) | 0 SCCs of size > 1 |
| Simple 2-cycle | A → B → A | 1 SCC: {A, B} |
| Simple 3-cycle | A → B → C → A | 1 SCC: {A, B, C} |
| Two independent cycles | A → B → A, C → D → C | 2 SCCs |
| Nested/overlapping cycles | A → B → C → A, B → D → B | 1 SCC containing all 4 nodes |
| Self-loop | A → A | 1 SCC: {A} (report only if size > 1 per spec, so skip) |
| Large cycle (100 nodes) | Ring of 100 → 100 nodes | 1 SCC of size 100 |
| Containment edges excluded | A →(Contains) B →(Call) C →(Call) A | 1 SCC: {A, C} (not B via containment) |

**Fan-in/fan-out tests:**

| Test Case | Graph | Expected |
|-----------|-------|----------|
| Isolated node | A (no edges) | fan_in=0, fan_out=0 |
| Linear chain | A → B → C | A: fi=0/fo=1, B: fi=1/fo=1, C: fi=1/fo=0 |
| Fan-out hub | A → B, A → C, A → D | A: fi=0/fo=3 |
| Fan-in sink | B → A, C → A, D → A | A: fi=3/fo=0 |
| Containment excluded | A →(Contains) B, C →(Call) B | B: fi=1 (only Call counted) |
| Hotspot threshold | 20 nodes, top 1 has fan_in=15 | Top node is_hotspot=true |

**Module coupling tests** (modules defined by path-prefix grouping per Section 3.3):

| Test Case | Graph | Expected |
|-----------|-------|----------|
| All internal edges | Analysis module M with A→B, B→C (all same prefix) | coupling=0.0 |
| All external edges | Analysis module M with A inside, A→X (X different prefix) | coupling=1.0 |
| Mixed | Analysis module M: 2 internal, 3 external | coupling=0.6 |
| Module with 1 node | Analysis module M with only A | Skipped (< 3 nodes) |
| Path-prefix grouping | `src/middleware/cors/a.ts` and `src/middleware/jwt/b.ts` both in A→B | internal (same prefix `src/middleware`) |
| Cross-prefix | `src/middleware/a.ts` → `src/router/b.ts` | external (different prefixes) |

**Dead code tests:**

| Test Case | Graph | Expected |
|-----------|-------|----------|
| Called function | A → B | B: not dead |
| Uncalled function | A (no incoming) | Flagged (unless entry point) |
| Test function | A with is_test=true, no incoming | Exempt |
| Index file export | A in `index.ts`, no incoming | Exempt |
| Framework handler | A named `handleRequest`, no incoming | Exempt |
| JSX component | A named `MyButton` (PascalCase), no incoming | Exempt |
| JSX intrinsic | A under `intrinsic_element::components::`, no incoming | Exempt |
| Property accessor | A named `url` under `MyClass`, no incoming | Exempt |
| Mock function | A named `mockFetch`, no incoming | Exempt |
| Real dead code | A in `src/utils/deprecated.ts`, no incoming, not test, not handler, not JSX, not accessor | Flagged |

**Blast radius tests:**

| Test Case | Graph | Expected |
|-----------|-------|----------|
| Leaf node | A → B, B → C | C: blast_radius=0 |
| Root node | A → B, A → C | A: blast_radius=0 (nobody depends on A) |
| Sink node | B → A, C → A | A: blast_radius=2 |
| Transitive chain | A → B → C → D | D: blast_radius=3 (A, B, C depend on D transitively) |
| DAG | B → A, C → B (so C transitively depends on A) | A: blast_radius=2 |

### 9.2 Integration Tests (Real SQLite Databases)

**Required test databases:**

| Database | Source | What It Tests |
|----------|--------|--------------|
| `test_empty.sqlite` | Empty database (0 nodes) | Edge case: empty project |
| `test_linear.sqlite` | 10-node linear chain | Baseline: no cycles, predictable fan-in/out |
| `test_cyclic.sqlite` | 20 nodes with 2 deliberate cycles | Cycle detection accuracy |
| `test_realistic_ts.sqlite` | Parse of a real 200-500 node TypeScript project | Full pipeline accuracy against known codebase |
| `test_large.sqlite` | 5,000+ node synthetic graph | Performance testing |

**The most important integration test:** Parse the GridSeak UE codebase itself (or the GraphEngine Rust codebase) with `graphengine-parsing`, then run `ge-analyze` on the result. Manually verify:
- Are the cycles real? (Check by reading the actual code)
- Are the hotspots meaningful? (Do they correspond to genuinely central functions?)
- Is the dead code actually uncalled? (Verify in the source)
- Is the health score intuitively correct? (Does it match your sense of the codebase's health?)

### 9.3 Acceptance Criteria for "Findings Feel Right"

This is the subjective but essential quality bar. Run `ge-analyze` on at least 3 real codebases and evaluate:

| Criteria | Pass | Fail |
|----------|------|------|
| **Cycle accuracy** | Every reported cycle is a real circular dependency verifiable in the source code | Any false positive cycle (nodes that don't actually form a dependency loop) |
| **Hotspot relevance** | Hotspot nodes are genuinely central functions that a developer would recognize as important | Hotspot flags trivial utility functions or framework boilerplate |
| **Coupling meaning** | High-coupling modules are ones a developer would agree "have too many external dependencies" | High coupling reported for modules that are intentionally well-connected (e.g., a shared utility module) |
| **Dead code precision** | >80% of flagged functions are genuinely uncalled (verify a random sample of 10) | >30% of flagged functions are actually called (just not through the graph — framework invocation, reflection, etc.) |
| **Health score intuition** | Well-maintained projects consistently rank above population median (percentile > 50 when norms available). Layer A metrics are factually accurate. | Rankings don't correlate with perceived code quality (clean project ranks below messy project) |
| **Finding count** | 10-60 total findings. Each finding type has at most 10 detailed + 1 overflow summary. | >100 findings (data dump) or <3 findings (too aggressive filtering) |
| **Finding descriptions** | A developer can read any finding description and immediately understand the problem without looking at raw data | Descriptions are cryptic, require knowledge of graph theory, or reference internal IDs |

#### 9.3.1 Empirical Calibration Data (2026-02-24)

The following results were obtained from running `ge-analyze` against Hono (11,395 nodes, 1,226 functions, 23,338 edges). This is the first calibration data point. The table records both the initial (pre-calibration) and post-calibration results.

| Metric | Pre-Calibration | Post-Calibration | Notes |
|--------|----------------|-----------------|-------|
| Health Score | 53 | 52 | Coupling avg rose due to correct module granularity; dead code improved |
| Total Modules | 88 (leaf folders) | 29 (path-prefix) | 88→29 is the correct architectural boundary |
| Avg Coupling | 0.565 | 0.640 | Rose because test modules (correctly) have high coupling |
| Dead Functions | 59 (76% FP) | 12 (~50% FP) | Remaining FPs are parser limitations, not heuristic gaps |
| Total Findings | 572 | 54 | From data dump to focused analysis |
| IFC Findings | 302 | 11 (10 + overflow) | 95th percentile threshold eliminated noise |
| Hub Findings | 174 | 11 (10 + overflow) | Percentile + module granularity fix |

**Health score interpretation (updated 2026-02-25):** The hand-tuned quadratic curve formulas have been replaced by a population-based percentile system. The previous issues with coupling_health (17/100 for Hono) and hotspot_concentration (12/100 for Hono) were symptoms of the fundamental problem: formula-based scores have no external meaning.

**Resolution:** The scoring system now operates in two modes:
1. **Percentile mode** (with `--norms`): Each metric is ranked against the population database. A coupling of 0.64 becomes "lower than 55% of projects" — a statement backed by data, not a curve.
2. **Fallback mode** (without norms): Simple linear penalties provide a conservative estimate. These are intentionally less opinionated than the previous quadratic curves.

**Calibration path forward:** Run `mass_calibrate.py` to build a population database of 300-500 public repos. Once populated, percentile rankings self-calibrate — no engineer ever needs to tune a curve again. See `SCORING_REDESIGN_AND_CALIBRATION_PIPELINE.md` for the full plan.

The raw Layer 1 metrics (cycles, fan-in, blast radius, depth) remain validated as accurate and are now surfaced directly in the `metrics` block.

---

## 10. Example Ideal Outputs

### 10.1 Healthy TypeScript Project (e.g., Zod)

```json
{
  "version": "1.0.0",
  "health_score": 84,
  "summary": {
    "total_nodes": 312,
    "total_edges": 891,
    "total_functions": 145,
    "total_modules": 12,
    "cycles_found": 0,
    "hotspot_count": 1,
    "high_coupling_modules": 0,
    "dead_functions": 8,
    "max_call_depth": 7,
    "tangle_index": 0.0,
    "avg_module_coupling": 0.22
  },
  "findings": [
    {
      "id": "hotspot-1",
      "type": "blast_radius_hotspot",
      "severity": "warning",
      "description": "ZodType.parse: called by 12 paths, affects 34 nodes",
      "node_ids": ["zod-type-parse"],
      "fan_in": 12,
      "blast_radius": 34,
      "recommendation": "This is likely intentional as the core parsing function. Monitor for growth."
    },
    {
      "id": "dead-1",
      "type": "potentially_unreachable",
      "severity": "info",
      "description": "8 potentially unreachable functions",
      "node_ids": ["..."],
      "count": 8
    }
  ]
}
```

### 10.2 Troubled TypeScript Project (e.g., a messy internal codebase)

```json
{
  "version": "1.0.0",
  "health_score": 41,
  "summary": {
    "total_nodes": 847,
    "total_edges": 2341,
    "total_functions": 312,
    "total_modules": 23,
    "cycles_found": 9,
    "cycle_total_nodes": 27,
    "hotspot_count": 4,
    "high_coupling_modules": 5,
    "dead_functions": 47,
    "max_call_depth": 18,
    "tangle_index": 0.23,
    "avg_module_coupling": 0.58
  },
  "findings": [
    {
      "id": "cycle-1",
      "type": "circular_dependency",
      "severity": "critical",
      "description": "auth → database → config → auth",
      "detail": "3-node circular dependency: authentication depends on database, database depends on config, config depends on authentication",
      "node_ids": ["auth-verify", "db-connect", "config-load"],
      "cycle_length": 3,
      "blast_radius": 34,
      "recommendation": "Extract shared types into a dedicated module that auth, database, and config all import. Break the config → auth edge."
    },
    {
      "id": "cycle-2",
      "type": "circular_dependency",
      "severity": "high",
      "description": "middleware/rateLimit → services/redis → middleware/rateLimit",
      "node_ids": ["rate-limit", "redis-client"],
      "cycle_length": 2,
      "blast_radius": 12,
      "recommendation": "Inject the Redis client into the rate limiter instead of importing it directly."
    },
    {
      "id": "hotspot-1",
      "type": "blast_radius_hotspot",
      "severity": "critical",
      "description": "handlePayment: called by 14 paths, affects 27 nodes",
      "node_ids": ["handle-payment"],
      "fan_in": 14,
      "blast_radius": 27,
      "recommendation": "Break handlePayment into smaller functions. Any change here risks breaking 27 downstream consumers."
    },
    {
      "id": "coupling-1",
      "type": "high_coupling",
      "severity": "high",
      "description": "auth/ module: coupling 0.78 (41 external vs 12 internal edges)",
      "node_ids": ["auth-module"],
      "coupling_score": 0.78,
      "recommendation": "The auth module has 41 connections outside itself. Consider making auth functions accept interfaces rather than importing concrete implementations."
    },
    {
      "id": "dead-1",
      "type": "potentially_unreachable",
      "severity": "info",
      "description": "47 potentially unreachable functions",
      "node_ids": ["..."],
      "count": 47
    },
    {
      "id": "hub-1",
      "type": "hub_node",
      "severity": "warning",
      "description": "connectPool bridges 6 distinct modules: auth, api, billing, notifications, analytics, admin",
      "node_ids": ["connect-pool"],
      "hub_score": 0.24,
      "recommendation": "connectPool is a critical cross-cutting dependency. Consider dependency injection to reduce direct coupling."
    }
  ]
}
```

### 10.3 What "Good Findings" Look Like

A finding is good if a senior engineer would read it and say one of:

- "I didn't know that." (novel insight)
- "I knew that was a problem but didn't realize how bad it was." (quantified known pain)
- "That explains the bug we had last month." (connects to real experience)
- "We should fix this before the next refactor." (actionable)

A finding is bad if a senior engineer would say:

- "That's not actually a problem." (false positive)
- "Of course that has high fan-in, it's the main API." (trivially obvious)
- "What does this even mean?" (unclear description)
- "Every project would have this." (non-specific / noise)

---

## 11. What This Enables (Business Context)

Understanding why each metric matters for the business helps prioritize correctly.

### 11.1 Audit Delivery (Revenue from Week 3-4)

The bootstrap GTM strategy requires `ge-analyze` output to deliver paid structural health audits ($3K-$15K per engagement). The health JSON is the foundation of every deliverable: health score, findings, recorded walkthrough.

**What matters most for audits:** Finding accuracy. A single false positive in a $10K presentation destroys credibility. Prioritize precision over recall for every algorithm.

### 11.2 Content Marketing (Proof from Week 2)

Publishing health analyses of popular open-source TypeScript repos (Next.js, tRPC, Prisma, Hono) is the core content strategy. Findings must be:
- Surprising enough to share ("Next.js has 3 circular dependency chains you can't see in the code")
- Specific enough to be credible (not "there are some coupling issues")
- Visually representable (the UE spatial view shows findings spatially)

**What matters most for content:** Interesting findings. If `ge-analyze` produces 50 trivial "info" findings and nothing dramatic, the content has no hook. Cycle detection and blast radius produce the most dramatic, shareable findings.

### 11.3 Self-Serve Product (Gate A, Months 2-3)

When the UE product ships, `ge-analyze` runs automatically after every parse. The health report drives the findings panel, overlays, and score display. The "60-second moment": parse → see → know what's wrong within 60 seconds.

**What matters most for self-serve:** Speed and reliability. Analysis must complete in <2s for typical projects. It must never crash. It must degrade gracefully.

### 11.4 MCP Server (Agent Integration)

The MCP server (Phase 5) wraps `ge-analyze` output as tools that coding agents call. `get_health`, `get_blast_radius`, `get_cycles` all read from the health JSON.

**What matters most for MCP:** Structured, complete JSON. Every field documented. No ambiguity in interpretation.

### 11.5 Tracking Architectural Drift Over Time

When `ge-analyze` runs on the same codebase at different points in time (before/after a refactoring sprint, weekly health checks, per-commit in CI), the health score and per-component breakdown become a **time series** that tracks architectural drift.

- Health score trending down = structural debt accumulating
- Cycle count trending up = dependency discipline failing
- Coupling scores trending up = module boundaries eroding
- Dead code ratio trending up = cleanup discipline failing
- Tangle index trending up = overall architecture deteriorating

This is the premium audit upsell: "You went from 58 to 67 in 4 weeks. Here's exactly what improved and what's still degrading." No competitor produces this composite time-series view.

---

## 12. Phased Implementation Order

### Phase 4 (P4-1) — Ship First (3-5 days)

Build these algorithms first. They produce the highest-impact findings for audits and content.

| Priority | Algorithm | Effort | Why First |
|----------|-----------|--------|-----------|
| 1 | Cycle detection (Tarjan's SCC) | 1 day | Most dramatic findings. "You have circular dependencies you've never seen." |
| 2 | Fan-in / Fan-out + hotspot detection | 0.5 day | Foundation for blast radius. Immediately useful for identifying central functions. |
| 3 | Blast radius (reverse BFS) | 1 day | Most actionable metric. "If you change X, these 27 things might break." |
| 4 | Module coupling | 0.5 day | Quantifies what developers feel but can't prove. |
| 5 | Dead code detection | 0.5 day | Easy win. Developers love cleaning up dead code. |
| 6 | Depth complexity | 0.5 day | Quick to implement, rounds out the health picture. |
| 7 | Health score composition | 0.5 day | Ties everything together into the single number. |
| 8 | JSON output + CLI | 0.5 day | Wiring. Uses existing Rust JSON/CLI patterns. |

### Phase 4 Extended — Additional Metrics (2-3 days)

These add depth but can follow immediately after the core ships.

| Priority | Algorithm | Effort | Why |
|----------|-----------|--------|-----|
| 9 | Instability (Martin) | 0.5 day | Complements coupling with direction information |
| 10 | Package tangle index | 0.25 day | Single number for overall cyclicality |
| 11 | Information flow complexity | 0.25 day | Catches bottleneck functions that fan-in alone misses |
| 12 | Hub score | 0.5 day | Identifies cross-cutting dependency bridges |
| 13 | API surface ratio | 0.25 day | Measures encapsulation quality per module |

### Phase 6 (Future — 15-20 days)

These require new data sources or parser changes. Design for them now, implement later.

| Algorithm | Data Source | Effort |
|-----------|-------------|--------|
| Cyclomatic complexity | AST (parser extension) | 3-4 days |
| Cognitive complexity | AST (parser extension) | Included with cyclomatic |
| Function/file LOC | Existing source location data | Trivial (already have start/end lines) |
| Temporal coupling | Git log analysis | 3-4 days |
| Coverage import | lcov file parsing | 2-3 days |
| SARIF import | SARIF JSON parsing | 2-3 days |
| Composite risk score | All above dimensions combined | 1-2 days |

---

## 13. Resolved Questions (Answers from Codebase Audit 2026-02-24)

| # | Question | Answer | Source |
|---|----------|--------|--------|
| 1 | Column names for node type and edge type? | **`kind`** for both nodes and edges. No `label` or `rel` columns exist. | `graphengine-parsing/src/infrastructure/storage/schema.rs` |
| 2 | Complete set of node types? | `Function`, `Struct`, `Module`, `Interface`, `Enum`, `Variable`, `Type`, `Import`, `Project`, `Crate`, `File`, `Folder`. No `Namespace` — `Module` covers that role. `Import`, `Project`, `Crate` are additional types not in the original spec. | `graphengine-parsing/src/domain/node.rs` (`NodeKind` enum) |
| 3 | Does `is_exported` exist? | **No.** Not a column, not a property key. Dead code detection must rely on containment-in-index-file heuristic and framework handler name matching. | Schema audit + grep of entire codebase |
| 4 | Maximum observed graph size? | **Benchmarked 2026-02-24:** Hono (284 TS files) → 11,395 nodes, 23,338 edges (11,395 Contains + 4,786 Uses + 4,330 Call + 2,382 Import + 445 Type). Parse time ~65s. DB size 18MB. Test fixture saved at `graphengine-analysis/test-fixtures/test_realistic_ts.sqlite`. | Hono parse run |
| 5 | Standalone binary or subcommand? | **Standalone binary.** Consistent with `ge-template` pattern. New crate `graphengine-health` or new `[[bin]]` in `graphengine-analysis`. | Architecture decision |
| 6 | Phase 6 CLI args now or later? | **Design now, parse but return "not yet implemented".** This validates the CLI contract early without implementation cost. | Architecture decision |

---

## 14. Post-Analysis Follow-Up Items (Parsing & Integration)

These items were identified during the 2026-02-24 parsing data quality audit. They do not block ge-analyze (which reads from SQLite directly) but should be addressed once ge-analyze ships, before the desktop app end-to-end pipeline is wired.

### 14.1 FQN Builder: Local Variable Collisions

**Problem:** `build_simple_fqn()` in `graphengine-parsing/src/syntax/utils/fqn_builder.rs` builds FQNs from the file path by extracting components after a `src/` directory marker. For TypeScript files **not** under `src/` (e.g., `runtime-tests/deno/hono.test.ts`), the entire directory context is dropped. Result: multiple files with the same stem produce identical FQNs for their local variables (e.g., 176 nodes all named `hono.test::res`).

**Impact:** Node IDs (SHA-256 of FQN + location) remain unique, so graph algorithms are unaffected. But FQN collisions:
- Make human-readable finding descriptions noisier for deeply-nested test variables
- Could cause issues if any future code assumes FQN uniqueness
- Affect the `module_fqns` map in the containment builder (mitigated by the `(FQN, file)` keying fix applied 2026-02-24, but the root cause remains)

**Fix:**
1. Modify `build_simple_fqn()` to accept an optional `workspace_root` parameter
2. When no `src/` directory is found, use the repo-relative path components as module context instead of dropping them entirely
3. For TypeScript specifically, the path between the workspace root and the file should always contribute to the FQN
4. Update all callers of `build_fqn()` / `build_simple_fqn()` to pass the workspace root (available in the parsing pipeline as `SyntaxResults.workspace_root`)

**Files to modify:**
- `graphengine-parsing/src/syntax/utils/fqn_builder.rs` — core FQN construction
- `graphengine-parsing/src/syntax/treesitter.rs` — passes workspace root to `build_fqn`
- `graphengine-parsing/src/syntax/extractors/symbol_extractor.rs` — same

**Effort:** 0.5–1 day. Requires re-parsing test fixtures and updating any golden tests that assert specific FQN strings.

**When:** After ge-analyze ships. Running ge-analyze on real codebases will reveal whether FQN collisions surface in findings, giving empirical data on priority.

### 14.2 ParserService Implementation (Desktop App Integration)

**Problem:** `SubmitRepoUseCase` in `graphengine-core` now defines a `ParserService` port (trait), but no concrete implementation exists. The port was reconnected on 2026-02-24 (previously the entire `execute()` method returned `Ok(())` with all parsing commented out). The desktop app and UE sidecar need an implementation to invoke `graphengine-parsing` programmatically.

**Design decision:** The implementation should invoke the `graphengine-parsing` binary as a subprocess (consistent with how UE invokes it via `FPlatformProcess::CreateProc` and how `ge-template` is invoked). This keeps the clean architecture boundary — core defines the port, infrastructure provides a subprocess-based implementation.

**Implementation outline:**
```rust
pub struct SubprocessParserService {
    binary_path: PathBuf,
}

#[async_trait]
impl ParserService for SubprocessParserService {
    async fn parse_repository(&self, repo_path: &Path, language: &str, db_path: &Path) -> Result<ParseResult> {
        let output = tokio::process::Command::new(&self.binary_path)
            .args(["parse", "--root", repo_path.to_str().unwrap(),
                   "--lang", language, "--db", db_path.to_str().unwrap()])
            .stderr(std::process::Stdio::piped())
            .output()
            .await?;
        // Parse exit code per Section 2.1 contract, extract stats from stderr
    }
}
```

**Files to create/modify:**
- `graphengine-infra/src/services/parser_service_impl.rs` — subprocess-based implementation
- `graphengine-infra/src/services/mod.rs` — register the module
- Desktop app composition root — wire `SubprocessParserService` into `SubmitRepoUseCase`

**Effort:** 0.5 day for the implementation, 0.5 day for wiring into the desktop app.

**When:** When integrating ge-analyze into the desktop app pipeline (Phase 4-2 onwards). This is the natural point where "parse → analyze → display" needs to be a single flow.

### 14.3 Containment Tree Fixes Applied (Reference)

The following containment tree bugs were found and **already fixed** during the 2026-02-24 audit. Documenting here for context:

| Bug | Root Cause | Fix Applied |
|-----|-----------|-------------|
| Every File had 2 parents (Project + Folder) | `Project → File` edges created for ALL files, not just root-level | Only connect Project to files/folders without a folder parent |
| Modules had 2 parents (FQN hierarchy + File mapping) | Both `Module → Module` and `File → Module` containment edges created | Skip `File → Module` when module already has an FQN parent |
| 18 Module nodes orphaned (no parent) | FQN collisions in `module_fqns: HashMap<String, String>` silently dropped duplicate modules | Key modules by `(FQN, file_path)` with same-file preference |

**Validation result:** After fix, 11,395 nodes each have exactly 1 containment parent. Contains edges = 11,395 (one per non-root node). Zero orphans, zero multi-parent nodes.

---

## 15. Relationship to Other Documents

> Note: Section 14 was previously "Relationship to Other Documents". It was renumbered to Section 15 when Section 14 (Post-Analysis Follow-Up Items) was added on 2026-02-24.

| Document | Relationship |
|----------|-------------|
| `PHASE_4_STRUCTURAL_HEALTH.md` | This spec expands P4-1. The Phase 4 doc defines the full UE integration (P4-2 through P4-6). |
| `PHASE_6_DEEP_HEALTH_INTELLIGENCE.md` | Future extensions. This spec's Section 7.5 (extensibility) and Section 2.1 (optional arguments) are designed for Phase 6. |
| `ARCHITECTURE.md` | `ge-analyze` fits into the pipeline: `Parse → SQLite → ge-analyze → Health JSON → UE Visualization + Overlays` |
| `docs/01-status/CURRENT_STATE.md` | Notes `ge-analyze` as "Not yet built." Update when shipped. |
| `~/Desktop/GridCash/strategy/GO_TO_MARKET.md` | Revenue from audits is gated on `ge-analyze` existing. (Business doc — lives in GridCash workspace.) |
| `~/Desktop/GridCash/strategy/BUSINESS_STRATEGY.md` | Content marketing strategy requires running `ge-analyze` on popular open-source repos. (Business doc — lives in GridCash workspace.) |
| `MVP_SHIP_GATE.md` | Phase 4 section of the ship gate requires health analysis working end-to-end. |
