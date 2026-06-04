## GraphEngine Template Query Contract + Traversal Plan (High-Integrity, AI-Friendly)

### Status
- **Type**: Plan + contract proposal
- **Scope**: `ge-template query` / `TemplateService` semantics over the parsing DB (`nodes`/`edges` tables)
- **Goal**: Make template queries **truthful, deterministic, explicit**, and safe for **AI-generated TOML**.

---

## Why this exists (product + integrity)
GridSeak’s promise is “truthful visualization”: clients must be able to say:
- “These are the facts returned by the query.”
- “These are view-policy transforms applied on top.”

Templates are the bridge between user/AI intent and GraphEngine facts. If template semantics are ambiguous (or silently ignored), AI cannot safely generate them, and downstream consumers (Unreal/GridSeak) cannot trust payloads for caching/diffing/UI stability.

---

## Current state (observed + fixed)

### Today’s implementation reality (must be explicit)
`TemplateService` currently behaves like a **filtered dump**:
- **Nodes**: selected by `node_filter` (plus some properties-based predicates).
- **Edges**: selected by `edge_filter`.
- **Traversal**: `seed/depth/direction` are parsed/logged but **not** used to walk the graph.

### Recently fixed (subgraph consistency contract, externals off)
When `show_externals=false`, emitted edges are now restricted so:
- edges satisfy `edge_filter`
- both endpoints exist in `nodes[]`

This eliminates “dangling edges” and “unexpected edge types” for closed-subgraph queries.

---

## The high-quality target (Option B)
**Yes: the highest-quality option is “Option B”**: implement traversal semantics so templates mean what they say.

However, the best real-world design is a **two-mode system**:
- **Mode 1: Filtered Dump** (explicit, useful for “give me everything matching constraints”)
- **Mode 2: Traversal Query** (seeded walk with depth/direction)

The crucial integrity rule: **Templates must never “pretend” to do one mode while actually doing the other.**

---

## Contract v1: Output semantics (what clients can rely on)

### Top-level payload
Payload MUST include:
- `nodes[]`
- `edges[]`
- `metadata{ ... }`

### Metadata schema (exact fields for v1)
`metadata` MUST be present and MUST include:
- `contract_version`: string, fixed for this contract (e.g. `"template_query_v1"`)
- `query_mode`: `"filtered_dump" | "traversal"` (the actual executed mode)
- `externals`: object describing external endpoint behavior
- `capabilities`: machine-readable feature flags (see below)

`metadata` SHOULD include (when available):
- `schema_version`: parsing DB schema version (string/int)
- `recommended_view_roots`: array (already present today; derived from Project node properties)
- `warnings[]`: array of strings (e.g. truncation, broad seeds, deprecated keys)

#### Example payload (shape + intent)

```json
{
  "nodes": [
    {
      "id": "n_folder",
      "type": "Folder",
      "fqn": "src",
      "location": null,
      "provenance": null,
      "properties": { "path_repo_rel": "src" }
    }
  ],
  "edges": [
    { "source": "n_folder", "target": "n_file", "type": "Contains", "provenance": null }
  ],
  "metadata": {
    "contract_version": "template_query_v1",
    "query_mode": "filtered_dump",
    "schema_version": "parsing_db_vX",
    "externals": { "show_externals": false, "mode": "closed_subgraph" },
    "capabilities": {
      "traversal_supported": false,
      "externals_modes_supported": ["closed_subgraph", "stubs"],
      "template_fields": {
        "node": [
          "node_type",
          "properties.role",
          "properties.is_vendor",
          "properties.is_build_output",
          "properties.is_generated",
          "properties.is_test",
          "properties.path_repo_rel"
        ],
        "edge": ["rel"]
      },
      "operators": ["==", "in", "starts_with"],
      "limits": { "max_depth": 25, "max_nodes": 50000, "max_edges": 200000 }
    },
    "recommended_view_roots": [],
    "warnings": []
  }
}
```

### Nodes
1) Every returned node MUST satisfy `node_filter` (if provided).
2) Node IDs are unique.
3) Node ordering MUST be deterministic (see “Determinism”).

### Edges
1) Every returned edge MUST satisfy `edge_filter` (if provided).
2) Ordering MUST be deterministic.
3) External endpoint rules depend on `show_externals`:

#### If `show_externals=false` (closed subgraph)
- Every edge endpoint MUST exist in `nodes[]`.

#### If `show_externals=true` (externals allowed)
Pick ONE semantics and make it a contract:

- **Preferred (consumer-friendly)**: **materialize external node stubs**
  - If an edge endpoint is not selected by `node_filter` but is needed as an endpoint, include it in `nodes[]` as a stub.
  - Stub nodes MUST be clearly marked, e.g. `properties.is_stub=true` and `provenance.confidence="Low"`.
  - Stub nodes SHOULD include best-known fields (`id`, `type` if known, `fqn`, and any safe properties like `path_repo_rel`).

- **Alternative (simpler)**: allow dangling endpoints
  - edges may reference IDs not present in `nodes[]`.
  - If chosen, this MUST be clearly marked in `metadata.externals_mode="dangling"`, and clients must handle it.

Recommendation: **stubs**. They preserve determinism and eliminate downstream “drop edges” logic.

---

## Contract v1: Determinism requirements (non-negotiable)
To support caching/diffing and stable UI:

### Stable ordering
- Nodes MUST be sorted by a stable key (e.g. `type`, then `fqn`, then `id`).
- Edges MUST be sorted by (`type`, `source`, `target`) or equivalent.

### Stable selection
Given the same DB + same template, output MUST be identical (byte-for-byte if pretty-print disabled), except for explicitly time-varying metadata fields (which should be avoided or separated).

### Stable tie-breaking
Whenever the engine must choose among equals (e.g. canonical parent in a “tree mode”), tie-break rules must be:
- deterministic
- documented
- based on stable keys (paths/FQNs/IDs), never iteration order.

---

## Template semantics v1 (inputs that AI can reason about)

### The big design goal
AI (and humans) must be able to look at a TOML template and know:
- **Which engine mode** it will run (`filtered_dump` vs `traversal`)
- Which fields are **supported** (validated) vs **rejected**
- What “externals” will mean (stubs vs dangling)
- Whether the result will be a closed subgraph

### Proposed minimal TOML schema (explicit mode + clear seeds)

```toml
[perspective]
name = "Filesystem: folders/files contains"

[seed]
# Multi-root is first-class. At least one required for traversal mode.
# Seeds can be node IDs or resolvable selectors.
roots = [
  { by_path_repo_rel = "src" },
  { by_path_repo_rel = "packages/app" }
]

[graph]
mode = "traversal"            # "filtered_dump" | "traversal"
depth = 10
direction = "out"             # "out" | "in" | "both"
node_filter = "node_type in ['Folder','File']"
edge_filter = "rel == 'Contains'"
show_externals = false

[engine]
prefer = "sql"
```

Notes:
- `seed.roots` is a list: this is the simplest way to support “multiple roots” without inventing complex syntax.
- `graph.mode` prevents silent mismatch. If omitted, the engine must choose a default AND record it in output metadata.

### Seed resolution (how the engine finds start nodes)
Traversal requires seeds to be resolved to concrete node IDs. Seeds SHOULD support:
- `by_id`: explicit node ID
- `by_semantic_id`: stable semantic identifier (if your parsing DB provides one)
- `by_path_repo_rel`: resolve to Folder/File nodes with matching repo-relative path
- `pattern`: legacy “%” style selection (discouraged for traversal; too broad for AI)

Seed resolution MUST be:
- deterministic
- explainable (record resolution counts and selected IDs in metadata when `--explain` is requested)
- validated (if a seed selects 0 nodes, that should be an explicit error or explicit empty result; no silent fallback)

### Filters (node_filter / edge_filter)
Filters MUST be either:
- fully supported (parsed + applied), or
- rejected at template validation time.

No more “looks valid but ignored”.

For Contract v1, define a strict subset, for example:
- Node fields:
  - `node_type` (maps to `nodes.kind`)
  - `properties.role`, `properties.is_vendor`, `properties.is_build_output`, etc.
  - `properties.path_repo_rel` with `starts_with`
- Edge fields:
  - `rel` (maps to `edges.kind`)

### Contract v1 supported filter subset (validator truth table)
This table is what the template validator enforces. Anything else is a **hard error** (not ignored).

#### Node filters
Supported expressions (case-sensitive keys; values are quoted):
- **Kinds**:
  - `node_type == 'File'`
  - `node_type in ['Folder','File']`
- **Properties** (requires `nodes.properties` JSON column):
  - `role == 'source'`
  - `role in ['source','test']`
  - `is_vendor == true|false`
  - `is_build_output == true|false`
  - `is_generated == true|false`
  - `is_test == true|false`
  - `path_repo_rel starts_with 'src/'`

Rejected (v1):
- arbitrary boolean logic (AND/OR/NOT), parens
- regex
- numeric comparisons
- unknown keys

#### Edge filters
Supported expressions:
- `rel == 'Contains'`
- `rel in ['Contains','Call']`

Rejected (v1):
- `is_internal_call` (unless/ until implemented)
- regex
- unknown keys

### External endpoints (show_externals)
If `show_externals=true`, Contract v1 SHOULD standardize on:
- `externals_mode = "stubs"`
- stub nodes are included even if they fail `node_filter`, but are marked as stubs.

This is the most consumer/AI-friendly option because it preserves “edges always have endpoints”.

---

## Traversal semantics (Option B) — what “depth/direction” really mean

### The core behavior
Given seed node set \(S\), produce a result graph \(G'\) by walking edges:
- Choose edges eligible for traversal by `edge_filter` (and possibly by direction).
- Expand frontier up to `depth`.
- The resulting node set includes every visited endpoint (and, if externals stubs are enabled, any endpoint required by returned edges).

### Direction
- `out`: follow edges from `from_id -> to_id`
- `in`: follow edges where `to_id` points back to the current node (`from_id <- to_id`)
- `both`: union of `out` and `in`

### Depth
Depth is hop-count in the traversed edge graph.
- `depth=0`: return only resolved seed nodes, and (optionally) zero edges.
- `depth=1`: include edges adjacent to seeds (subject to filters) and their endpoints.

### How filters interact with traversal (important!)
There are two valid semantics; pick one and document it:

#### Semantics A (recommended): “edge filter controls traversal; node filter controls emission”
- Traversal expands along edges that match `edge_filter`.
- Nodes are emitted if they match `node_filter`, OR if they are required as stubs when externals are enabled.

Why this is good:
- You can traverse along “Contains” edges while restricting emission to files/folders.
- You can traverse Calls but only emit Functions.

#### Semantics B: “node filter also gates traversal”
- Only traverse into nodes that match `node_filter`.

This can be useful but is easy to misunderstand; it can also prematurely cut traversals.

Recommendation: **Semantics A**, and if needed add an explicit flag later:
`graph.traversal_node_gate = true|false`.

---

## Implementation plan (phased, quality-first)

## Execution sequence (recommended order + exit criteria)
This is the concrete build order that keeps integrity high at every step.

### Step 0 — Publish the contract boundaries (no more ambiguity)
- **Action**: Declare `Template Query Contract v1` as the external truth surface for `ge-template query`.
- **Action**: Declare the current behavior as `graph.mode="filtered_dump"` until traversal exists.
- **Exit criteria**:
  - Docs explicitly say seed/depth/direction are ignored in filtered-dump mode.
  - Output metadata contains `metadata.contract_version`.
  - Template validation rejects unsupported predicates (no silent ignores).

### Step 1 — Add explicit `graph.mode` + strict validation (AI safety foundation)
- **Action**: Add `graph.mode = "filtered_dump" | "traversal"` to template schema.
- **Action**: Implement strict template validation:
  - reject unknown operators/fields
  - reject ambiguous legacy keys unless mapped explicitly (e.g. `label` → `node_type` if supported)
  - for traversal: reject missing seeds or invalid direction/depth
- **Action**: Add `--explain` (or always-on metadata) to expose:
  - selected mode
  - normalized filters
  - seed resolution summary
- **Exit criteria**:
  - Templates that previously “worked but were ignored” now fail fast with a clear error.
  - AI can deterministically infer what will happen from the template + capabilities metadata.

### Step 2 — Deterministic output ordering + version/capabilities (caching + diff stability)
- **Action**: Sort nodes/edges deterministically.
- **Action**: Add the following metadata (stable, machine-readable):
  - `metadata.contract_version = "template_query_v1"`
  - `metadata.capabilities = {...}` (supported filter fields/operators, externals mode, traversal support)
  - `metadata.externals_mode = "stubs"` (once chosen)
  - `metadata.query_mode = "filtered_dump" | "traversal"`
- **Exit criteria**:
  - Same DB + same template yields byte-stable JSON (pretty-print aside).
  - Downstream can gate behavior on `contract_version` and `capabilities`.

### Step 3 — Lock externals-on semantics with tests (choose stubs vs dangling)
- **Decision**: Choose externals semantics. Recommendation remains **stubs**.
- **Action**: Add regression tests for `show_externals=true`:
  - if stubs: edges MUST have endpoints, and stub nodes are marked
  - if dangling: metadata MUST declare dangling mode, and behavior is deterministic
- **Exit criteria**:
  - External-mode behavior is explicit, documented, tested.
  - Consumers never need to “guess” how to interpret missing endpoints.

### Step 4 — Add an explicit edge-filter test (clarity + future-proofing)
- **Action**: Add a focused test where:
  - endpoints are inside the node set
  - both `Contains` and `Call` edges exist
  - `edge_filter="rel == 'Contains'"` yields only `Contains`
- **Exit criteria**:
  - Edge filtering correctness is locked independent of endpoint restriction.

### Step 5 — Implement traversal (Option B) behind `graph.mode="traversal"`
- **Action**: Implement seed resolution (multi-root) to node IDs.
- **Action**: Implement traversal via SQL recursive CTE:
  - direction out/in/both
  - depth hop limit
  - edge_filter applied to traversal edges
  - define Semantics A vs B for node_filter gating (recommend Semantics A)
- **Action**: Emission rules:
  - nodes satisfy node_filter (plus external stubs if enabled)
  - edges satisfy edge_filter
  - endpoint constraints follow externals mode
- **Exit criteria**:
  - `depth/direction` demonstrably change results in tests.
  - Traversal results remain deterministic and validated.

---

## Traversal SQL sketch (recursive CTE) — concrete starting point
This section is intentionally “implementation-shaped” to reduce ambiguity.

### 1) Resolve multi-root seeds to node IDs
Example for `by_path_repo_rel` (Folder/File roots):

```sql
-- Inputs:
--   :path_repo_rel values (e.g. 'src', 'packages/app')
SELECT id
FROM nodes
WHERE json_extract(properties, '$.path_repo_rel') IN (:root1, :root2)
  AND kind IN ('Folder','File');
```

### 2) Traverse edges up to depth (Semantics A)
Key design: `edge_filter` gates traversal; `node_filter` gates emission.

```sql
-- Parameters:
--   :max_depth (int)
--   :direction ('out'|'in'|'both') handled by selecting the appropriate join(s)
--   :edge_kinds list from edge_filter (e.g. ('Contains'))
WITH RECURSIVE
seed(id) AS (
  SELECT id FROM nodes WHERE id IN (/* resolved seed ids */)
),
walk(node_id, depth) AS (
  SELECT id, 0 FROM seed
  UNION ALL
  SELECT
    -- out: move to e.to_id ; in: move to e.from_id ; both: handle in join
    CASE WHEN :direction = 'in' THEN e.from_id ELSE e.to_id END AS node_id,
    w.depth + 1 AS depth
  FROM walk w
  JOIN edges e
    ON (
      (:direction = 'out' AND e.from_id = w.node_id)
      OR (:direction = 'in' AND e.to_id = w.node_id)
      OR (:direction = 'both' AND (e.from_id = w.node_id OR e.to_id = w.node_id))
    )
  WHERE e.kind IN (/* :edge_kinds */)
    AND w.depth < :max_depth
),
visited_nodes AS (
  SELECT DISTINCT node_id AS id FROM walk
),
visited_edges AS (
  -- For initial v1 traversal, emit edges whose BOTH endpoints are visited (closed by traversal reachability).
  SELECT DISTINCT e.from_id, e.to_id, e.kind, e.provenance
  FROM edges e
  JOIN visited_nodes v1 ON v1.id = e.from_id
  JOIN visited_nodes v2 ON v2.id = e.to_id
  WHERE e.kind IN (/* :edge_kinds */)
)
SELECT * FROM visited_nodes;
-- plus a second query to SELECT * FROM visited_edges
```

### 3) Emission rule (nodes + stubs) in SQL terms
- `nodes_emitted = visited_nodes ∩ node_filter_matches` (plus stub nodes if externals enabled)
- `edges_emitted = visited_edges ∩ edge_filter_matches` AND (endpoint rule depending on externals mode)

If `show_externals=true` with `externals.mode="stubs"`:
- an edge is allowed to have an endpoint that fails `node_filter`, but the endpoint MUST appear in `nodes[]` as a stub.

### Step 6 — Performance + guardrails (make it safe for AI to use at scale)
- **Action**: Add practical bounding rules:
  - maximum node/edge caps (with explicit “truncated” metadata if exceeded)
  - validation warnings for “broad seeds” in traversal mode (e.g. `pattern="%"`)
- **Action**: Add indexes if needed for hot filters (especially JSON properties).
- **Exit criteria**:
  - Worst-case templates don’t DOS the system silently; they fail fast or truncate explicitly.

### Step 7 — (Optional) Containment tree hints (DAG truth + consumer ergonomics)
- **Action** (optional): Add `preferred_parent_id` hint or `ContainsPreferred` edges for “folder tree” views.
- **Exit criteria**:
  - Truth (DAG) remains available; hints are advisory and deterministic.

### Phase 0: Contract + validation + explainability (highest leverage)
- **Add `graph.mode`** and enforce it.
- Implement a strict template validator:
  - reject unsupported predicates/operators
  - reject missing required keys for traversal mode (e.g. no seeds)
- Add `ge-template --explain`:
  - prints a machine-readable “execution plan”:
    - mode selected
    - seeds resolved (counts + IDs)
    - SQL queries compiled (or at least a normalized representation)
    - externals behavior

Deliverable: AI can generate TOML without guessing what the engine will do.

### Phase 1: Deterministic output guarantees
- Add deterministic ordering in the query emitter (nodes/edges sort).
- Add `metadata.contract_version = "template_query_v1"`.
- Add `metadata.capabilities` describing supported operators/features.

Deliverable: caching/diff/CI tests become reliable.

### Phase 2: Traversal engine (SQL-first)
Implement traversal using SQLite **recursive CTE** so performance scales and behavior is deterministic.

High-level algorithm:
1) Resolve seeds to seed node IDs (SQL select).
2) Recursive CTE to compute visited nodes/edges up to depth:
   - base: seeds
   - step: join edges by direction, apply edge_filter, advance hop
3) Emission:
   - nodes: visited nodes filtered by node_filter (plus stub logic)
   - edges: visited edges filtered by edge_filter, then restricted to emitted endpoints according to externals rules

Deliverable: `depth/direction` become real and trustworthy.

### Phase 3: Externals as stubs (recommended)
When `show_externals=true`, materialize stub nodes for endpoints that are referenced by emitted edges but not selected by node_filter.

Deliverable: clients never have to drop edges; AI gets consistent graphs.

### Phase 4: Multi-root ergonomics (AI + UX)
- Enhance seed selection helpers:
  - `by_path_repo_rel` for folders
  - optional `by_glob_repo_rel` (careful: must be validated and bounded)
- Add a template lint rule: warn/error on “broad seeds” in traversal mode.

Deliverable: AI can express “multiple roots” safely and precisely.

---

## Testing strategy (lock the contract)
Add explicit regression tests that cover:

### Closed subgraph (already added)
- `show_externals=false` ensures no dangling endpoints.

### Externals ON contract (must add)
- If stubs mode:
  - edges may reference nodes that fail node_filter, but those nodes MUST appear as stub nodes.
  - stubs are marked and deterministic.

### Edge filter correctness (explicit)
- Even when both endpoints are in the node set, `edge_filter="rel == 'Contains'"` must remove non-Contains edges.

### Traversal correctness
- depth 0/1/2 behavior from known seeds
- direction in/out/both
- filter interaction semantics A vs B (whichever is chosen)

### Validation behavior
- unsupported predicates cause a hard error (no silent ignore)
- missing required keys in traversal mode cause a hard error

---

## Versioning / compatibility (so clients can gate)
Add to stdout payload:
- `metadata.contract_version`: e.g. `"template_query_v1"`
- `metadata.schema_version`: parsing DB schema version (see `ARCHITECTURAL_NEEDS_ProjectClassification_ViewRoots.md`)
- `metadata.capabilities`: list of supported template operators/features

This allows GridSeak/Unreal/AI tooling to:
- refuse unknown/old contracts
- branch behavior safely
- keep caches consistent

---

## Containment DAG vs Tree (consumer expectations)
GraphEngine facts may legitimately contain multiple `Contains` parents (DAG). For “folder views” consumers often want a tree.

GraphEngine should remain truthful by default (emit DAG), but can help deterministically:
- Add optional hints (not enforced policy):
  - `properties.preferred_parent_id` on nodes (computed deterministically) OR
  - additional edge type `ContainsPreferred`

If GraphEngine does not own canonicalization (reasonable), Contract v1 still must ensure:
- containment edges are self-consistent with the returned node set
- consumer can canonicalize deterministically without missing endpoints

---

## Summary: What “high integrity AI control” looks like
When an AI generates a TOML template, it must be able to rely on:
- explicit mode (`filtered_dump` vs `traversal`)
- strict validation (no silent ignores)
- explicit externals semantics (prefer: stub nodes)
- deterministic output ordering + stable tie-breaks
- versioned contract + capabilities metadata
- explainability (`--explain`) for debugging and safe prompt-tooling loops

