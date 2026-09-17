# 02 — COMPILER TIER: batch semantic indexing as the PRIMARY engine

Status: DRAFT v2 (2026-07-01) — v2 decision: batch compiler indexes
REPLACE subprocess LSP as the default semantic engine for every language
that has a maintained batch indexer. LSP is demoted to (a) languages with
no batch indexer (Apex) and (b) an opt-in debugging/benchmark witness.
It is NOT a fallback link in the default chain.
Depends on: 01-TRUTH_RECOVERY (its diagnosis + disclosure contract).
Blocks: Gate G1 (with 01), plan 04 (benchmark winnability).
Executor profile: Rust agent; for Phase C also Node.js tooling access.

## Context

GridSeak stamps every edge with `Provenance { source, confidence }`
(`graphengine-parsing/src/domain/provenance.rs`). Today
`ProvenanceSource` = `TreeSitter | Lsp | Heuristic`. The highest-accuracy
resolution shape is "compiler as a library / batch semantic index":
in-process rust-analyzer already exists for Rust
(`graphengine-parsing/src/infrastructure/semantic/rust_layer2.rs` wrapping
`graphengine-ra-ide-adapter`) but mislabels its edges as `Lsp`
(rust_layer2.rs ~line 283) and buckets them into `stats.lsp_edges`
(~line 313). For TypeScript/Python, maintained SCIP indexers exist
(`@sourcegraph/scip-typescript`, `@sourcegraph/scip-python`, both alive
as of 2026-06) that emit a protobuf index of compiler-accurate
definitions/references/monikers.

Key architectural facts (verified 2026-06-20):
- `Provenance` persists as its own serde-JSON column in SQLite
  (`graphengine-parsing/src/infrastructure/storage/sqlite_repository.rs`
  lines 508–515) → new enum variants and serde-default fields are
  migration-free.
- The only exhaustive match on source is `Provenance::validate()`
  (provenance.rs ~line 60).
- Analysis reads only `confidence`, not `source`
  (`graphengine-analysis/src/health/graph/types.rs`, `Confidence::parse`).
- Merge policy is first-wins-with-a-guard: semantic resolvers record
  resolved call-site ranges in `ResolvedEdges::resolved_call_sites`; the
  heuristic (`.../parse_repo/resolution/fallback.rs` lines 61–77) skips
  those sites. No corroboration is recorded today.
- `Provenance` is `Copy` — any witness set must be a `Copy` bitset.

## Why batch indexing replaces LSP (rationale — the first-principles case)

LSP servers and batch indexers wrap the SAME compiler front-ends
(pyright ↔ scip-python; the TypeScript compiler ↔ scip-typescript;
rust-analyzer ↔ `rust-analyzer scip`). The precision ceiling is
identical. The difference is the ACCESS PATTERN:

- LSP is an interactive editor protocol: long-lived subprocess,
  per-position round-trips, readiness races, document sync, timeouts.
  Every measured failure in our telemetry (`returned_null`, timeouts,
  0 Apex LSP edges, silent skips) is a symptom of driving an editor
  protocol as a batch extractor.
- A batch index is an artifact: one indexer run (~cost of one full
  typecheck), one protobuf file, then in-memory lookups. Failure is an
  exit code — loud by construction, satisfying plan 01's disclosure
  contract. Artifacts cache per-commit SHA, which is what makes
  observatory-scale scanning (plan 05) economically possible.

GitHub's stack-graphs (hand-built universal resolution, no compilers)
was archived 2025-09; the surviving ecosystem is per-language compiler
indexers emitting SCIP. Verified coverage (Sourcegraph indexer table +
repo activity, checked 2026-07-01): Go `scip-go` (v0.2.7, 2026-05),
TS/JS `scip-typescript`, Python `scip-python`, Java/Scala/Kotlin
`scip-java`, C/C++ `scip-clang`, Ruby `scip-ruby`, C# `scip-dotnet`,
Rust natively via `rust-analyzer scip` (batch CLI, no language server
process). Every stable language GridSeak targets has a GA batch indexer.

Authority ordering: `Compiler >= Lsp > Heuristic > TreeSitter`.

### Routing policy (the v2 decision, binding)

Per language, the default chain is:
1. Batch semantic index (SCIP or in-process compiler library) — primary.
2. Heuristic fallback with machine-readable `SkipReason` — and nothing
   in between. Subprocess LSP is NOT a fallback for batch-indexed
   languages: when an indexer fails, the cause (project doesn't
   typecheck / deps not installed) would defeat the LSP server too, and
   the subprocess path re-introduces the silent failure modes we are
   eliminating.
3. Subprocess LSP remains the semantic engine ONLY for languages with no
   batch indexer (today: Apex/jorje). The `infrastructure/lsp` stack
   moves to maintenance mode: no new investment except what the Apex
   decision (00-MASTER_PLAN, plan 04 follow-ups) explicitly funds.
4. Opt-in escape hatch `--semantic-engine lsp` kept for debugging and as
   an independent witness in the benchmark (plan 04) — never the default.

## Phase A — Compiler provenance + centralized authority ladder (Rust truth)

1. `graphengine-parsing/src/domain/provenance.rs`:
   - Add `ProvenanceSource::Compiler` variant with doc: "Whole-program
     semantic analysis via an in-process compiler/analyzer library or a
     batch compiler-produced index (rust-analyzer as a library, SCIP).
     No subprocess LSP."
   - Add constructor `Provenance::compiler()` → `(Compiler, High)`.
   - Extend `validate()`: `(Compiler, Low)` is an error; Medium/High ok.
2. New file `graphengine-parsing/src/domain/authority.rs`:
   - `pub fn authority_rank(s: ProvenanceSource) -> u8`
     (Compiler=4, Lsp=3, Heuristic=2, TreeSitter=1).
   - `pub fn default_confidence(s: ProvenanceSource) -> Confidence`.
   - `pub struct SourceSet(u8)` — `Copy` bitset over the closed enum:
     `insert`, `contains`, `count`, `iter`. Unit-test the bit layout.
3. `rust_layer2.rs`: stamp `ProvenanceSource::Compiler` at the edge emit
   site; route counters to a new `stats.compiler_edges`; update the
   module-header docs that currently say "provenance: Lsp / High|Medium".
4. `graphengine-parsing/src/application/ports.rs`
   (`ResolutionStatsSummary`): add `pub compiler_edges: usize`, include
   in `total_call_edges()`. Audit every `lsp_edges` consumer
   (`graphengine-analysis/src/health/resolution_degraded.rs`,
   `.../lsp/stats/aggregator.rs`, `.../pipeline/orchestrator.rs`,
   `.../lsp/telemetry_export.rs`,
   `graphengine-analysis/src/health/pipeline/segments/findings_assembly.rs`)
   and treat compiler_edges as authoritative alongside lsp_edges.
5. Tests: validate() arms; serde round-trip; extend
   `rust_layer2_integration.rs` to assert `Compiler` source and
   `compiler_edges` counting; storage round-trip in `storage_tests.rs`.

## Phase B — Witness fusion (agreement raises confidence, additively)

1. `provenance.rs`: add `#[serde(default)] pub corroborating: SourceSet`
   to `Provenance` (stays `Copy`; old DB rows deserialize to empty).
2. New `graphengine-parsing/src/domain/fusion.rs`:
   `pub fn adjudicate(witnesses: &[ProvenanceSource]) -> Provenance` —
   winner = max authority_rank; base = default_confidence(winner);
   promote Medium→High iff >= 2 distinct sources agree; never downgrade
   the winner; non-winners recorded in `corroborating`. Pure, unit-tested.
3. `.../resolution/fallback.rs` `create_call_edges`: at the
   `resolved_call_sites` skip (lines ~70–77), compute the heuristic's
   would-be unique target; if it matches the existing winning edge's
   callee, stamp corroboration on that edge (build a
   `call_site.location -> &mut Edge` map once). Never emit a sibling
   edge; a disagreeing heuristic match adds nothing.
4. Tests: agreement stamps corroboration + no sibling; disagreement
   stamps nothing + no sibling; serde back-compat (old JSON without the
   field parses).

## Phase C — The universal SCIP ingester (TypeScript first, then Python)

This is where accuracy meets adoption: TS/Python are the languages most
users scan. Design for "add a language = add an adapter, zero core edits".
Architectural target: ONE SCIP ingestion pipeline serving N per-language
indexers — not N bespoke adapters. Because rust-analyzer emits SCIP
natively, even Rust can eventually flow through this same pipeline
(see "Rust unification" below), collapsing the semantic layer to a
single well-tested ingestion path.

1. New port in `graphengine-parsing/src/application/ports.rs` (or a new
   `ports/semantic_index.rs`):
   `trait SemanticIndex { fn definition_at(&self, file, line, col) -> Option<IndexTarget>; fn language(&self) -> &str; }`
   with `IndexTarget { file, line, col, symbol_moniker, confidence }`.
2. New crate `graphengine-scip-adapter`:
   - Dependency: `protobuf`/`prost` + the SCIP schema (vendor the
     `scip.proto` from github.com/scip-code/scip; pin the version).
   - Reads an `index.scip` file, builds an in-memory occurrence table
     (file → sorted occurrences with symbol + role), resolves
     definition-at-position queries offline.
   - Provisioning module: given a repo root, detect `package.json` /
     `tsconfig.json`, run `npx --yes @sourcegraph/scip-typescript index`
     (bounded timeout, captured stderr); on failure return a
     `SkipReason::IndexerFailed(exit_code)` for the disclosure contract
     from plan 01 — NEVER silently degrade.
3. New `graphengine-parsing/src/infrastructure/semantic/index_backed.rs`:
   `IndexBackedSemanticResolver` implementing the existing
   `SemanticResolver` port, generic over `SemanticIndex` — mirrors
   rust_layer2's caller/callee symbol-index mapping (reuse its
   `SymbolIndex` by extracting it to a shared module
   `infrastructure/semantic/symbol_mapping.rs`; do not copy-paste).
   Edges stamped `ProvenanceSource::Compiler`.
4. Factory: route TS scans to it when an index exists/builds; else
   heuristic with `SkipReason` disclosure. Per the routing policy above,
   subprocess LSP is NOT in this chain.
5. Python: repeat with `@sourcegraph/scip-python` (same port, new
   provisioning arm). Ship TS first, prove, then Python.
6. Tests: a committed tiny TS fixture repo + prebuilt `index.scip` (so CI
   needs no Node); assert exact expected edges with Compiler provenance;
   an E2E gated test that builds the index live when node is present.

### Rust unification (follow-up inside this phase, after TS proves)

Run `rust-analyzer scip .` on this repo and feed the artifact through the
same ingester; diff resulting edges against the in-process
`rust_layer2` adapter's output. Two free wins: (a) a deterministic
cross-check of both engines (disagreements are bugs in one of them —
triage each), (b) evidence for choosing Rust's production engine.

MANDATORY QUESTION this cross-check must answer (feeds master-plan G1's
Rust self-scan bar): does the batch `rust-analyzer scip` output include
call sites inside proc-macro-expanded bodies that the per-position
adapter path misses? UF-FU-012(b)/UF-FU-003 measured ~88.9% adapter miss
share on macro-heavy code (this repo is serde-derive dense; production
slice measured 2.52% high ratio in UF-FU-011). If SCIP output covers
expansions, the unification decision likely flips to the SCIP path for
Rust regardless of speed; if it does not, macro-expansion coverage via
ra's expansion API becomes a named, scoped follow-up — not a silent gap.
Decision rule: if SCIP-path quality >= ra-ide-adapter quality and total
scan time is within 1.5x, switch Rust to the universal pipeline and keep
`graphengine-ra-ide-adapter` for future incremental/per-position use;
otherwise keep the in-process adapter as Rust's engine and record why in
this file. Either way the provenance is `Compiler`.

### Language expansion roadmap (post-TS/Python; plan 08 opens to contributors)

| Language | Indexer | Provisioning need |
|---|---|---|
| Go | scip-go | go toolchain, module download |
| Java/Scala/Kotlin | scip-java | build-tool integration (Gradle/Maven) — hardest provisioning |
| C/C++ | scip-clang | compile_commands.json |
| Ruby | scip-ruby | bundler env |
| C# | scip-dotnet | .sln/.csproj restore |
| Apex | none exists | stays on subprocess LSP (jorje) or heuristic — see Apex decision |

## Speed note (why this also makes mass scanning feasible)

Batch indexes amortize: one `scip-typescript index` run replaces
thousands of per-position LSP round-trips with in-memory lookups. Record
per-scan timing in the report (`resolution_quality.semantic_ms`) so plan
05 can budget observatory throughput from measured data.

## Acceptance criteria (binding)

- [ ] Rust self-scan edges carry `Compiler` source; `compiler_edges > 0`
      through the standard MCP scan path (depends on plan 01 fix).
- [ ] `Provenance` with corroboration round-trips storage; old DBs load.
- [ ] Heuristic agreement corroborates, never duplicates (tests prove).
- [ ] TS fixture scan produces Compiler-provenance call+import edges from
      a SCIP index with zero subprocess LSP involvement.
- [ ] A failed indexer surfaces `SkipReason::IndexerFailed` in the
      disclosure section — verified by a test with a broken fixture.
- [ ] Factory chain for batch-indexed languages contains no subprocess
      LSP link (test asserts the chain composition).
- [ ] Rust unification cross-check executed and the engine decision
      recorded in this file with measured numbers.
- [ ] `cargo +1.91 test --workspace` green.

## Explicit non-goals

- No DELETION of `infrastructure/lsp` in this plan — demotion to
  maintenance mode only. It still serves Apex, the benchmark witness
  role, and the escape hatch. Deleting ~thousands of tested LOC while it
  has live consumers is churn, not cleanup; revisit after the Apex
  decision.
- No statistical calibration model yet (plan 04 measures calibration;
  the model stays ordinal until data says otherwise).
- No Go/Java/C#/Ruby adapters yet (plan 08 opens that door to
  contributors; scip-java/scip-dotnet/scip-ruby exist when ready).
- MCP `tier_legend` wording update ships with plan 04's vocabulary
  release, not here.
