# Plan 01 adjudication — owner review (2026-07-01)

Reviewer: parent agent on behalf of owner. Method: independent re-run,
not trust. Rebuilt `gridseak` from `plan-01-truth-recovery` @ b17cf01
and ran a full `gridseak scan .` (92.6s). Scan id
`e9ce6a78-c1c8-4954-aa64-0b73af649aca`.

## Measured results (this scan, plan-01 binary)

- `measured_fidelity.high_ratio_on_calls` = **0.1586**
  (baseline 0.143, scan `7d3f8035…`). Call edges: high 4,127 /
  medium 97 / low 21,794. Tier still `syntactic_only`.
- Graph DB metadata now contains:
  - `resolution_disclosure_rust` =
    `{"language":"rust","tier_attempted":"layer2","tier_used":"layer2","high_edges":14641}`
  - `resolution_layer2_telemetry_json` =
    `{"call_refs_seen":41077,"high_resolutions":14641,"medium_resolutions":5,
      "no_target_misses":26259,"adapter_errors":172,"edges_emitted":5299,
      "symbol_caller_misses":10,"symbol_callee_misses":4536}`
  - `resolution_lsp_edges` = 5299, `resolution_heuristic_edges` = 21666.
- Report JSON top level contains NO disclosure section (checked
  programmatically) — disclosure lives in DB metadata only.

## Bar-by-bar verdict

| Bar | Verdict | Evidence |
|---|---|---|
| Layer-2 active, compiler edges > 0 | **PASS** | 5,299 emitted; disclosure row present; telemetry present |
| Conversion >= 0.90 | **FAIL — and the number is not yet trustworthy either way** (see below) | 5,299 / 14,646 = 0.362 |
| high_ratio_on_calls >= 0.18 | **FAIL (close)** | 0.1586 measured on full scan |
| Per-language disclosure | **PARTIAL** | rust row only; no rows for typescript/python/javascript/apex; not threaded into report JSON/MCP |
| F2 routing regression tests | PASS (per executor; tests exist) | gridseak-local-store |
| Workspace tests green | **NOW FIXED** | `test_python_config_loading` asserted the obsolete `pyright`; contradicted `lsp_config_contract.rs` (PR1 decision: `pyright-langserver`). Fixed in tests/config_loading.rs; suite passes 7/7. This was a real conflict between two of our own tests, not ignorable noise |

## Why the conversion number cannot be accepted OR rejected yet

Two accounting problems make 0.362 uninterpretable:

1. **Apples-to-oranges vs the 0.635 baseline.** UF-FU-012 measured
   10,311 resolutions; T1's improvements raised resolutions 42% to
   14,646. The denominator grew; the mapping layer stayed capped by
   tree-sitter symbol coverage. A conversion drop alongside a
   resolutions rise is not necessarily a regression — but emitted also
   fell in absolute terms (5,299 vs 6,550 in April), which is NOT
   explained yet.
2. **The arithmetic does not close.** emitted (5,299) + caller misses
   (10) + callee misses (4,536) = 9,845, leaving ~4,801 resolved
   references unaccounted (self-loop drops? duplicate-edge dedupe?
   multi-language routing?). Until every resolved reference lands in a
   named bucket, the conversion metric is unfalsifiable.

Also noted: the disclosure row reports `high_edges: 14641`, which equals
high_RESOLUTIONS, not emitted edges (5,299) — a mislabel that would
mislead any consumer.

## Punch list (plan 01 remains OPEN until these close)

- P1. Conversion accounting: add drop counters so
  `emitted + caller_misses + callee_misses + self_loop_drops +
  duplicate_drops + <other named buckets> == resolutions` EXACTLY.
  Re-derive conversion as `emitted / (resolutions - legitimate_drops)`
  (self-loops and true duplicates are correct behavior, not loss) and
  re-measure against the 0.90 bar.
- P2. Explain the absolute emitted drop (5,299 now vs 6,550 April) with
  evidence, not hypothesis.
- P3. Spot-check >= 20 `symbol_callee_misses` via the new telemetry:
  classify each as (a) proc-macro/generated target (plan 02 scope) or
  (b) mappable-but-missed (plan 01 scope — fix). If a meaningful share
  of 4,536 misses are (b), high_ratio likely crosses 0.18
  ((4127 + ~900) / 26,018 ≈ 0.193).
- P4. Disclosure completeness: one row per scan language (typescript,
  python, javascript, apex → with real `skip_reason` values); thread
  disclosure into the report JSON and MCP scan/status responses
  (currently DB-metadata-only); fix the `high_edges` mislabel to report
  emitted edges (or rename the field honestly, e.g. `high_resolutions`).
- P5. Commit the `test_python_config_loading` fix (made during this
  adjudication, uncommitted).

## Disposition

Real, verified progress: Layer-2 runs in production scans, telemetry
exists, disclosure machinery exists, F2 fixed, high_ratio improved
0.143 → 0.159. But plan 01 is NOT complete: two bars fail, one is
unmeasurable as reported, and disclosure is one-language-deep. Do not
start plan 02 until P1–P5 close — P1/P3 directly determine how much of
the remaining gap belongs to plan 02's proc-macro scope, and starting
plan 02 without that attribution would repeat the exact
unverifiable-handoff pattern this workstream exists to kill.

## Addendum — punch-list re-check (2026-07-01, late)

Verified at commit 0924610, scan `6785ed5b…`:
- P1/P2 CLOSED: identity holds (self_loop_drops 4,799; duplicate_edge_drops
  1,187; 4,127+1,187 ≈ prior 5,299 — the emitted drop was dedupe, not
  regression). Conversion redefined: 0.475.
- P4 CLOSED: 5 per-language disclosure rows verified in report JSON
  (rust layer2 4,127; ts LSP 15; py LSP 12; js server_missing;
  apex policy_disabled). Field renamed emitted_edges.
- P5 CLOSED: full workspace suite run by reviewer — zero failures.
- P3 REJECTED: sample was 25 misses from ONE file, and >=5 were
  misclassified. `git_signals_attach` / `coverage_attach` are in-repo
  modules (graphengine-analysis/src/health/), called as
  `module::function(...)`. Root cause identified by reviewer:
  `caret_for_callee` targets call START for non-method calls, so for
  qualified path calls rust-analyzer resolves the MODULE segment, not
  the function — producing module/crate-root targets that can never map
  to a Function node. This is plan-01 scope (caret positioning), not
  proc-macro scope.

### Punch list v2 (plan 01 still OPEN)
- P6. Fix `caret_for_callee` for scoped/qualified calls (last path
  segment before the argument list); unit tests for `a::b::f(x)`.
- P7. Re-do P3: 30 misses, random-stratified across >=10 files,
  classified with the module-segment pattern in mind.
- P8. Rebuild, rescan, re-measure conversion + high_ratio_on_calls.
Bars stand: conversion >= 0.90 (on legit denominator), high_ratio >= 0.18.

## Final adjudication (2026-07-02, plan owner) — plan 01 CLOSED

Verified at commit 17a2773 + P9 change (this commit), fresh full-parse
self-scan (`gridseak scan . --no-incremental --full-analysis`, ~96s,
report `3755b582-1d49-459e-966f-6d2737b21dac`).

### P6/P7/P8 executor claims re-verified
- P6 caret fix: real and effective (callee misses 4,548 → 1,470; edges
  +1,732). One executor-committed caret unit test asserted the wrong
  column (`a::b::` is 6 chars → col 4+6=10, test expected 9) and was
  FAILING at HEAD 17a2773 despite the "tests pass" claim; corrected in
  this commit. The production code was right; the test was wrong.
- P7 corrected sample accepted: 29/30 external registry/crate targets.

### P9 — external-target split (closes the conversion bar honestly)
P7's evidence showed the `symbol_callee_misses` counter conflated two
populations: in-repo mapping loss (plan 01's responsibility) and
resolutions into cargo-registry / `target/` build output, which can
NEVER map to an in-repo node. The LSP channel already makes exactly this
distinction (`FallbackReason::ExternalDefinition` vs
`DefinitionUnmappable`, "so the histogram does not cry wolf"); the
Layer-2 adapter now does too: `is_external_target()` splits misses into
`symbol_callee_misses` (in-repo, still mapping loss) vs
`external_target_misses` (legitimate drop, joins self-loops/duplicates
in the conversion denominator exclusion). Unit tests cover registry
paths, workspace sources, `target/` build output, and a source crate
named `target-utils`.

Post-split telemetry (identity: 5,862+10+298+1,172+4,793+1,480 =
13,615 = 13,610+5 ✓; note 298+1,172 = 1,470, exactly the pre-split
count):

| Metric | Value | Bar | Verdict |
|--------|------:|-----|---------|
| Mapping conversion (mappable universe) | **0.9501** (5,862 / 6,170) | ≥ 0.90 | **PASS** |
| Whole-repo `high_ratio_on_calls` | **0.2168** (5,886 high / 27,151 call edges) | ≥ 0.18 | **PASS** |
| Raw conversion (unchanged definition) | 0.431 | — | reported |
| In-repo callee misses remaining | 298 (26/30 sampled plan02, 4/30 plan01) | — | residual |

Was P9 bar-lowering? No: the bar's own rationale (P1: "self-loops and
true duplicates are correct behavior, not loss") extends to targets
outside the scanned workspace — emission is impossible there, not lossy.
The split is measurement correction backed by the P7 sample, mirroring
an existing product distinction. Raw conversion is still computed and
reported so nobody can hide behind the new denominator.

### Bar-by-bar final verdict
- Diagnosis doc with root cause: PASS (01-diagnosis.md).
- Layer-2 active through MCP/CLI scan path: PASS (5,862 emitted edges,
  disclosure row `rust: layer2`).
- Conversion ≥ 0.90: PASS (0.9501).
- high_ratio_on_calls ≥ 0.18: PASS (0.2168, baseline 0.143).
- Per-language ResolutionDisclosure with machine-readable SkipReason:
  PASS (5 rows: rust layer2 / ts+py subprocess_lsp / js server_missing /
  apex policy_disabled).
- F2 MCP routing regression-tested: PASS (2 tests).
- Workspace tests green: PASS (re-run at this commit).

**Plan 01 is COMPLETE. Plan 02 is unblocked.** Remaining residuals carried
into plan 02: 298 in-repo callee misses (~13% of a 30-sample still
plan01-flavored — constant/static refs, small enough to ride along),
proc-macro/macro-expansion coverage, and the rust-scip cross-check that
sets the Rust self-scan G1 numeric bar.
