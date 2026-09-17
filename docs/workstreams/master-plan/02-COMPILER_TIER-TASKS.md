# 02 — COMPILER TIER: single-session task breakdown

Status: v2 (2026-07-02) — review pass split T7 into T7a/T7b, hardened
T2's struct-literal list, made T5 hard-blocked by T2, and pinned the
disclosure tier-mapping decision. Depends on plan 01 (COMPLETE 2026-07-02).
Each task is one Composer session (~one PR, <= ~6 files). Follow
`EXECUTION_PROTOCOL.md` (`cargo +1.91`, additive serde, tests-are-done,
ports-before-adapters). **Read `evidence/02-notes.md` first** — it
records two plan defects this breakdown corrects inline (the
`compiler_edges` metadata chain, and the `SkipReason::IndexerFailed`
shape/ownership).

## Orientation (plan in 5 lines)

1. Batch compiler indexes become the PRIMARY semantic engine; subprocess
   LSP is demoted to no-indexer languages (Apex) + an opt-in benchmark
   witness, never a fallback link for batch-indexed languages.
2. **Phase A** adds a first-class `Compiler` provenance source + a
   centralized authority ladder (`Compiler>=Lsp>Heuristic>TreeSitter`)
   and relabels the existing in-process rust-analyzer edges (today
   mislabeled `Lsp`) as `Compiler`.
3. **Phase B** records agreement between resolvers as additive
   corroboration on `Provenance` (a `Copy` `SourceSet` bitset), never
   emitting duplicate sibling edges.
4. **Phase C** builds ONE universal SCIP ingester (new
   `graphengine-scip-adapter` crate) serving N per-language indexers,
   wires TypeScript first (then Python) through it with `Compiler`
   provenance and a machine-readable `IndexerFailed` skip reason, and
   cross-checks Rust via `rust-analyzer scip`.
5. **Exit (G1 contribution):** Rust self-scan edges are `Compiler` with
   `compiler_edges>0`; corroboration round-trips storage; a TS SCIP
   fixture produces `Compiler` edges with zero subprocess LSP; a failed
   indexer surfaces `IndexerFailed`; the Rust engine decision is recorded
   with measured numbers.

## Verified-state notes (drift vs. the plan, confirmed 2026-07-02)

- `graphengine-parsing/src/domain/provenance.rs`: `ProvenanceSource =
  TreeSitter | Lsp | Heuristic` (lines 8-15); `Provenance` and
  `ProvenanceSource` are `Copy` (lines 7, 29); `validate()` is the ONLY
  exhaustive match on `source` (lines 58-72) — confirmed by grep, all
  other sites are `Provenance::new(...)` constructors or `== ` compares.
- `rust_layer2.rs` line numbers drifted (file is now 1518 lines): the
  `Lsp` stamp is at **line 440** (`Provenance::new(ProvenanceSource::Lsp,
  confidence)`), NOT ~283; the `out.stats.lsp_edges += 1` bucket is at
  **line 526**, NOT ~313; the "provenance: Lsp / High|Medium" module doc
  is at **line 8** (and the heuristic-bucket note at lines 28-31).
- `ResolutionStatsSummary` (`ports.rs` lines 880-896) has `lsp_edges`,
  `heuristic_edges`, ...; `total_call_edges()` (lines 903-905) =
  `lsp_edges + heuristic_edges`. No `compiler_edges` yet.
- Storage: provenance persists as its own serde-JSON column in `edges`
  (`sqlite_repository.rs` lines 507-516) and `nodes` (line 497) →
  new enum variants + serde-default fields are migration-free. Confirmed.
- Merge guard: `fallback.rs::create_call_edges` skips
  `resolved_call_sites`-contained sites at **lines 70-77**. Confirmed.
- Analysis reads only `confidence`, never `source`
  (`graphengine-analysis/src/health/graph/types.rs` `Confidence::parse`
  lines 357-364). Adding a `Compiler` source does not perturb analysis.
- `SkipReason` (`resolution_disclosure.rs` lines 19-27) is
  `#[derive(Copy)] #[serde(rename_all="snake_case")]` with FIVE unit
  variants; `ResolutionDisclosure::layer2_active(lang, edges)` already
  exists (lines 48-56) and `rust_layer2` already calls it (line 644).
- Factory routing (`factory.rs` lines 153-183): apex → dispatcher; rust →
  `build_rust_semantic_resolver` (lines 227-261, feature `rust-layer2`);
  **every other language (incl. TypeScript) → generic `LspResolver`**
  (lines 176-183). TS has no batch-index path today.
- To-be-created (do NOT cite as existing): `domain/authority.rs`,
  `domain/fusion.rs`, `infrastructure/semantic/symbol_mapping.rs`,
  `infrastructure/semantic/index_backed.rs`, a `SemanticIndex` port,
  the `graphengine-scip-adapter` crate, and the `--semantic-engine`
  escape hatch (mentioned in the plan's routing policy point 4 but
  ABSENT from code and NOT a binding acceptance criterion).

## Task dependency graph

```
T1 (Compiler variant + authority ladder)
  ├─► T2 (rust_layer2 stamps Compiler + compiler_edges in summary)
  │      ├─► T3 (compiler_edges metadata → analysis; fix ResolutionDegraded)
  │      └─► T5 (SemanticIndex port + extract shared symbol_mapping.rs)
  └─► T4 (Phase B: corroboration field + fusion.rs + fallback corroboration)

T6  (new graphengine-scip-adapter crate)          — standalone
T7a (IndexerFailed disclosure vocabulary e2e)      — standalone

T5 + T6 + T7a ─► T7b (IndexBackedSemanticResolver + TS routing + fixture)
                    ├─► T8 (Python provisioning arm)
                    └─► T9 (Rust unification cross-check + decision)

T10 (plan-complete verify + evidence/02-results.md) ← blocked by T1..T9
```

Parallelism: **T4** only needs T1 and can run alongside the T2→T3 chain
(T1 lands the enum; T4 adds the `corroborating` field after T1 merges).
**T5 is HARD-blocked by T2** — both edit `rust_layer2.rs` heavily and
parallel edits guarantee merge conflicts and line-number drift. **T6**
and **T7a** are standalone and can start immediately; **T7b** needs all
of T5, T6, T7a. **Rule: make NO fidelity/scan-quality claims between T2
and T3 landing** — the intermediate state (compiler edges counted but
analysis unaware) misfires `ResolutionDegraded` (DEF-1).

---

## T1 — Add `Compiler` provenance + centralized authority ladder

- **Blocked by:** nothing. **Advances:** Phase A steps 1-2.
  **Acceptance:** feeds criterion 1 (Compiler source exists) + criterion
  8 (workspace green).
- **Files (3):**
  `graphengine-parsing/src/domain/provenance.rs`;
  new `graphengine-parsing/src/domain/authority.rs`;
  `graphengine-parsing/src/domain/mod.rs` (register `pub mod authority;`).

### Changes

1. `provenance.rs`:
   - Add variant to `ProvenanceSource` (keep it `Copy`):
     ```rust
     /// Whole-program semantic analysis via an in-process
     /// compiler/analyzer library or a batch compiler-produced index
     /// (rust-analyzer as a library, SCIP). No subprocess LSP.
     Compiler,
     ```
   - Add constructor:
     ```rust
     pub fn compiler() -> Self { Self::new(ProvenanceSource::Compiler, Confidence::High) }
     ```
   - Extend `validate()` (lines 58-72) — this is the ONLY exhaustive
     match on `source`, so it MUST gain arms or it fails to compile:
     `(Compiler, Confidence::Low) => Err(...)`,
     `(Compiler, Medium | High) => Ok(())`.
2. New `authority.rs` (pure, no I/O):
   ```rust
   pub fn authority_rank(s: ProvenanceSource) -> u8 { /* Compiler=4, Lsp=3, Heuristic=2, TreeSitter=1 */ }
   pub fn default_confidence(s: ProvenanceSource) -> Confidence { /* Compiler/Lsp=High, Heuristic/TreeSitter=Low */ }
   #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
   pub struct SourceSet(u8);      // Copy bitset over the closed 4-variant enum
   impl SourceSet { pub fn insert(&mut self, s: ProvenanceSource); pub fn contains(&self, s: ProvenanceSource) -> bool; pub fn count(&self) -> u32; pub fn iter(&self) -> impl Iterator<Item=ProvenanceSource>; }
   ```

### Gotchas

- `SourceSet` MUST stay `Copy` (Phase B puts it inside the `Copy`
  `Provenance`). A `u8` bitset satisfies this; do not use `Vec`/`HashSet`.
- Bit layout must be stable if it is ever serialized — but in Phase B it
  is serialized via serde on `Provenance`, so pick an explicit
  `#[repr]`-free bit per variant and unit-test the mapping so a future
  enum reorder can't silently corrupt persisted corroboration.
- Do NOT wire `authority_rank`/`fusion` into resolution yet (that's T4);
  this task only introduces the vocabulary + tests.

### Verify

- `cargo +1.91 test -p graphengine-parsing` green; new unit tests in
  `authority.rs` assert `authority_rank(Compiler) > authority_rank(Lsp)
  > authority_rank(Heuristic) > authority_rank(TreeSitter)`, and a
  `SourceSet` insert/contains/count/iter round-trip across all 4
  variants. `provenance.rs` test asserts `Provenance::compiler().validate()`
  is `Ok` and `(Compiler, Low)` is `Err`. `cargo +1.91 clippy -p
  graphengine-parsing` no new warnings.

---

## T2 — Stamp `Compiler` in rust_layer2 and route to `compiler_edges`

- **Blocked by:** T1. **Advances:** Phase A steps 3-4 (parser side).
  **Acceptance:** criterion 1 (edges carry `Compiler`; `compiler_edges`
  counted).
- **Files (7 — four are one-line struct-literal fixes):**
  `graphengine-parsing/src/infrastructure/semantic/rust_layer2.rs`;
  `graphengine-parsing/src/application/ports.rs`;
  `graphengine-parsing/src/infrastructure/lsp/stats/collector.rs` (line 56);
  `graphengine-parsing/src/syntax/language/apex/resolver_dispatch.rs` (line 357);
  `graphengine-parsing/src/syntax/language/apex/heuristic_resolver.rs` (line 435);
  `graphengine-parsing/src/infrastructure/lsp/telemetry_export.rs` (line 228, test);
  `graphengine-parsing/tests/rust_layer2_integration.rs` (assertions).

### Changes

1. `ports.rs` `ResolutionStatsSummary` (lines 880-896): add
   `pub compiler_edges: usize,` and include it in `total_call_edges()`
   (line 904): `self.compiler_edges + self.lsp_edges + self.heuristic_edges`.
   The struct derives `Default`, so serde/back-compat is a non-issue here.
2. `rust_layer2.rs`:
   - **Line 440** (the edge emit site): change
     `Provenance::new(ProvenanceSource::Lsp, confidence)` →
     `Provenance::new(ProvenanceSource::Compiler, confidence)`.
   - **Line 526**: change `out.stats.lsp_edges += 1;` →
     `out.stats.compiler_edges += 1;`.
   - Update the module-header doc (line 8, "provenance: Lsp / High|Medium")
     and the measured-fallback note (lines 28-31, which claims the counter
     surfaces in `heuristic_*`) to say `Compiler` / `compiler_edges`.
3. **DEF-3 (verified by grep `ResolutionStatsSummary \{`):** FOUR sites
   construct `ResolutionStatsSummary` with a full struct literal, and
   adding a field breaks ALL of them, not just collector.rs:
   - `infrastructure/lsp/stats/collector.rs` line 56 (`into_summary`),
   - `syntax/language/apex/resolver_dispatch.rs` line 357,
   - `syntax/language/apex/heuristic_resolver.rs` line 435,
   - `infrastructure/lsp/telemetry_export.rs` line 228 (`sample_stats`, test).
   Add `compiler_edges: 0,` at each (none of these paths produce
   compiler edges). Do not switch them to `..Default::default()` — the
   explicit-field style is deliberate there (a new field then forces a
   review at each site, which is exactly what caught this).
4. `tests/rust_layer2_integration.rs`: line 118 asserts
   `edge.provenance.source == ProvenanceSource::Lsp` → change to
   `Compiler`; line 130 asserts `edges.stats.lsp_edges == 1` → change to
   `edges.stats.compiler_edges == 1`. Update the assert messages.

### Gotchas

- Do NOT also increment `lsp_edges` — criterion 1 requires
  `compiler_edges > 0`, and double-counting would corrupt
  `total_call_edges()`.
- rust_layer2's `resolution_disclosure()` already returns
  `layer2_active("rust", snap.edges_emitted)` (line 644) — leave it; the
  tier disclosure is already correct. This task is only about the numeric
  edge accounting.
- **Do not** touch the analysis-side accounting here; that is T3 (kept
  separate because it crosses the crate boundary and is the drift-prone
  part). After T2 alone, a Rust scan will show `compiler_edges` in the
  in-memory summary but the analysis `ResolutionDegraded` finding would
  MIS-fire — T3 closes that gap and must land before any G1 measurement.

### Verify

- `cargo +1.91 test -p graphengine-parsing` green (updated integration
  test passes with `Compiler` + `compiler_edges`). Build the parser and
  run the direct path from plan-01 T0
  (`./target/release/graphengine-parsing --verbose parse --root . --lang
  rust --db /tmp/t2.sqlite --lsp-policy fast --no-incremental`); the
  `rust Layer-2 resolve done:` line shows non-zero `edges=`, and
  `sqlite3 /tmp/t2.sqlite "select provenance from edges limit 20;"` shows
  `"Compiler"` in the JSON. `cargo +1.91 clippy -p graphengine-parsing`
  clean.

---

## T3 — Propagate `compiler_edges` to analysis; keep `ResolutionDegraded` honest

- **Blocked by:** T2. **Advances:** Phase A step 4 (analysis side).
  **Acceptance:** criterion 1 end-to-end through the MCP/scan report;
  prevents a false `ResolutionDegraded` regression.
- **Files (3):**
  `graphengine-parsing/src/application/use_cases/parse_repo/pipeline/orchestrator.rs`;
  `graphengine-analysis/src/health/pipeline/segments/findings_assembly.rs`;
  `graphengine-analysis/src/health/resolution_degraded.rs`.

### Why this task exists (plan defect DEF-1, see evidence/02-notes.md)

Plan Phase A.4 says "treat `compiler_edges` as authoritative alongside
`lsp_edges`" but does not name the *metadata-key* mechanism that carries
edge counts from parser to analyzer. Today `orchestrator.rs` writes only
`resolution_lsp_edges`/`resolution_heuristic_edges` (lines 507-512) and
`findings_assembly.rs` reads only those (lines 123-124) into the
analysis-local `ResolutionStatsSnapshot`. After T2 moves Rust edges into
`compiler_edges`, `resolution_lsp_edges` becomes ~0 for Rust, so
`ResolutionDegraded.fallback_rate()` (resolution_degraded.rs lines
102-111) would compute a spuriously high fallback rate and fire a
"degraded call graph" finding on the HIGHEST-fidelity scans — the exact
opposite of calibrated honesty. This task closes that.

### Changes

1. `orchestrator.rs` (next to lines 507-512): add
   `graph.metadata.insert("resolution_compiler_edges".into(),
   stats.compiler_edges.to_string());`. Also review the disclosure
   fallback at lines 1069-1084: `else if stats.lsp_edges > 0` should also
   consider `stats.compiler_edges` — though for Rust the resolver's own
   `resolution_disclosure()` short-circuits at line 1069, so this is
   defensive; note it, don't over-engineer.
2. `resolution_degraded.rs` `ResolutionStatsSnapshot` (lines 64-75): add
   `pub compiler_edges: usize,`. Treat it as authoritative (non-fallback)
   exactly like `lsp_edges`: include it in `total_resolution_work()`
   (lines 90-94) and NOT in the fallback numerator (lines 107-110).
   Update the doc-comment fallback-rate formula (lines 28-30) and add/
   extend a unit test proving a pure-`compiler_edges` scan reports
   fallback_rate ~0 and fires no finding.
3. `findings_assembly.rs` (lines 122-129): add
   `compiler_edges: parse_u("resolution_compiler_edges"),` to the
   `ResolutionStatsSnapshot` construction.

### Gotchas

- Additive-only: missing `resolution_compiler_edges` key (old DBs) must
  default to 0 — `parse_u` already returns 0 on miss (line 120).
- `graphengine-analysis` must not gain a new dependency edge on
  `graphengine-parsing`'s application layer; this task only reads a
  scalar metadata string, so no cross-crate type import is needed
  (mirrors how `resolution_lsp_edges` is already handled).

### Verify

- `cargo +1.91 test -p graphengine-analysis` green (new test:
  `pure_compiler_edges_is_not_degraded`). Full scan
  `./target/release/gridseak scan .`; read the report JSON and confirm
  `resolution_quality.measured_fidelity.call_edges_by_confidence.high`
  reflects the Rust compiler edges AND no spurious `ResolutionDegraded`
  finding appears for Rust. `cargo +1.91 test --workspace` green.

---

## T4 — Phase B: witness fusion (corroboration, additive, no duplicate edges)

- **Blocked by:** T1 (needs `SourceSet` + `authority`). **Advances:**
  Phase B steps 1-4. **Acceptance:** criteria 2 and 3.
- **Files (5):**
  `graphengine-parsing/src/domain/provenance.rs`;
  new `graphengine-parsing/src/domain/fusion.rs`;
  `graphengine-parsing/src/domain/mod.rs` (register `pub mod fusion;`);
  `graphengine-parsing/src/application/use_cases/parse_repo/resolution/fallback.rs`;
  `graphengine-parsing/tests/storage_tests.rs` (serde/DB round-trip).

### Changes

1. `provenance.rs`: add
   `#[serde(default)] pub corroborating: crate::domain::authority::SourceSet`
   to `Provenance`. It stays `Copy` (SourceSet is `Copy`). `#[serde(default)]`
   means old DB rows deserialize to an empty set — this is the
   back-compat criterion. Update `Provenance::new`/`compiler`/`lsp`/
   `heuristic`/`tree_sitter` constructors to init it empty.
2. New `fusion.rs`:
   ```rust
   pub fn adjudicate(witnesses: &[ProvenanceSource]) -> Provenance
   // winner = max authority_rank; base = default_confidence(winner);
   // promote Medium->High iff >= 2 DISTINCT sources agree;
   // never downgrade the winner; non-winners recorded in `corroborating`.
   ```
   Pure and unit-tested (no I/O).
3. `fallback.rs::create_call_edges` at the skip guard (lines 70-77):
   when a reference's `call_site.location` IS in `resolved_call_sites`,
   compute the heuristic's would-be UNIQUE target; if it matches the
   existing winning edge's `to_id`, stamp corroboration onto that edge
   (build a `location -> &mut Edge` map once over `resolved_edges.call_edges`).
   NEVER emit a sibling edge; a disagreeing heuristic match adds nothing.
   Do not change the current `continue` behavior for the no-match case.

### Gotchas

- `Provenance` is `Copy`; `SourceSet` must remain `Copy` or `Provenance`
  loses `Copy` and dozens of call sites break. Verify with a `cargo
  check` early.
- `Provenance` derives `PartialEq`, so adding `corroborating` CHANGES
  equality semantics: two provenances that used to compare equal now
  differ once corroboration is stamped. Grep-verified there are no
  struct-literal `Provenance {` constructions outside the domain module
  (all sites use constructors), but any test that compares whole
  `Provenance` values on edges the fallback pass touches may start
  failing — fix by asserting `source`/`confidence` fields where the
  corroboration set is not the thing under test.
- The corroboration map must key on the SAME `Range` identity used by
  `resolved_call_sites` (it is a `HashSet<Range>`); `Range` already
  derives `Hash`/`Eq` (used as the set key today).
- serde back-compat: the round-trip test in `storage_tests.rs` MUST load
  a JSON `provenance` string WITHOUT the `corroborating` field and assert
  it deserializes to an empty set (paste a literal old-format JSON, do
  not regenerate it).
- Do not alter authority resolution order in the primary resolvers here;
  Phase B is strictly additive corroboration on already-won edges.

### Verify

- `cargo +1.91 test -p graphengine-parsing` green with new tests:
  (a) `fusion::adjudicate` promotes Medium→High on 2 distinct agreeing
  sources and records non-winners; (b) agreement stamps corroboration +
  produces NO sibling edge; (c) disagreement stamps nothing + no sibling;
  (d) old-format `Provenance` JSON parses (empty corroboration).
  `cargo +1.91 clippy -p graphengine-parsing` clean.

---

## T5 — Phase C: `SemanticIndex` port + extract shared `symbol_mapping.rs`

- **Blocked by:** T2 (HARD — T2 and T5 both edit `rust_layer2.rs`
  heavily; running them in parallel guarantees merge conflicts, and the
  extracted SymbolIndex must already stamp `Compiler`).
  **Advances:** Phase C step 1 + step 3's "reuse, do not copy-paste".
  **Acceptance:** enabling scaffold for criteria 4 and 6 (no behavior
  change of its own).
- **Files (4):**
  `graphengine-parsing/src/application/ports.rs` (or a new
  `graphengine-parsing/src/application/ports/semantic_index.rs` module —
  prefer a dedicated file per the DRY/modular rule);
  new `graphengine-parsing/src/infrastructure/semantic/symbol_mapping.rs`;
  `graphengine-parsing/src/infrastructure/semantic/rust_layer2.rs`
  (delete its private `SymbolIndex` + helpers, import the shared one);
  `graphengine-parsing/src/infrastructure/semantic/mod.rs`
  (`pub mod symbol_mapping;`).

### Changes

1. Define the port:
   ```rust
   pub trait SemanticIndex {
       fn definition_at(&self, file: &Path, line: u32, col: u32) -> Option<IndexTarget>;
       fn language(&self) -> &str;
   }
   pub struct IndexTarget { pub file: PathBuf, pub line: u32, pub col: u32, pub symbol_moniker: String, pub confidence: Confidence }
   ```
2. Extract `rust_layer2`'s `SymbolIndex` (rust_layer2.rs lines 740-908)
   and its free helpers (`contains`, `line_encloses`, `path_matches`,
   `fqn_ends_with`, `range_span`, etc., lines 910-1212) into
   `symbol_mapping.rs` as a reusable caller/callee mapper. The current
   `find_callee_for` is typed against `ResolvedTarget` (from
   `graphengine-ra-ide-adapter`); generalize it to accept the
   language-neutral `IndexTarget` (file/line/name) so both rust_layer2
   and the coming `index_backed.rs` share ONE implementation.

### Gotchas

- This is a **pure refactor**: rust_layer2's observable output must not
  change. The existing `rust_layer2_integration.rs`, `symbol_index_tests`,
  `fqn_tests`, `external_target_tests`, and `caret_tests` (rust_layer2.rs
  lines 1214-1500) are the safety net — move/keep them green, do not
  weaken assertions.
- Keep `ResolvedTarget → IndexTarget` adaptation thin (a `From`/mapping
  fn) so rust_layer2 keeps its ra-adapter dependency while the shared
  mapper stays adapter-agnostic.
- Do not move the ra-ide-specific `caret_for_callee` logic into the
  shared module — that is rust-analyzer-query-shape-specific and belongs
  with rust_layer2.

### Verify

- `cargo +1.91 test -p graphengine-parsing` green with ALL previously
  passing rust_layer2 tests unchanged. `cargo +1.91 clippy -p
  graphengine-parsing` no new warnings. Diff shows SymbolIndex logic
  MOVED, not duplicated (grep `struct SymbolIndex` returns one hit in
  `symbol_mapping.rs`).

---

## T6 — Phase C: new `graphengine-scip-adapter` crate (SCIP parse + provisioning)

- **Blocked by:** nothing hard (can parallel T5); consumed by T7b.
  **Advances:** Phase C step 2. **Acceptance:** enabling for criteria 4
  and 5.
- **⚠ Tooling flag:** vendoring `scip.proto` requires fetching the pinned
  schema from `github.com/sourcegraph/scip`; generating test fixtures
  needs `npx @sourcegraph/scip-typescript` (one-time). Crate compilation
  and `.scip` parsing need NEITHER network nor Node. If the Composer
  environment blocks network for the proto fetch, flag to OWNER and
  commit the vendored `scip.proto` in this PR so downstream is hermetic.
- **Files (~5, all new + 2 workspace edits):**
  `graphengine-scip-adapter/Cargo.toml`;
  `graphengine-scip-adapter/build.rs` (prost codegen);
  `graphengine-scip-adapter/proto/scip.proto` (vendored, version-pinned);
  `graphengine-scip-adapter/src/lib.rs` (occurrence table + queries);
  `graphengine-scip-adapter/src/provisioning.rs` (run indexer);
  plus `Cargo.toml` workspace `members` + `default-members` (lines 3-39).

### Changes

1. Deps — **check the official crate FIRST**: Sourcegraph publishes a
   `scip` Rust crate (protobuf bindings for the SCIP schema) on
   crates.io. If it exists at a maintained version, prefer it over
   hand-vendoring `scip.proto` + `prost-build` codegen — fewer moving
   parts, no build.rs, no proto fetch. Pin the exact version and record
   the choice (crate vs. vendored proto) in `evidence/02-notes.md`.
   Only if the crate is unsuitable: vendor the proto + `prost`/
   `prost-build`, pinning the schema version in a comment next to it.
2. `lib.rs`: read an `index.scip`, build an in-memory occurrence table
   (`file -> sorted occurrences with symbol + SymbolRole`), resolve
   definition-at-position offline. This crate does NOT depend on
   `graphengine-parsing` — it returns its own plain types.
3. `provisioning.rs`: given a repo root, detect `package.json`/
   `tsconfig.json`, run `npx --yes @sourcegraph/scip-typescript index`
   with a bounded timeout and captured stderr; return
   `Result<PathBuf /*index.scip*/, IndexerError>` where `IndexerError`
   carries the process exit code. **Do NOT** reference
   `SkipReason::IndexerFailed` here (see architecture note DEF-2 in
   evidence/02-notes.md) — mapping to the disclosure enum happens in the
   parsing layer (T7a/T7b), keeping this crate free of a dependency on
   `graphengine-parsing`.

### Gotchas

- Add the crate to BOTH `[workspace].members` and `default-members`
  (Cargo.toml lines 3-17 and 26-39) or `cargo +1.91 test --workspace`
  won't include it.
- The plan text says `github.com/scip-code/scip`; the maintained schema
  lives under `github.com/sourcegraph/scip`. Use the real source and
  record the exact commit/tag pinned.
- Occurrence positions in SCIP are 0-based `[startLine, startChar,
  endLine, endChar]` (and may be 3-element for single-line) — handle the
  packed-range encoding explicitly and unit-test it against a tiny
  hand-written `index.scip`.

### Verify

- `cargo +1.91 test -p graphengine-scip-adapter` green: a unit test
  parses a small committed `.scip` fixture and resolves a known
  definition-at-position. `cargo +1.91 build --workspace` compiles the
  new crate. Provisioning has a test that asserts a broken/missing
  `package.json` yields `IndexerError` with a non-zero exit code (no
  network needed for the failure path).

---

## T7a — Add `IndexerFailed` to the disclosure vocabulary, end-to-end

- **Blocked by:** nothing (standalone; MUST merge before T7b).
  **Advances:** Phase C step 2's disclosure contract. **Acceptance:**
  enabling half of criterion 5.
- **Why a separate task (review finding, extends DEF-2):** `SkipReason`
  is not one enum — it has a serde **mirror** on the analysis side
  (`ResolutionSkipReason` in `graphengine-analysis/src/health/report.rs`,
  imported by the CLI renderer) and an exhaustive match in
  `gridseak-cli/src/render/resolution_disclosure.rs::skip_reason_label`
  (lines 41-49). Adding the variant only in parsing would make the
  analysis mirror FAIL to deserialize any disclosure row carrying
  `"indexer_failed"` — a silent disclosure drop, the exact failure class
  this plan exists to kill. Bundling this into the resolver task would
  also blow the 6-file budget.
- **Files (3 + tests):**
  `graphengine-parsing/src/application/resolution_disclosure.rs`;
  `graphengine-analysis/src/health/report.rs` (mirror enum
  `ResolutionSkipReason`);
  `gridseak-cli/src/render/resolution_disclosure.rs` (`skip_reason_label`
  arm → `"indexer_failed"`, extend the renderer test at lines 55-80).

### Changes

1. `resolution_disclosure.rs`: add a **unit** variant `IndexerFailed` to
   `SkipReason` (DEF-2 shape decision: unit variant keeps the enum `Copy`
   and every existing bare-string snake_case wire value byte-stable; the
   indexer's exit code travels in logs/telemetry, NOT in the enum).
   Extend the serde round-trip test to pin `"indexer_failed"`.
2. Same file: add the missing constructor for the batch-index chain —
   no existing constructor expresses "attempted the index, fell back to
   heuristic" (`heuristic_only` hardcodes `tier_attempted: SubprocessLsp`,
   which would be a false disclosure for TS):
   ```rust
   pub fn layer2_fallback_to_heuristic(language: &str, reason: SkipReason) -> Self {
       // tier_attempted: Layer2, tier_used: Heuristic, skip_reason: Some(reason)
   }
   ```
3. `report.rs`: add `IndexerFailed` to the mirror `ResolutionSkipReason`
   (same serde snake_case), plus a deserialization test proving a JSON
   row with `"skip_reason":"indexer_failed"` parses.
4. CLI renderer: add the `IndexerFailed` arm; grep for any other
   exhaustive `match` on either enum and update it.

### Verify

- `cargo +1.91 test -p graphengine-parsing -p graphengine-analysis -p
  gridseak-cli` green, including the new round-trip/deserialize/renderer
  tests. `cargo +1.91 test --workspace` green.

---

## T7b — Phase C: `IndexBackedSemanticResolver` + TypeScript factory routing

- **Blocked by:** T5 (port + shared mapper), T6 (the crate), T7a (the
  vocabulary). **Advances:** Phase C steps 3-4. **Acceptance:** criteria
  4, 5, and 6.
- **Files (~6):**
  new `graphengine-parsing/src/infrastructure/semantic/index_backed.rs`;
  `graphengine-parsing/src/infrastructure/semantic/mod.rs`;
  `graphengine-parsing/src/application/use_cases/parse_repo/factory.rs`
  (TS routing);
  `graphengine-parsing/Cargo.toml` (dep on `graphengine-scip-adapter`,
  behind a `scip` feature mirroring `rust-layer2`);
  new fixture + test under `graphengine-parsing/tests/`
  (committed tiny TS repo + prebuilt `index.scip`).

### Changes

1. `index_backed.rs`: `IndexBackedSemanticResolver<I: SemanticIndex>`
   implementing the `SemanticResolver` port, using the shared
   `symbol_mapping` (T5) for caller/callee lookup. Edges stamped
   `Provenance::compiler()` (source `Compiler`). Mirror rust_layer2's
   `resolved_call_sites` marking + `compiler_edges` accounting so the
   heuristic guard (T2/T4) works identically.
2. Disclosure wiring (vocabulary from T7a). **Tier-mapping decision
   (pinned here so the executor does not invent an enum):** the
   index-backed ACTIVE path reuses `ResolutionTierKind::Layer2` via
   `ResolutionDisclosure::layer2_active(language, edges)` — it is
   in-process semantic resolution with no subprocess, which is exactly
   this codebase's `Layer2` meaning; a dedicated `BatchIndex`/`Compiler`
   tier NAME is part of the tier-legend vocabulary release deferred to
   plan 04 (a plan-02 non-goal). The failure path uses the T7a
   constructor `layer2_fallback_to_heuristic(language,
   SkipReason::IndexerFailed)`.
3. `factory.rs`: route `language == "typescript"` (verify the exact
   language identifier the runner passes — baseline lists
   `typescript`) to build the index-backed resolver: attempt
   provisioning → on success wrap `IndexBackedSemanticResolver`; on
   failure return a `DisclosedSemanticResolver` over the pure-heuristic
   path with the T7a fallback disclosure. **Per routing policy, the TS
   chain MUST NOT contain a subprocess-LSP link** (criterion 6) — do not
   reuse the generic `LspResolver` branch (factory.rs lines 176-183)
   for TS.

### Gotchas

- The committed `index.scip` fixture makes the primary test hermetic (no
  Node in CI). Generating it once needs `scip-typescript` — flag as a
  one-time tooling step; the resulting artifact is checked in.
- Provisioning must NEVER silently degrade (plan-01 disclosure contract):
  a failed indexer MUST surface `IndexerFailed`, verified by a
  broken-fixture test.
- Feature-gate the scip dep like `rust-layer2` so
  `--no-default-features` still builds the old TS path; decide and
  document whether `scip` is default-on (open question for plan owner in
  evidence/02-notes.md).

### Verify

- `cargo +1.91 test -p graphengine-parsing` green with:
  (a) TS fixture scan asserts exact expected call+import edges carrying
  `ProvenanceSource::Compiler` and asserts ZERO subprocess-LSP
  involvement; (b) a chain-composition test asserts the TS factory chain
  contains no `LspResolver` (criterion 6); (c) a broken-fixture test
  asserts `SkipReason::IndexerFailed` surfaces in the disclosure. An
  optional `#[ignore]`/feature-gated E2E test builds the index live when
  Node is present. `cargo +1.91 test --workspace` green.

---

## T8 — Phase C: Python provisioning arm (`scip-python`)

- **Blocked by:** T7b (reuses the whole ingestion path). **Advances:**
  Phase C step 5. **Acceptance:** contributes to criterion 4/8 for a
  second language (no NEW binding criterion — TS is the gate; Python is
  "ship after TS proves").
- **Files (~3):**
  `graphengine-scip-adapter/src/provisioning.rs` (new arm: detect
  `pyproject.toml`/`setup.py`/`requirements.txt`, run `npx --yes
  @sourcegraph/scip-python index`);
  `graphengine-parsing/src/application/use_cases/parse_repo/factory.rs`
  (`language == "python"` routing, same shape as TS);
  new Python fixture + prebuilt `index.scip` test under
  `graphengine-parsing/tests/`.

### Gotchas

- Same "no subprocess-LSP link in the chain" rule as TS.
- scip-python provisioning realistically needs a resolvable Python env;
  the committed-fixture test path must stay hermetic (parse a prebuilt
  `.scip`); live provisioning is the gated/`#[ignore]` test.
- Ship ONLY after T7b is merged and TS is proven — do not develop TS and
  Python provisioning in the same PR (plan: "Ship TS first, prove, then
  Python").

### Verify

- `cargo +1.91 test -p graphengine-parsing` green: Python fixture scan
  yields `Compiler`-provenance edges; broken-fixture yields
  `IndexerFailed`. `cargo +1.91 test --workspace` green.

---

## T9 — Rust unification cross-check (`rust-analyzer scip` vs. in-process adapter)

- **Blocked by:** T6 (ingester) + T7b (index-backed resolver).
  **Advances:** Phase C "Rust unification". **Acceptance:** criterion 7
  (cross-check executed; engine decision recorded WITH measured numbers).
- **⚠ Tooling flag (NOT headless-safe):** requires a `rust-analyzer`
  binary supporting `rust-analyzer scip .` on this repo. This is a
  measurement + decision task, not primarily a code task. Assign to an
  agent/OWNER with the toolchain; Composer may prepare the diff harness
  but cannot produce the binding numbers without the binary.
- **Files (2):** a cross-check harness (a `#[ignore]` test or a small
  `src/bin/` tool under `graphengine-parsing/`) that feeds
  `rust-analyzer scip .` output through the T6 ingester and diffs edges
  against `rust_layer2`; and an APPEND to
  `docs/workstreams/master-plan/02-COMPILER_TIER.md` recording the
  decision (EXECUTION_PROTOCOL §7 permits appending measured results to
  the plan's own decision section for this explicitly plan-mandated
  cross-check — keep it to the "record why here" the plan requests at
  lines 198-199; if in doubt, record in `evidence/02-results.md` and
  reference it).

### The mandated question (plan lines 186-199)

Does batch `rust-analyzer scip` output include call sites inside
proc-macro-expanded bodies that the per-position adapter misses?
(UF-FU-012(b)/UF-FU-003 measured ~88.9% adapter miss on macro-heavy code;
this repo is serde-derive dense.) Decision rule: if SCIP-path quality >=
ra-ide-adapter quality AND total scan time within 1.5x, switch Rust to
the universal pipeline (keep `graphengine-ra-ide-adapter` for future
incremental/per-position use); else keep the in-process adapter and
record why. Either way provenance stays `Compiler`.

### Verify

- The harness runs and emits a deterministic edge diff (counts of
  agree/adapter-only/scip-only). The decision is recorded with the
  measured high-ratio and scan-time numbers cited to artifacts (no
  invented numbers). If the toolchain is unavailable, STOP and report
  that criterion 7 is blocked on `rust-analyzer scip` availability — do
  not fabricate the comparison.

---

## T10 — Plan-complete verification + `evidence/02-results.md`

- **Blocked by:** T1..T9. **Advances:** re-checks EVERY acceptance
  criterion in `02-COMPILER_TIER.md` (lines 219-233).
- **Files (1 + tick):** create
  `docs/workstreams/master-plan/evidence/02-results.md`; you MAY tick the
  acceptance checkboxes in `02-COMPILER_TIER.md` ONLY for criteria you
  proved (EXECUTION_PROTOCOL §7).
- **⚠ Flag — not fully Composer-headless:** criterion 1 ("through the
  standard MCP scan path") requires a live MCP agent session
  (`gridseak_scan`), and criterion 7 requires the `rust-analyzer scip`
  toolchain (T9). Composer can prove the CLI path and all unit/integration
  tests; the MCP re-verification and the rust-scip cross-check must be run
  by an agent/OWNER. Record both; never claim the MCP/tooling half from
  CLI output alone.

### Checklist to record (before → after, cite artifacts)

1. **Compiler edges** (T2/T3): full scan; report JSON
   `resolution_quality.measured_fidelity.call_edges_by_confidence` and
   `compiler_edges > 0`; edges in DB carry `"Compiler"`. Baseline to
   beat: `evidence/2026-07-01-dogfood-baseline.md` (`syntactic_only`,
   `high_ratio=0.143`, high 3772 / med 97 / low 22440).
2. **Corroboration round-trips storage; old DBs load** (T4): paste the
   passing serde/back-compat test.
3. **Heuristic agreement corroborates, never duplicates** (T4): paste
   the agree/disagree tests.
4. **TS SCIP → Compiler edges, zero LSP** (T7b): paste the fixture test.
5. **Failed indexer → `IndexerFailed`** (T7a/T7b): paste the
   broken-fixture test.
6. **Batch-indexed chain has no subprocess LSP link** (T7b): paste the
   chain-composition test.
7. **Rust unification decision recorded** (T9): link the recorded numbers
   + decision.
8. **`cargo +1.91 test --workspace` green** (paste tail); clippy no new
   warnings on touched crates; `cargo +1.91 fmt` clean.

- **Verify (observable):** `evidence/02-results.md` cites every number to
  a command / scan-id / report path (no invented numbers,
  EXECUTION_PROTOCOL §3), and states honestly which criteria passed,
  which are agent/tooling-only, and any that could not be met.

---

## Cross-cutting reminders for every task

- **Scope:** do only your task's plan phase. Unrelated bugs → append to
  `evidence/02-notes.md` and move on (EXECUTION_PROTOCOL §1).
- **No silent failure:** paste real errors into evidence; never fake a
  verification (this is the plan's whole reason for existing).
- **Clean architecture:** ONE SCIP ingestion path (do not fork per
  language); ports before adapters; the shared `symbol_mapping.rs` must
  not be copy-pasted between rust_layer2 and index_backed; no file grows
  past ~500 lines when it can be split.
- **Additive serialization:** every new persisted field is
  `#[serde(default)]`; `Provenance`/`ProvenanceSource`/`SourceSet` stay
  `Copy`; the only new-variant sites are `ProvenanceSource::Compiler`
  (validate() exhaustive match) and `SkipReason::IndexerFailed` (all
  match sites incl. the CLI renderer).
- **OWNER / tooling-gated (never blind-assign to Composer):** T6 proto
  vendoring if network-restricted; T7b/T8 live index generation (Node);
  T9 `rust-analyzer scip` cross-check; T10 MCP-path re-verification.
- **The tier/provenance vocabulary is the product** (00-MASTER_PLAN §7):
  every new structural claim carries source + confidence; the MCP
  `tier_legend` wording update is explicitly deferred to plan 04 (plan 02
  non-goal), so do NOT change it here.
