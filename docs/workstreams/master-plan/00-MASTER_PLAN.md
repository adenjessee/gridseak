# 00 — MASTER PLAN: GridSeak from invisible alpha to trusted foundation

Status: DRAFT v1 (2026-07-01)
Owner: Aden Jessee
Audience: any agent or human executing sub-plans 01–09. Read this file first.
Each sub-plan in this folder is self-contained and can be executed by a
small LLM without reading the others; this file explains how they fit.

## 1. The objective, in one paragraph

GridSeak becomes the deterministic structural-truth engine that AI agents
and developers rely on to answer "who calls X, what breaks, what's risky"
with **calibrated honesty** — every answer carries evidence tiers and
confidence that is measurably accurate. Adoption is driven by artifacts
that spread without a founder audience: a public benchmark that owns the
category's measuring stick, a public observatory of scanned OSS repos, a
report card/badge every scan produces, and registry-native distribution
aimed at agents (machines) rather than social followers. Nothing scales
until accuracy gates pass, because inaccurate scale destroys the one
asset GridSeak has: trustworthiness.

## 2. Verified current state (evidence, not vibes)

These facts were established 2026-07-01 by (a) a skeptical repo audit and
(b) dogfooding the GridSeak MCP on this repo itself. Do not re-litigate
them; re-verify them only after changes claim to fix them.

- **F1 — The semantic tier is effectively absent in production scans.**
  Fresh MCP-triggered self-scan `7d3f8035-e63e-438b-b440-0314570b22d8`
  (2026-07-02T03:02Z, 85.6s, primary language rust) reports
  `resolution_quality.measured_fidelity.tier = "syntactic_only"`,
  `high_ratio_on_calls = 0.143`. Call edges by confidence:
  high 3,772 / medium 97 / low 22,440. Rust is the ONLY language
  `LIMITATIONS.md` claims as production Tier 3, and on our own repo the
  rust-analyzer Layer-2 path contributed nearly nothing via the MCP scan
  path — and the report contains **no telemetry section explaining why**.
- **F2 — MCP default-project routing is wrong.** `gridseak_context_for_llm`
  with default `project: "."` returned cached data for an unrelated repo
  (`pg-meta`, on ~/Desktop/supabase) instead of the workspace repo. An
  agent can silently receive the wrong codebase's answers.
- **F3 — Apex validation shows 0 LSP edges** and 82–94% heuristic
  fallback on all four public corpora (docs/workstreams/apex/VALIDATION_RESULTS.md),
  and `tests/fixtures/apex_baseline/lsp_thresholds.json` gates are all 0.
- **F4 — Distribution is mechanically broken.** Release workflow builds
  x86_64 only (no Apple Silicon), core crates are `publish = false`, no
  Homebrew/npm, no MCP registry/directory listings.
- **F5 — The market validates the problem but not our position.**
  Competitors (CodeGraph ~tree-sitter+SQLite, Serena ~live LSP, 25k
  stars) won attention with proof artifacts (benchmark claims, articles)
  and zero-friction installs. None of them has GridSeak's provenance/
  confidence/tier model. GitHub stack-graphs (universal precise
  resolution without compilers) was archived 2025-09 — the "99% everywhere
  by hand" path is a graveyard; delegating precision to compilers (SCIP,
  rust-analyzer-as-library) is alive and maintained.
- **F6 — The repo is a serious alpha** (~125k LOC prod Rust, 1,717 tests,
  14-tool MCP, honest LIMITATIONS.md) with over-breadth: 9 languages
  claimed / 1 deep, ~20k LOC Apex bet half-proven, 73k lines of markdown,
  two MCP server implementations, an 8k-LOC shelved visual app.
- **F7 — Batch compiler indexers cover every stable target language**
  (verified 2026-07-01): scip-go, scip-typescript, scip-python,
  scip-java (Java/Scala/Kotlin), scip-clang, scip-ruby, scip-dotnet, and
  Rust natively via `rust-analyzer scip`. LSP-vs-index is the same
  compiler front-ends behind two access patterns; the batch artifact
  pattern eliminates the subprocess failure modes we measured. This is
  why plan 02 v2 makes batch indexing the primary engine and demotes
  subprocess LSP to no-indexer languages (Apex).

## 3. Strategy in one sentence per pillar

1. **Truth first**: make the measured fidelity real (batch compiler
   indexes as the PRIMARY semantic engine — subprocess LSP is demoted to
   no-batch-indexer languages only, see plan 02 v2 routing policy), make
   every skip/fallback loud, and never report what we cannot prove.
2. **Own the measuring stick**: publish the category's first reproducible
   accuracy+calibration benchmark, scored against competitors, including
   where we lose.
3. **Artifacts are the distribution**: benchmark, observatory, report
   cards, badges, videos — things that spread with zero founder audience.
4. **Agents are the customer**: registry-native, one-command install,
   machine-readable quality; humans follow the machines.
5. **Scale only behind gates**: accuracy gate → distribution gate →
   volume (observatory, videos, launches).

## 4. Sub-plans and execution order

| # | File | What it delivers | Depends on |
|---|------|------------------|------------|
| 01 | 01-TRUTH_RECOVERY.md (**COMPLETE 2026-07-02**) | Diagnose+fix why semantic tier is skipped in production scans; no-silent-fallback contract; fix MCP project routing | — |
| 02 | 02-COMPILER_TIER.md | Batch semantic indexing as PRIMARY engine (replaces subprocess LSP for all batch-indexed languages); Compiler provenance + authority ladder + witness fusion; universal SCIP ingester (TS, Python, Rust unification) | 01 |
| 03 | 03-DISTRIBUTION_UNBLOCKERS.md | Apple Silicon builds, registries, install paths, repo hygiene, brand separation | — (parallel with 01/02) |
| 04 | 04-TRUTH_BENCHMARK.md | Public reproducible benchmark: precision/recall/token-cost/calibration vs competitors | 02 (to be winnable), 03 (to be citable) |
| 05 | 05-OBSERVATORY.md | Mass public-repo scan pipeline + public stats site (human- and agent-readable) | Gate G1, 03 |
| 06 | 06-ARTIFACTS_AND_BADGES.md | Scan report card, README badge, public gallery pages | Gate G1 |
| 07 | 07-VIDEO_ENGINE_AND_BRANDING.md | Flagship founder video + AI-generated per-repo video review pipeline | Gate G1, 05 |
| 08 | 08-CONTRIBUTOR_ENGINE.md | Language-adapter port + "add a language" guide + launch posts | 02, 04 |
| 09 | 09-FEEDBACK_AND_MEASUREMENT.md | Adoption telemetry, credibility-weighted feedback triage, AI-run monitoring loop | 03 onward, continuous |

Parallelism: 01+03 start immediately and in parallel. 02 starts as soon
as 01's diagnosis lands. 04 starts when 02's TS or Rust milestone passes.
05–08 are gated. 09 runs forever.

## 5. Gates (hard, measurable, no exceptions)

- **G1 — Accuracy gate** (exit of plans 01+02; AMENDED 2026-07-01 after
  UF-FU-012 evidence showed the Rust-Layer-2 ceiling on this
  proc-macro-dense repo is ~10–20% without macro-expansion coverage):
  - Plan 01 interim bars (owned by plan 01's amended criteria): Layer-2
    active via MCP path; adapter-resolved→emitted conversion >= 0.90;
    self-scan `high_ratio_on_calls >= 0.18` (from 0.143 baseline).
    **PASSED 2026-07-02** — conversion 0.9501 (mappable universe, see
    plan 01 AMENDMENT NOTE 2), high_ratio 0.2168, report
    `3755b582-1d49-459e-966f-6d2737b21dac`; final adjudication in
    evidence/01-adjudication.md.
  - `high_ratio_on_calls >= 0.60` measured on at least one language with
    COMPLETE compiler-tier coverage end-to-end: the TS/SCIP corpus
    (plan 02 Phase C), or Rust via `rust-analyzer scip` if plan 02's
    cross-check proves macro-expanded call-site coverage.
  - Rust SELF-SCAN numeric target is set empirically after plan 02's
    rust-scip cross-check answers the macro-coverage question; record
    the chosen bar and its evidence here when set.
  - Zero silent fallbacks: every scan report enumerates, per language,
    which resolution tier ran, and if a semantic tier did not run, a
    machine-readable reason (enum, not prose) is present.
  - MCP project-routing bug (F2) fixed with a regression test.
- **G2 — Distribution gate** (exit of plan 03): a stranger on an Apple
  Silicon Mac installs with one command and completes an MCP-connected
  scan in under 5 minutes; listed on >= 3 registries/directories.
- **G3 — Proof gate** (exit of plan 04): benchmark published, reproducible
  by `git clone && make bench`, with GridSeak calibration error <= 0.10
  and per-language precision/recall published even where we lose.
- **G4 — Scale gate** (entry to 05/07 volume): G1+G2+G3 all pass.

## 6. What "success" is measured by (plan 09 owns the dashboard)

Leading: registry listings live, install completions (telemetry opt-in),
benchmark repo stars/citations, observatory page visits, MCP tool-call
volume from external users. Lagging: unsolicited mentions in comparison
articles, contributor PRs on language adapters, credible-source feedback
items processed. Anti-metric: any public claim we cannot reproduce.

## 7. Standing rules for every executing agent

- Never cut corners; never overstate results. When a number is measured,
  cite the artifact (scan id, report path, benchmark run id).
- Every plan's "Acceptance criteria" are binding. If a criterion cannot
  be met, stop and report exactly what blocked it; do not redefine it.
- Keep clean architecture: new logic goes in purpose-built modules;
  ports (traits) before adapters; no god-files.
- The tier/provenance vocabulary is the product. Any new feature that
  emits a structural claim must carry source + confidence, and any
  consumer-facing surface must be able to display it.
- Update this file's Status line and the gate table when gates pass,
  with the evidence artifact linked.
