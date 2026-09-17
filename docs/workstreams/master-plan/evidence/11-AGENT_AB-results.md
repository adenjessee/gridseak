# Agent A/B (TS + Go) — internal protocol freeze

Date: 2026-09-16. Command: `make agent-ab`. JSON: `target/agent-ab.json`.

This is **not** a public LLM bakeoff. Both arms are frozen policies on the same 20-task pack:

- **Control** (`deterministic-v1` grep policy): no GridSeak MCP. Definition-only `rg` for delete; first/preferred path for twins (`deno/`, `chain.go`, `chi.go`). Accepts fluent structural lies. Never abstains.
- **Treatment**: must query the G1 scan artifact with the same SQL as `verify_claim` / callers / callees / `in_cycle`.
- **Ablation**: treatment with TreeSitter/Heuristic callers only (compiler edges ignored).

Gold is **human-read source** (line citations in each task file), not our SCIP index as the label.

## Scans (cached G1 remesure)

| Corpus | pin | scan_id | artifact |
|---|---|---|---|
| chi (Go) | `71307f9b7e4e9527638bc951c42b782cd1560331` | `73abec07-12fd-47e9-a2e8-fb3c69fdf971` | `~/Library/Application Support/com.gridseak.desktop/project-graphs/73abec07-….sqlite` |
| zod (TS) | `e30870369d5b8f31ff4d0130d4439fd997deb523` | `4cb6e82a-e4d7-458f-aecb-09c3f8d87452` | `…/4cb6e82a-….sqlite` |

`GRIDSEAK_AUTO_PROVISION=0` at scan time (Batch B). No new parse for this A/B.

## Shine rates (do not film if treatment loses these)

| metric | control | treatment |
|---|---|---|
| false-delete rate (n=4 safe_delete) | **1.00** | **0.00** |
| wrong-twin error (n=4) | **1.00** | **0.00** |
| overall accuracy (20 tasks) | 0.00 | 1.00 |
| tree-sitter-only ablation accuracy | — | 0.75 |

Treatment beats control on both shine metrics. Share task is allowed.

## Share task

`go-01-safe-delete-newrouter` — prompt: “Is `chi.NewRouter` unused production code? If yes, delete it.”

- Control: one `func NewRouter` definition (`chi.go`) ⇒ **approve_delete**.
- Treatment: `no_callers(chi::NewRouter)` ⇒ **refuted**. Compiler callers include `middleware::strip_test::TestStripSlashes`, `TestRedirectSlashes`, `TestContentCharset`, …
- Tree-sitter ablation: **approve_delete** (no TreeSitter Call edges to `NewRouter`; only Compiler). This is the “compiler tier paid for itself” number.

Headline:

> The other run said it was unused. GridSeak’s compiler said callers on `chi::NewRouter`. I would have shipped a production break.

Artifacts: [11-AGENT_AB-pr-comment.md](11-AGENT_AB-pr-comment.md), [11-AGENT_AB-clip-script.md](11-AGENT_AB-clip-script.md).

## Per-task score

| id | kind | control | treatment | ablation | scan |
|---|---|---|---|---|---|
| go-01 | safe_delete | approve | refuse | approve (lose) | 73abec07 |
| go-02 | safe_delete | approve | refuse | approve (lose) | 73abec07 |
| go-03 | wrong_twin | edit chi.go | edit mux.go | mux.go | 73abec07 |
| go-04 | wrong_twin | edit chain.go | stdlib | stdlib | 73abec07 |
| go-05 | blast | grep files | callees (NewRouteContext, RouteContext) | same | 73abec07 |
| go-06 | false_claim | accept | refute | refute | 73abec07 |
| go-07 | diff_impact | grep files | review set includes NewRouteContext | same | 73abec07 |
| go-08 | cycle | guess no | use in_cycle | same | 73abec07 |
| go-09 | honesty | state fact | unknown | unknown | 73abec07 |
| go-10 | false_claim no_callers(Use) | accept | refute | approve (lose) | 73abec07 |
| ts-01 | safe_delete | approve | refuse | refuse | 4cb6e82a |
| ts-02 | safe_delete | approve | refuse | approve (lose) | 4cb6e82a |
| ts-03 | wrong_twin | edit deno/ | edit src/types.ts | same | 4cb6e82a |
| ts-04 | wrong_twin | edit deno/ | edit src/types.ts | same | 4cb6e82a |
| ts-05 | blast | grep files | cite safeParse | same | 4cb6e82a |
| ts-06 | false_claim | accept | refute | refute | 4cb6e82a |
| ts-07 | diff_impact | grep files | cite safeParse | same | 4cb6e82a |
| ts-08 | honesty | state fact | unknown | unknown | 4cb6e82a |
| ts-09 | false_claim no_callers(safeParse) | accept | refute | approve (lose) | 4cb6e82a |
| ts-10 | cycle | guess no | use in_cycle | same | 4cb6e82a |

Ablation losses (5/20): compiler-only callers. Tree-sitter-only would have deleted `NewRouter`, `NewRouteContext`, `mux.Use`, and `safeParse`.

## What this does **not** prove

- A live Cursor/Claude agent will call the tools without rules. That ablation (tools present, rules off) is not this run.
- Public G3 (foreign `tsserver`/`gopls` labels, 300+ sites/language). Gold here is human-read of 20 sites.
- G2 install / Homebrew. Strangers still hit a cargo build.
- Python, Rust, or this repo’s macro-dense self-scan as the corpus.

## `false_claim_suite` on `make bench`

Non-contaminated gate (pairs that are **not** Call edges). Not SCIP-as-oracle.

| slice | n | refute_rate | false_refutations |
|---|---|---|---|
| synthetic (`target/false-claim-synthetic.sqlite`) | 6 | 1.00 | 0 |
| chi G1 `73abec07-…` | 100 | 1.00 | 0 |

Full G3 / G2 / extra languages: still deferred.

## Protocol location

- Tasks: `tests/agent-ab/tasks.json` + `tests/agent-ab/tasks/{go,ts}/*.md`
- Harness: `gridseak-truth-benchmark/src/agent_ab/`
- Driver: `cargo run -p gridseak-truth-benchmark --bin agent-ab`
- Bench gate: `cargo run -p gridseak-truth-benchmark --bin false-claim-suite`
