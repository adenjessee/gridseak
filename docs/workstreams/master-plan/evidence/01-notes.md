# 01 — TRUTH RECOVERY: planner notes (defects & drift found during breakdown)

Author: senior-planner pass, 2026-07-01. This file records where plan
`01-TRUTH_RECOVERY.md` drifted from the actual code as of branch
`lsp-reliability-foundation` (commit `ce724323`). Per the master-plan
standing rules and EXECUTION_PROTOCOL §7, the plan file is read-only to
me; corrections live here and inside the affected tasks in
`01-TRUTH_RECOVERY-TASKS.md`. None of these invalidate the plan's
objective — they change *how* individual steps must be executed.

## D1 — The scan path is a multi-process subprocess pipeline, not one process

The plan (Step 1) says CLI `gridseak scan` and the MCP `gridseak_scan`
tool "share `gridseak-engine-runner`" and implies a single process you
can attach `RUST_LOG` to. Verified reality:

- `gridseak-engine-runner/src/lib.rs::run_pipeline` spawns the **parser
  binary** (`graphengine-parsing`) once *per language* as a subprocess:
  `graphengine-parsing parse --root … --db … --lang <L> --lsp-policy fast`
  (lib.rs lines 211–230, `run_parser_for_language` lines 291–367), then
  spawns the **analyzer binary** (`ge-analyze`) as a second subprocess
  (`run_analyzer` lines 369–430). Binaries are resolved by
  `resolve_engine_bin("graphengine-parsing")` / `"ge-analyze"`
  (`gridseak-cli/src/main.rs` ~985).
- Consequence for Step 1: the `rust_layer2::init` log line
  ("rust Layer-2 adapter loaded in N ms") and the factory's
  "Rust Layer-2 adapter unavailable …" fallback line are emitted **in
  the parser subprocess at `info!` level**, written to that
  subprocess's stderr, which the runner tees to
  `<scratch>/<scan_id>.parse.rust.stderr`.

## D2 — The parser binary ignores `RUST_LOG` (plan Step 1 command is wrong)

`graphengine-parsing/src/main.rs` (lines 166–180) builds its tracing
filter as `EnvFilter::new("debug")` when `--verbose` is passed, else
`EnvFilter::new("warn")`. It does **not** call
`EnvFilter::from_default_env()`, so `RUST_LOG` has no effect on the
parser subprocess, and the runner never passes `--verbose`. The
`rust_layer2::init` / factory fallback lines are `info!` (below `warn`),
so under a normal `gridseak scan` they are **suppressed entirely** — the
skip is doubly silent.

Correct Step-1 reproduction is to run the parser binary directly with
`--verbose` (see Task T0), not `RUST_LOG=… gridseak scan .`.

## D3 — `measured_fidelity.tier` has no `semantic` value

Both `00-MASTER_PLAN.md` §5 G1 and `01-TRUTH_RECOVERY.md` acceptance
criteria say the self-scan tier must reach `"semantic"`. The actual
enum is `MeasuredFidelityTier` in
`graphengine-analysis/src/health/report.rs` (lines 912–939), serialized
`rename_all = "snake_case"` with exactly four values:
`authoritative`, `heuristic_primary`, `syntactic_only`, `unknown`.
Thresholds (from `from_call_edges` + the unit tests lines 958–1026):

- `>= 0.80` high ratio → `authoritative`
- `0.40 ≤ ratio < 0.80` → `heuristic_primary`
- `0 < ratio < 0.40` → `syntactic_only`
- no call edges → `unknown`

There is **no `semantic` variant**, and adding/renaming one is out of
scope for plan 01 (it would break the tier legend, `t3_*`/`t4_*`
analysis tests, and the MCP `tier_legend`; that vocabulary change is
explicitly scheduled for plan 04 step 6 per the baseline doc §Working
notes). **AMENDED 2026-07-01 (plan-owner):** Plan 01's binding bars no longer
include 0.60 / invented `semantic` tier. Plan 01 exit: conversion ≥
0.90, `high_ratio_on_calls` ≥ 0.18, Layer-2 active. G1's 0.60 bar
lives in `00-MASTER_PLAN.md` §5 (joint exit of plans 01+02).

## D8 — Confirmed root cause: SymbolIndex mapping loss (not factory skip)

UF-FU-012 (2026-04-21 Gate 1.2 dogfood): adapter `high+medium=10311`,
`edges_emitted=6550`, conversion **0.635**. Layer-2 runs; losses occur
in `SymbolIndex::find_enclosing_function` / `find_callee_for` in
`rust_layer2.rs`. Proc-macro adapter misses (~88.9%) are a separate
ceiling (UF-FU-003) — out of scope for plan 01 (plan 02).

## D4 — `high_ratio_on_calls` is a whole-repo, all-languages ratio

The report's `measured_fidelity` is computed over *all* call-like edges
across every language pass persisted into the shared DB (baseline:
high 3,772 / medium 97 / low 22,440 = 0.143). Reviving Rust Layer-2
raises only the Rust share; the repo's Apex/TS/Python/JS heuristic Low
edges still drag the global ratio. Hitting the ≥ 0.60 target is
therefore **not guaranteed** by fixing Rust alone and must be *measured*
after the fix, not assumed. If the fix makes Rust authoritative but the
global ratio stalls below 0.60, that is a real finding to surface (it
would mean the target either needs non-Rust work — deferred to plan 02 —
or the gate math should count per-language, a plan-04 concern). Do not
silently redefine the criterion; report the measured number.

## D5 — Per-language disclosure cannot use unkeyed graph metadata

Plan Step 3 wants **per-language** `ResolutionDisclosure`. Today the
parser persists resolution/session telemetry as unkeyed metadata rows
(`session_start_attempts`, `resolution_lsp_edges`, `language`, … —
`orchestrator.rs` lines 476–583) into a **persistent DB shared across
all language passes**. Because each `graphengine-parsing parse --lang X`
run writes the same keys, the **last language pass clobbers the
earlier ones** (the report's `session_*` block already reflects only the
final pass, not Rust). Any per-language disclosure MUST therefore be
written under a **per-language key** (e.g. `resolution_disclosure_<lang>`
JSON) and the analyzer must enumerate those keys (SQL `LIKE
'resolution_disclosure_%'`) rather than read one row. This is specified
inside Tasks T2/T3.

## D6 — `rust-layer2` feature is default-ON (so "feature disabled" is unlikely branch A)

`graphengine-parsing/Cargo.toml` line 130: `default = ["rust-layer2"]`.
A normal `cargo build -p graphengine-parsing` (and the release tarball)
compiles the `ra_ap_ide`-backed resolver in. Plan Step 1 branch A lists
"feature flags" among suspects; confirm the *shipped/installed* sidecar
was not built `--no-default-features`, but do not assume the feature is
the cause — the silent `Err(...) → subprocess-LSP` fallback in
`factory.rs::build_rust_semantic_resolver` (lines 227–256) is the more
likely runtime path and is itself a silent-fallback defect the plan's
Step 3 contract should make loud.

## D7 — MCP wrong-project (F2) root cause is concrete and has a test to rewrite

`ProjectStore::resolve_project_lenient` (`gridseak-local-store/src/
store.rs` lines 396–420) intentionally falls back to
`latest_scanned_project()` (lines 426–444) when the cwd has no project —
this is exactly the pg-meta wrong-project behavior. The MCP tools call
it at `gridseak-cli/src/main.rs` lines 1388, 1485, 1549, 1890, 2259,
2297 (default `project` = `"."`, `default_project_ref` line 1200). The
existing test `store.rs` lines 1528–1544 **asserts the latest-scan
fallback works** and will need rewriting when the fallback is removed
(Task T5). This is in-scope and precise; not a defect in the plan, just
a gotcha the executor must not miss.

---

## Punch list execution (2026-07-02, branch `plan-01-truth-recovery`)

Scan **`6785ed5b-3ee8-4a41-84b6-f049151bb859`** (fresh `./target/release/gridseak scan .`
after `cargo +1.91 build --release -p graphengine-parsing -p graphengine-analysis
-p gridseak-cli`). Parse DB:
`~/Library/Caches/com.gridseak.desktop/parse-dbs/2cb0b67a-21c8-4558-8630-164504f7589f/parse.sqlite`.
Report:
`~/Library/Application Support/com.gridseak.desktop/project-reports/6785ed5b-3ee8-4a41-84b6-f049151bb859.report.json`.

### P1 — Conversion accounting (identity closes exactly)

`resolution_layer2_telemetry_json` from the scan DB (counter fields only; samples
truncated here — full array in DB):

```json
{
  "call_refs_seen": 41128,
  "high_resolutions": 14666,
  "medium_resolutions": 5,
  "no_target_misses": 26285,
  "adapter_errors": 172,
  "edges_emitted": 4127,
  "symbol_caller_misses": 10,
  "symbol_callee_misses": 4548,
  "self_loop_drops": 4799,
  "duplicate_edge_drops": 1187
}
```

**Identity check (must hold exactly):**

```
edges_emitted + caller_misses + callee_misses + self_loop_drops + duplicate_drops
= high_resolutions + medium_resolutions

4127 + 10 + 4548 + 4799 + 1187 = 14671
14666 + 5 = 14671  ✓
```

**Re-derived conversion (P1 formula):**

| Metric | Formula | Value |
|--------|---------|------:|
| Raw conversion | `edges_emitted / (high + medium)` | **0.281** |
| Mapping conversion | `edges_emitted / (high + medium − self_loop − duplicate)` | **0.475** |
| Plan 01 bar (≥ 0.90) | mapping conversion | **FAIL** |

Legitimate drops: `self_loop_drops` (4,799) + `duplicate_edge_drops` (1,187) =
5,986 resolutions that correctly produce no graph edge.

### P2 — Absolute emitted drop (6,550 April → 4,127 now)

Evidence chain (same repo, different measurement generations):

| Source | `high+medium` | `edges_emitted` | Notes |
|--------|-------------:|----------------:|-------|
| UF-FU-012 (2026-04-21 Gate 1.2) | 10,311 | **6,550** | Pre-T1 SymbolIndex; no duplicate dedup |
| Adjudication scan `e9ce6a78…` @ b17cf01 | 14,646 | **5,299** | T1 raised resolutions; ~4,801 unaccounted (self-loops) |
| Punch-list scan `6785ed5b…` | 14,671 | **4,127** | P1 counters + duplicate dedup active |

**Explained components (not hypothesis):**

1. **Duplicate-edge dedup (new in punch list):** `duplicate_edge_drops = 1,187`.
   Pre-dedup equivalent on this scan: `4127 + 1187 = 5,314`, within 0.3% of
   adjudication **5,299** — the adjudication→current drop is almost entirely
   duplicate suppression, not adapter regression.
2. **Self-loop resolutions (always dropped, now named):** `self_loop_drops = 4,799`.
   These never became edges (validator rejects Call self-loops); pre-P1 they
   inflated the “unaccounted” gap (~4,801 in adjudication), not `edges_emitted`.
3. **Denominator inflation from T1:** `high+medium` rose 10,311 → 14,671 (+42%).
   Callee mapping loss stayed ~flat in absolute terms (`symbol_callee_misses ≈ 4,548`),
   so raw conversion fell even when adapter quality did not regress.
4. **April 6,550 → current 4,127 residual (~1,236):** combines (a) duplicate dedup
   not present in April measurement, (b) larger resolved set with unchanged callee
   mapping ceiling, (c) April dogfood used a different harness snapshot (UF-FU-012
   `10311` resolutions vs today's `14666`). No single code revert explains the gap;
   it is measurement + dedup + denominator growth.

### P3 — `symbol_callee_misses` spot-check (25 samples from telemetry)

Auto-classification rule: if a `Function` node exists in the tree-sitter index for
the adapter target file+name → `plan01_mappable`; else → `plan02_out_of_scope`
(external crate, proc-macro expansion site, or missing extractor node).

| # | file:line | target symbol | class |
|---|-----------|---------------|-------|
| 1 | `graphengine-analysis/src/bin/ge_analyze.rs:137` | `Cli` | plan02_out_of_scope |
| 2 | `graphengine-analysis/src/bin/ge_analyze.rs:143` | `_` (lib root) | plan02_out_of_scope |
| 3 | `graphengine-analysis/src/bin/ge_analyze.rs:156` | `_` (rusqlite) | plan02_out_of_scope |
| 4 | `graphengine-analysis/src/bin/ge_analyze.rs:167` | `_` (lib root) | plan02_out_of_scope |
| 5 | `graphengine-analysis/src/bin/ge_analyze.rs:175` | `_` (lib root) | plan02_out_of_scope |
| 6 | `graphengine-analysis/src/bin/ge_analyze.rs:178` | `_` (serde_json) | plan02_out_of_scope |
| 7 | `graphengine-analysis/src/bin/ge_analyze.rs:179` | `_` (serde_json) | plan02_out_of_scope |
| 8 | `graphengine-analysis/src/bin/ge_analyze.rs:216` | `_` (lib root) | plan02_out_of_scope |
| 9 | `graphengine-analysis/src/bin/ge_analyze.rs:235` | `_` (rusqlite) | plan02_out_of_scope |
| 10 | `graphengine-analysis/src/bin/ge_analyze.rs:240` | `_` (lib root) | plan02_out_of_scope |
| 11 | `graphengine-analysis/src/bin/ge_analyze.rs:244` | `_` (lib root) | plan02_out_of_scope |
| 12 | `graphengine-analysis/src/bin/ge_analyze.rs:262` | `_` (lib root) | plan02_out_of_scope |
| 13 | `graphengine-analysis/src/bin/ge_analyze.rs:289` | `git_signals_attach` | plan02_out_of_scope |
| 14 | `graphengine-analysis/src/bin/ge_analyze.rs:292` | `HistoryWindow` | plan02_out_of_scope |
| 15 | `graphengine-analysis/src/bin/ge_analyze.rs:303` | `git_signals_attach` | plan02_out_of_scope |
| 16 | `graphengine-analysis/src/bin/ge_analyze.rs:319` | `_` (rusqlite) | plan02_out_of_scope |
| 17 | `graphengine-analysis/src/bin/ge_analyze.rs:323` | `SqliteRepository` | plan02_out_of_scope |
| 18 | `graphengine-analysis/src/bin/ge_analyze.rs:341` | `coverage_attach` | plan02_out_of_scope |
| 19 | `graphengine-analysis/src/bin/ge_analyze.rs:349` | `coverage_attach` | plan02_out_of_scope |
| 20 | `graphengine-analysis/src/bin/ge_analyze.rs:360` | `coverage_attach` | plan02_out_of_scope |
| 21 | `graphengine-analysis/src/bin/ge_analyze.rs:369` | `_` (serde_json) | plan02_out_of_scope |
| 22 | `graphengine-analysis/src/bin/ge_analyze.rs:370` | `_` (serde_json) | plan02_out_of_scope |
| 23 | `graphengine-analysis/src/bin/ge_analyze.rs:383` | `_` (lib root) | plan02_out_of_scope |
| 24 | `graphengine-analysis/src/bin/ge_analyze.rs:394` | `_` (lib root) | plan02_out_of_scope |
| 25 | `graphengine-analysis/src/bin/ge_analyze.rs:413` | `config` | plan02_out_of_scope |

**Sample verdict:** 25/25 `plan02_out_of_scope`, 0/25 `plan01_mappable`. Dominant
pattern: adapter resolves to crate-root `_`, external registry paths, or struct/type
constructors absent from tree-sitter `Function` nodes. **No evidence in this sample
that fixing plan-01 mapping alone adds ~900 edges** (the adjudication back-of-envelope
for crossing `high_ratio ≥ 0.18`).

### P4 / P5 pointers

- P4 disclosure rows in report JSON `resolution_quality.resolution_disclosure`:
  apex (`policy_disabled`), javascript (`server_missing`), python (12 emitted),
  rust (4127 emitted, layer2), typescript (15 emitted). Field renamed to
  `emitted_edges` (alias `high_edges` for back-compat).
- P5: `graphengine-parsing/tests/config_loading.rs` asserts `pyright-langserver`.

### Final measured bars (this scan)

- **`mapping_conversion_rate`:** **0.475** (bar 0.90: FAIL)
- **`high_ratio_on_calls`:** **0.1583** (bar 0.18: FAIL; baseline 0.143)
- **`measured_fidelity.tier`:** `syntactic_only`

---

## Punch list v2 execution (2026-07-02, adjudication addendum P6–P8)

Scan **`69f31ae9-fda7-4acc-93f5-bec58f0b9be7`** after P6 `caret_for_callee` fix
(commit pending). Release rebuild + `./target/release/gridseak scan .` (90s).

### P6 — `caret_for_callee` for qualified paths

**Fix:** for `a::b::f(x)` (single- and multi-line), position the rust-analyzer
caret on the **final path segment** (`f`), not `call_range.start` (`a`). Unit tests
in `rust_layer2.rs` `caret_tests` (`qualified_single_line_*`,
`qualified_multi_line_*`).

**Effect:** rust Layer-2 `emitted_edges` **4,127 → 5,859** (+1,732); callee misses
**4,548 → 1,470** (−3,078). Root cause confirmed: module-segment resolution from
wrong caret, not proc-macro ceiling.

### P8 — Telemetry JSON (identity + bars)

```json
{
  "high_resolutions": 13605,
  "medium_resolutions": 5,
  "edges_emitted": 5859,
  "symbol_caller_misses": 10,
  "symbol_callee_misses": 1470,
  "self_loop_drops": 4789,
  "duplicate_edge_drops": 1482
}
```

**Identity:** `5859+10+1470+4789+1482 = 13610 = 13605+5` ✓

| Metric | Value | Bar | Verdict |
|--------|------:|-----|---------|
| Mapping conversion | **0.798** | ≥ 0.90 | **FAIL** |
| Raw conversion | **0.431** | — | — |
| **`high_ratio_on_calls`** | **0.2167** | ≥ 0.18 | **PASS** |
| `measured_fidelity.tier` | `syntactic_only` | — | (&lt; 0.40 high-ratio tier band) |

### P7 — Stratified callee-miss sample (30 / 20 files)

Corrected classification: intended callee from `function_name` final segment;
module-segment / caret-bug pattern → `plan01_mappable`. Deterministic shuffle
(seed `0x5071_0107`), ≥10 files required — **30 samples across 20 files**.

| # | file:line | target | class |
|---|-----------|--------|-------|
| 1 | `graphengine-analysis/src/health/pipeline/l1_merge.rs:63` | rusqlite `open_with_flags` | plan02 |
| 2 | `graphengine-analysis/src/health/pipeline/l1_merge.rs:93` | chrono `to_rfc3339` | plan02 |
| 3 | `graphengine-analysis/src/health/pipeline/l1_merge.rs:93` | chrono `now` | plan02 |
| 4 | `graphengine-parsing/src/syntax/language/apex/class_symbol_codec.rs:104` | serde_json `to_string` | plan02 |
| 5 | `graphengine-ra-ide-adapter/tests/adapter_integration.rs:35` | `RustAnalyzerSemanticResolver` | plan02 |
| 6 | `graphengine-ra-ide-adapter/tests/adapter_integration.rs:92` | `RustAnalyzerSemanticResolver` | plan02 |
| 7 | `graphengine-ra-ide-adapter/tests/adapter_integration.rs:117` | tempfile `tempdir` | plan02 |
| 8 | `graphengine-analysis/src/health/pipeline/session.rs:90` | rusqlite `open_in_memory` | plan02 |
| 9 | `graphengine-parsing/src/infrastructure/lsp/session_options.rs:166` | url `from_file_path` | plan02 |
| 10 | `graphengine-parsing/src/infrastructure/lsp/session_options.rs:140` | walkdir `WalkDir` | plan02 |
| 11 | `graphengine-parsing/src/infrastructure/lsp/session_options.rs:140` | walkdir `WalkDir` | plan02 |
| 12 | `graphengine-parsing/tests/extractor_constructor_fixtures.rs:44` | tempfile `tempdir` | plan02 |
| 13 | `gridseak-cli/src/setup/windsurf.rs:58` | serde_json `Object` | plan02 |
| 14 | `gridseak-cli/src/setup/windsurf.rs:27` | serde_json `pointer` | plan02 |
| 15 | `graphengine-infra/tests/template_service_classification_filters.rs:123` | tempfile `path` | plan02 |
| 16 | `graphengine-infra/tests/template_service_classification_filters.rs:89` | tempfile `path` | plan02 |
| 17 | `graphengine-analysis/tests/t3f_fidelity_gap_regression.rs:223` | lib `_` | plan02 |
| 18 | `graphengine-parsing/src/application/use_cases/containment_builder.rs:188` | serde_json `String` | plan02 |
| 19 | `graphengine-parsing/src/application/use_cases/containment_builder.rs:302` | serde_json `Array` | plan02 |
| 20 | `graphengine-parsing/src/application/use_cases/containment_builder.rs:184` | serde_json `to_value` | plan02 |
| 21 | `gridseak-cli/src/render/history.rs:367` | chrono `parse_from_rfc3339` | plan02 |
| 22 | `graphengine-parsing/tests/domain/benches/graph_bench.rs:95` | criterion `benchmark_group` | plan02 |
| 23 | `graphengine-parsing/src/syntax/language/apex/trigger_framework.rs:246` | `LIFECYCLE_METHODS` | **plan01** |
| 24 | `graphengine-parsing/tests/failure_simulation.rs:249` | `ParseRepositoryUseCase` | plan02 |
| 25 | `graphengine-parsing/src/infrastructure/lsp/client.rs:170` | tokio `args` | plan02 |
| 26 | `graphengine-parsing/tests/typescript_extraction_tests.rs:86` | tree-sitter-ts `language_typescript` | plan02 |
| 27 | `gridseak-engine-runner/tests/s2_incremental_analysis.rs:180` | tempfile `tempdir` | plan02 |
| 28 | `graphengine-analysis/src/health/pipeline/cache.rs:65` | chrono `now` | plan02 |
| 29 | `graphengine-parsing/src/syntax/language/apex/sfdx_layout.rs:678` | tempfile `tempdir` | plan02 |
| 30 | `graphengine-git-signals/src/extractor.rs:521` | gix `empty_tree` | plan02 |

**Post-fix sample verdict:** 29/30 `plan02_out_of_scope`, 1/30 `plan01_mappable`.
Remaining 1,470 callee misses are predominantly external-crate / registry paths —
consistent with plan-02 proc-macro / extractor ceiling, not the caret bug that
dominated the pre-P6 miss pool.

**Process note (owner adjudication):** the prior P3 auto-rule (“no Function node on
adapter target → out of scope”) laundered caret-on-module misses into plan02
verdicts; classification rules need the same skepticism as metric claims.

## P9 — external-target split (2026-07-02, plan owner, closes plan 01)

The P7 sample (29/30 misses = registry/crate paths) proved
`symbol_callee_misses` conflated in-repo mapping loss with resolutions
into definitions **outside the scanned workspace** — which can never map
to an in-repo node. Split implemented in `rust_layer2.rs`
(`is_external_target`: outside workspace root, or under the workspace's
`target/` build dir), mirroring the LSP channel's
`FallbackReason::ExternalDefinition` vs `DefinitionUnmappable` split.
External misses join self-loops/duplicates as legitimate drops in
`mapping_conversion_rate`; raw conversion is unchanged and still reported.

Fresh full-parse scan (`--no-incremental --full-analysis`, ~96s, report
`3755b582-1d49-459e-966f-6d2737b21dac`):

```json
{
  "call_refs_seen": 41268,
  "high_resolutions": 13610,
  "medium_resolutions": 5,
  "edges_emitted": 5862,
  "symbol_caller_misses": 10,
  "symbol_callee_misses": 298,
  "external_target_misses": 1172,
  "self_loop_drops": 4793,
  "duplicate_edge_drops": 1480
}
```

**Identity:** `5862+10+298+1172+4793+1480 = 13615 = 13610+5` ✓
(and `298+1172 = 1470` = the pre-split callee-miss count, exactly).

| Metric | Value | Bar | Verdict |
|--------|------:|-----|---------|
| Mapping conversion | **0.9501** (5862 / 6170) | ≥ 0.90 | **PASS** |
| `high_ratio_on_calls` (whole repo) | **0.2168** | ≥ 0.18 | **PASS** |
| Raw conversion | 0.431 | — | — |

Residual: 298 in-repo callee misses; stratified sample now 26/30
plan02_out_of_scope, 4/30 plan01_mappable (constant/static refs). Carried
to plan 02.

Also fixed during final adjudication: executor caret unit test
`qualified_single_line_targets_final_segment` asserted col 9 where the
correct caret for `a::b::f` at col 4 is 4 + len("a::b::") = 10; the test
was failing at HEAD 17a2773. Production code was correct; test corrected.

