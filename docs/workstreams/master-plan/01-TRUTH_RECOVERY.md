# 01 — TRUTH RECOVERY: find and fix why the semantic tier is skipped

Status: COMPLETE (2026-07-02) — all acceptance bars pass; final
adjudication in evidence/01-adjudication.md. Plan 02 unblocked.
Depends on: nothing. Highest priority. Blocks Gate G1.
Executor profile: an agent with Rust competence and access to this repo.
Everything you need to know is in this file.

## Context (read even if you know the repo)

GridSeak scans a repo (tree-sitter parse → semantic resolution → SQLite
graph → analysis → report). Semantic resolution has three shapes:

- **Layer-2 compiler adapter** (Rust only today): in-process
  rust-analyzer via `graphengine-ra-ide-adapter`, wrapped by
  `graphengine-parsing/src/infrastructure/semantic/rust_layer2.rs`,
  selected in `graphengine-parsing/src/application/use_cases/parse_repo/factory.rs`,
  consumed via `SemanticResolverService::resolve_with_fallback` in
  `.../pipeline/orchestrator.rs` (~line 419).
- **Subprocess LSP** (other languages): `graphengine-parsing/src/infrastructure/lsp/*`
  with per-language YAML configs in `graphengine-parsing/configs/`.
- **Heuristic fallback** (always available): name matching in
  `.../parse_repo/resolution/fallback.rs`, emits Low/Medium tree-sitter
  provenance edges.

Every edge is stamped `Provenance { source: TreeSitter|Lsp|Heuristic,
confidence: Low|Medium|High }` (`graphengine-parsing/src/domain/provenance.rs`).
The analysis crate computes `resolution_quality.measured_fidelity` from
edge confidence counts.

## The verified problem (2026-07-01 evidence)

1. Fresh MCP-triggered scan of this repo (scan id
   `7d3f8035-e63e-438b-b440-0314570b22d8`, report at
   `~/Library/Application Support/com.gridseak.desktop/project-reports/7d3f8035-e63e-438b-b440-0314570b22d8.report.json`)
   returned `measured_fidelity.tier = "syntactic_only"` and
   `high_ratio_on_calls = 0.143` (3,772 high / 97 medium / 22,440 low
   call edges) despite Rust being the flagship Tier-3 language and the
   Layer-2 adapter existing and having passing integration tests
   (`graphengine-parsing/tests/rust_layer2_integration.rs`).
2. The report JSON contains **no LSP/Layer-2 telemetry section at all**
   (no session metrics, no fallback-reason histogram), so the skip was
   silent — this is exactly the trust break the product exists to prevent.
3. Separate MCP defect: `gridseak_context_for_llm` with default
   `project: "."` resolved to a different repo (`pg-meta` under
   ~/Desktop/supabase) instead of the caller's workspace. Wrong-project
   answers are silent and catastrophic for agent trust.

## Objective

Make the production scan path (CLI `gridseak scan` and the MCP
`gridseak_scan` tool — they share `gridseak-engine-runner`) actually run
the best available semantic resolver, and make every skip loud,
machine-readable, and user-visible. Fix MCP project routing.

## Non-goals

- Do NOT add new resolution capability here (that is plan 02).
- Do NOT tune LSP performance knobs beyond what the diagnosis demands.
- Do NOT invest in fixing subprocess LSP for TypeScript/Python/Go/Java:
  plan 02 v2 replaces subprocess LSP with batch semantic indexes for all
  batch-indexed languages. Sinking effort into the subprocess path for
  those languages is wasted work. In THIS plan, the semantic engine to
  revive is the Rust Layer-2 in-process adapter (it exists and its
  integration tests pass); everything else gets honest disclosure, not
  repair.

## Step-by-step

### Step 1 — Reproduce with instrumentation (no code changes)
1. Build: `cargo +1.91 build -p gridseak-cli --release`.
2. Run a scan of this repo with maximal logging:
   `RUST_LOG=rust_layer2=debug,graphengine_parsing=info ./target/release/gridseak scan . 2>&1 | tee /tmp/truth_recovery_scan.log`.
3. In the log, find whether `rust_layer2::init` ("rust Layer-2 adapter
   loaded in N ms") appears. Three possible outcomes:
   - A: init line absent → factory never constructed the adapter for this
     scan path. Inspect `factory.rs` selection conditions (language
     detection, feature flags, env vars, `is_available` checks).
   - B: init appears but `resolve done: ... high=0` → adapter runs but
     resolves nothing (workspace-root mismatch, path shapes, or the
     multi-language scan (`scan_languages` includes 5 languages) routing
     references away from the Rust resolver).
   - C: init appears, resolutions happen, but confidence counts in the
     report do not reflect them → edges lost between resolver output and
     report aggregation (check `ResolvedEdges` merge and
     `graphengine-analysis` fidelity computation inputs).
4. Write the diagnosis to
   `docs/workstreams/master-plan/evidence/01-diagnosis.md` with log
   excerpts. This file is the deliverable of Step 1.

### Step 2 — Fix the root cause
Whatever branch A/B/C found, fix it minimally and cleanly. Known likely
suspects, in order:
- Factory gating: Layer-2 may only activate for single-language `rust`
  scans, while MCP scans pass `scan_languages: [rust, apex, typescript,
  python, javascript]`; if so, route per-language reference batches to
  per-language resolvers instead of first-match.
- Workspace root: the MCP scan canonicalizes paths through the local
  store; `RustLayer2SemanticResolver::new(workspace_root)` may receive a
  different root shape than tests use.
- Silent early return: `resolve()` short-circuits when
  `hints.language != "rust"` — verify what `language` is set to on
  multi-language scans.

### Step 3 — No-silent-fallback contract
1. Define, in `graphengine-parsing/src/application/ports.rs` (or a new
   small module `.../application/resolution_disclosure.rs`), a
   `ResolutionDisclosure` struct: per language →
   `{ tier_attempted, tier_used, skip_reason: Option<SkipReason> }` where
   `SkipReason` is a closed enum (e.g. `AdapterInitFailed`,
   `ServerMissing`, `LanguageNotRouted`, `NoReferences`,
   `PolicyDisabled`). No free-text-only reasons.
2. Thread it from the resolver service into the scan report
   (`graphengine-analysis` health report next to `resolution_quality`)
   and into the MCP scan/status responses.
3. CLI `gridseak scan` prints one line per language:
   `rust: semantic (layer2, 4,812 high edges)` or
   `python: heuristic only (skip: ServerMissing pyright-langserver)`.

### Step 4 — Fix MCP project routing (defect F2)
1. In `gridseak-cli` MCP server (`gridseak-cli/src/main.rs` and the
   local-store project resolution), make `project: "."`/default resolve
   against the MCP client's workspace root, not the most recent scan in
   the global store. If workspace root is unavailable, return an error
   listing known projects — never silently pick one.
2. Add a regression test: two registered projects, default param, assert
   error-or-correct rather than most-recent.

### Step 5 — Verify
1. Re-run the Step-1 scan command. Assert in the report JSON that
   Layer-2 contributed (compiler/semantic edge count > 0) and record
   `high_ratio_on_calls` + the adapter conversion rate (below) in
   `evidence/01-results.md` against the baseline (0.143 whole-repo,
   scan `7d3f8035…`).
2. Run the full test suite: `cargo +1.91 test --workspace`.
3. Re-run the MCP path (`gridseak_scan` tool) and confirm the same
   numbers arrive through MCP, and disclosure section is present.
4. Record before/after numbers in
   `docs/workstreams/master-plan/evidence/01-results.md`.

## Acceptance criteria (binding — AMENDED 2026-07-01, see note)

AMENDMENT NOTE: v1 of this plan copied master-plan G1's
`high_ratio_on_calls >= 0.60 / tier: semantic` bar into this list. That
was a drafting error: G1 is defined as the JOINT exit of plans 01+02,
and GridSeak's own measurements (UF-FU-012, 2026-04-21) show the
Rust-Layer-2 ceiling on this proc-macro-dense repo is ~10–20% until
plan 02's work lands (36% SymbolIndex mapping loss + adapter misses on
macro-expanded call sites). Plan 01's bars below are scoped to what this
plan controls; the 0.60 bar lives in 00-MASTER_PLAN §5 (G1, amended the
same day). The plan-owner made this amendment; executors still may not
alter criteria (§7).

AMENDMENT NOTE 2 (2026-07-02, plan owner): the conversion bar is
measured over the **mappable universe** — adapter resolutions minus
legitimate drops (recursion self-loops, duplicate pairs, and
definitions outside the scanned workspace: cargo registry / `target/`
build output). The P7 stratified sample proved 29/30 residual misses
were external-crate targets that can never map to an in-repo node; the
Layer-2 telemetry now splits them out (`external_target_misses`),
mirroring the LSP channel's `ExternalDefinition` vs
`DefinitionUnmappable` distinction. Raw conversion is still computed
and reported. Full rationale + numbers: evidence/01-adjudication.md §P9.

- [x] Diagnosis document exists with log evidence naming root cause A/B/C.
      (evidence/01-diagnosis.md)
- [x] Rust Layer-2 is active through the standard MCP scan path:
      compiler-tier call edges > 0 on the self-scan, adapter init and
      resolve visible in logs/telemetry. (5,862 emitted edges, report
      `3755b582-1d49-459e-966f-6d2737b21dac`)
- [x] SymbolIndex mapping loss fixed: adapter-resolved → emitted
      conversion rate >= 0.90 on the self-scan (UF-FU-012(a) baseline:
      6,550 / 10,311 = 0.635). (0.9501 on the mappable universe;
      see AMENDMENT NOTE 2)
- [x] Whole-repo `high_ratio_on_calls` >= 0.18 on the self-scan
      (evidence-derived ceiling target per UF-FU-012's own math;
      baseline 0.143), recorded with scan id in evidence/01-results.md.
      (0.2168, report `3755b582…`)
- [x] Every scan report + MCP response contains per-language
      `ResolutionDisclosure`; a language with no semantic resolution has
      a machine-readable `SkipReason`. (5 rows: rust layer2, ts/py
      subprocess_lsp, js server_missing, apex policy_disabled)
- [x] MCP default-project routing can no longer return an unrelated repo;
      regression test proves it. (2 regression tests in
      gridseak-local-store)
- [x] `cargo +1.91 test --workspace` passes; no clippy regressions on
      touched files. (re-run at closing commit)

## Rollback / risk

All changes are additive except factory routing; keep the old routing
behind a single commit so `git revert` restores it. The disclosure
struct is additive to reports (serde-default), no schema migration.
