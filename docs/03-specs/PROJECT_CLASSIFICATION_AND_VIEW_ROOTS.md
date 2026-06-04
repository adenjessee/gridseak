# Project Classification + Recommended View Roots

## Purpose

GraphEngine is the source of truth for parsed facts (nodes, edges, provenance). This document specifies what GraphEngine must produce so every consumer (ge-analyze, web reports, MCP server, future visualization clients) can:

- Present a **clean, meaningful root** without lying about the filesystem.
- Filter/label content consistently (source vs tests vs build output vs vendor vs generated).
- Avoid consumer-specific heuristics and drift.

This spec is **not** about layout or visualization. Layout remains a consumer concern.

---

## Problem Statement

When parsing a project, the resulting graph can show confusing "root containers":

- A container that is an **absolute path** (workspace/project root).
- A folder like `typescript` (real folder under `src/typescript` in that project).
- Other roots such as individual files, depending on the template.

Users interpret unexpected roots as "fake containers" or "bloat," even when the data is technically correct.

The core problems are:

- **Facts vs view-policy are conflated** (consumers guess what's meaningful).
- **No universal classification contract** (consumers can't reliably hide build output/vendor/generated).
- **No universal "recommended scope roots"** (consumers implement brittle rules like "skip single wrapper folder").

---

## Non-Goals

- Do not hardcode UI policy in GraphEngine (no "always collapse X").
- Do not delete or hide real filesystem structure in GraphEngine output.
- Do not require AI for correctness. AI may be added later, but deterministic classification must exist.

---

## Definitions

- **Workspace Root**: The directory GraphEngine is asked to parse (CLI `--root`) and/or the LSP workspace root.
- **Project Node**: A `NodeKind::Project` node representing a workspace root container.
- **Physical Hierarchy**: Folder/File containment derived from paths on disk.
- **Logical Hierarchy**: Language module/namespace containment derived from language semantics.
- **Classification**: A stable labeling of filesystem content into roles (source/tests/build/vendor/generated/etc.).
- **Recommended View Roots**: Ranked candidate directories representing meaningful "scope roots" for viewing (e.g. `src/`, `src/typescript/`, `packages/*`).

---

## Responsibility Boundary

GraphEngine should own:

- **Cross-language knowledge** (tsconfig/Cargo/sln/pom/etc.)
- **DB schema + properties** consumed by templates and consumers
- **Determinism + provenance** for all claims

GraphEngine should *not* own:

- Which root is selected for a particular user/view
- UI collapse/expand defaults
- Layout math / spatial arrangement

---

## Required GraphEngine Outputs

### 1) Always emit a Project node per workspace root (already present)

GraphEngine parsing currently creates a `NodeKind::Project` from `SyntaxResults.workspace_root` and emits `Contains` edges:

- `Project → Folder`
- `Project → File`
- (Rust) `Project → Crate`, etc.

### 2) Emit both physical + logical hierarchy when possible

- Physical: `Folder` and `File` nodes + `Contains` edges derived from paths.
- Logical: `Module` (and language equivalents) nodes + `Contains` edges derived from semantic FQNs.

Consumers can decide which lens to apply (or show both).

### 3) Universal classification contract (deterministic)

GraphEngine must classify files/folders into universal roles.

#### Universal roles (baseline)

- `source`
- `test`
- `docs`
- `tooling`
- `build_output`
- `generated`
- `vendor`
- `unknown`

#### Required properties (per File and Folder nodes)

Store as DB properties so templates can filter without consumer hacks.

| Property | Type | Description |
|---|---|---|
| `path_abs` | string | Absolute path |
| `path_repo_rel` | string | Repo-relative path |
| `role` | string | One of the universal roles |
| `role_confidence` | string | `high` / `medium` / `low` |
| `role_reason` | string | e.g. "matched tsconfig include", "under dist/", "node_modules" |
| `is_generated` | bool | |
| `is_vendor` | bool | |
| `is_build_output` | bool | |
| `is_test` | bool | |
| `language` | string | e.g. `typescript` |

#### Provenance requirement

Classification must have provenance:
- `provenance.source`: `detector` / `manifest` / `heuristic` / `ai`
- `provenance.confidence`: high/medium/low

**Key rule**: classification must be explainable (reason string) and reproducible.

### 4) Emit "Recommended View Roots" as data (ranked candidates)

These candidates are **not policy** — they are recommendations that consumers can accept or override.

#### Example candidate schema

- `root_path_repo_rel`: `src/typescript`
- `confidence`: `high`
- `reason`: `tsconfig.json rootDir/include resolved to src/typescript`
- `tags`: `[source]`

#### Storage mechanism

**Preferred**: store candidates in DB meta table keyed by repo/run:
- `meta.recommended_view_roots = JSON([...])`

Alternative: emit via stdout payload as a top-level `metadata` object alongside nodes/edges.

---

## Detector Pipeline

Implement language/framework-aware detectors that output universal roles.

### Detector inputs (universally available)

- workspace root path
- file tree snapshot
- known manifests (presence + parsed contents)
- parse results (which files contain parseable nodes; call/import density)

### Deterministic detectors by language

**TypeScript/JavaScript**
- manifests: `package.json`, `tsconfig.json`, `eslint`, `vite`, `next`, etc.
- `node_modules/` → `vendor`
- `dist/`, `build/`, `out/`, `dest/` → `build_output`
- `.d.ts` (sometimes generated), `*.generated.*`, `*.min.js` → `generated` (heuristic)
- recommended roots: `tsconfig.compilerOptions.rootDir` > `tsconfig.include` > `src/` > best source cluster

**Rust**
- `Cargo.toml` workspace members define candidate roots
- `target/` → `build_output`
- `vendor/` → `vendor`

**C#**
- `.sln` and `.csproj` define candidate roots
- `bin/`, `obj/` → `build_output`

**Python**
- `pyproject.toml` / `setup.cfg`
- `.venv/` → `vendor`
- `__pycache__/` → `build_output`

**Java**
- `pom.xml` / `build.gradle`
- `target/` / `build/` → `build_output`

### AI-assisted classification (optional, later)

- Must be stored with `provenance.source = ai`
- Must be overrideable
- Must not replace deterministic rules for common patterns

### Detector architecture

Create a modular service boundary:
- `ProjectClassifier` (port) returning `ClassificationMap + ViewRootCandidates + Evidence`
- Infra adapters: `TsClassifier`, `RustClassifier`, `PythonClassifier`, etc.
- Pipeline order: manifest-driven (high confidence) → directory heuristics (medium) → filename heuristics (low) → AI suggestions (explicitly tagged)

---

## Contract with Templates

Templates must be able to filter by:

- `node_type` (File/Folder/Module/Function/etc.)
- `role` / `is_vendor` / `is_build_output` / `is_generated` booleans
- `path_repo_rel` prefix matches (for scope roots)

---

## Architectural Analysis

### Current State

- **Node properties**: File/Folder nodes carry a stable `properties` JSON object (classification + paths).
- **Metadata**: Query output includes `metadata.recommended_view_roots` (from Project node properties).
- **Template-side filtering**: Can filter by `role`, `is_vendor`, `is_build_output`, `path_repo_rel starts_with`, etc.

### Key Drift Risk

Two overlapping stacks represent "GraphEngine":
- Newer `graphengine-parsing` stack (nodes/edges tables, deterministic IDs, JSON properties column).
- Older `graphengine-core/storage/infra` stack (different schema, "label/lang/module" style template expectations, extid mapping).

Without an explicit ownership decision, consumers will experience contradictions.

### Architectural Needs

**1) Single canonical graph model** (highest priority)
- Declare `graphengine-parsing` DB contract as the canonical contract.
- Treat the other schema as legacy behind an explicit adapter boundary.
- Add a DB schema version marker so tools can refuse to run against the wrong DB.

**2) Facts vs policy boundary**
- Introduce `GraphMetadata` structure: parse-run info, recommended roots, classification summary.
- Keep metadata explicitly advisory.

**3) Determinism and provenance**
- Maintain separate provenance namespace for classification (don't conflate with semantic edge provenance).
- Standardize: `role_provenance_source`, `role_provenance_confidence`, `role_reason`.
- Future-proof rule: **never remove keys**, only add.

**4) Schema versioning** (prevent silent breakage)
- Add `meta` table with `schema_version`, `run_id`, `workspace_root`, `language`.
- Add `metadata.schema_version` to stdout JSON.
- Update `ge-template` to refuse unknown schema versions unless `--allow-unknown-schema`.

**5) Template DSL alignment**
- Define "Template Contract v1" for parsing DB: supported fields, operators, filter subset.
- Add template validation tests so unsupported predicates are rejected (not silently ignored).

**6) Performance**
- Add indexes for hot filters (`role`, `is_vendor`, etc.) if JSON `json_extract` becomes slow.
- Classification time O(n) in file count. Root candidate generation bounded and capped.

---

## Risk Register

- **Silent schema mismatch** → wrong query results without errors. Mitigate with schema versioning + gating.
- **Contract drift across consumers** → contradictory roots/filters. Mitigate with stable keys + docs + tests.
- **Policy creep into engine** → GraphEngine starts deciding UI collapse/selection. Mitigate with advisory metadata.
- **Performance regression** from JSON properties filtering. Mitigate with indexing strategy and budgets.

---

## Definition of Done

- One canonical external contract (schema + payload) with version marker.
- Classification + view roots deterministic, explainable, and test-covered.
- Query/templates behave predictably (filters validated, not ignored).
- All consumers use recommendations as advisory, not as hard policy.
