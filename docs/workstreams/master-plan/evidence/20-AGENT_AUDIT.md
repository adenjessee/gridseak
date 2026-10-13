# 20 — `gridseak audit`: agents graded against compiler truth on five repos

Date: 2026-10-12/13. Command: `gridseak audit` (`gridseak-cli/src/audit.rs`,
`audit_edit.rs`, `audit/scope.rs`). Per-repo results, tasks with answer keys
and report cards: `tests/audit-results/<repo>/` (paths sanitised). 378 graded
caller-question runs plus 12 edit runs.

## What the audit does

- Plans tasks from the scan's call graph: "list every function that
  transitively calls X through direct calls", small (8–30 callers), medium
  (31–120) and large (121–400). Refuses to run unless every call edge came from
  the compiler index, and states in each question which code the compiler
  covers.
- Runs each task in a fresh headless Claude Code session, alone ("plain") and
  with GridSeak's MCP tools ("gridseak"), read-only tools plus the language
  toolchain. Grades recall and precision against the key; pass = recall ≥ 0.95
  and precision ≥ 0.9. A miss is **silent** when the reply presents the list as
  complete; a reply that warns its list may be incomplete is "flagged". (The
  first version of this detector missed plain-worded warnings such as "I
  couldn't produce the complete list" and overcounted silent misses; every
  miss was re-graded and reviewed by hand.)
- Edit family (`--family edits`, Go): change an interface method's signature
  and update every implementation and caller; graded by hidden compile-time
  assertions, `go build` and `go vet` in a throwaway copy.

## Are the answer keys right?

| Repo | Oracle | Keys checked | Exact |
|---|---|---:|---:|
| Caddy (Go) | go/types | 12 targets (two plans) | 12/12 |
| chi (Go) | go/types | 8 targets (two plans) | 8/8 |
| zod (TS) | TypeScript checker (fixed oracle) | 17 eligible targets | 17/17 |
| hono (TS) | TypeScript checker (fixed oracle) | 73 eligible targets | 73/73 |
| Kubernetes (Go) | — | not re-checked transitively; direct callers matched go/types file-for-file in `18` | — |

**Correction made during this work.** The first TypeScript oracle shared two
blind spots with GridSeak: string-keyed element-access calls
(`this["~validate"](x)`) and `new X()` reaching an inherited constructor. Both
agreed on keys that missed real callers (zod `isAsync`: 236 true callers, the
first key had 92). Both were fixed; the first TypeScript results were discarded
and re-run. Go keys never depended on that oracle.

Other defects fixed on the way: TS function nodes for class fields, private and
keyword-named methods; node-id collisions between identical method bodies;
calls misread as value uses; `super()` edges; MCP rows listing `<file scope>`
pseudo-callers; `file:line` symbol lookup for common names; question scope (Go
`_examples/`, files the TS project excludes, partly parsed files).

## Results — caller questions

Go: Caddy, chi and Kubernetes, 6 tasks each, 3 repeats per cell (162 runs per arm):

| Model | Alone: exact | Alone: silent | + GridSeak: exact | + GridSeak: silent | $ per correct (alone → GridSeak) | Minutes | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 | 45/54 | 0 | **54/54** | 0 | 1.65 → 0.74 | 160 → 78 | 42.3M → 21.0M |
| Sonnet 5.5 | 47/54 | 4 | **54/54** | 0 | 0.41 → 0.12 | 89 → 30 | 27.7M → 5.5M |
| Opus 5.5 | 51/54 | 3 | **54/54** | 0 | 0.58 → 0.25 | 100 → 45 | 24.3M → 7.0M |
| **All** | **143/162** | **7** | **162/162** | **0** | **0.86 → 0.37** | **349 → 153** | **94.4M → 33.5M** |

Per repo (all models): Caddy 47/54 → 54/54, chi 49/54 → 54/54, Kubernetes
47/54 → 54/54.

TypeScript: zod (3 tasks) and hono (6 tasks), one run per cell, corrected keys:

| Model | Alone | + GridSeak | $ per correct (alone → GridSeak) |
|---|---:|---:|---:|
| Haiku 5.5 | 7/9 (1 silent) | 8/9 (1 silent) | 2.98 → 1.30 |
| Sonnet 5.5 | 5/9 (4 silent) | **9/9** | 0.71 → 0.08 |
| Opus 5.5 | 7/9 (2 silent) | **9/9** | 0.77 → 0.24 |
| **All** | **19/27 (6 silent)** | **26/27 (1 silent)** | **1.57 → 0.51** |

The one GridSeak-arm miss: Haiku received GridSeak's certified 51-caller list
for hono `escapeToBuffer`, then removed 9 correct callers it judged did not
count (calls inside an arrow wrapped in an `as` cast). The key matches the
TypeScript checker. A weaker model can argue itself out of a certified answer.

## Results — edit tasks (Go, Caddy)

4 interface changes (1–12 implementations), plain arm: Haiku, Sonnet and Opus
each 4/4 (build, vet, signature all pass), $0.07–0.55 per task. Consistent with
`19`: compile-checked edits are easy for current agents; they are a baseline,
not where agents fail.

## What this changes

- `19` found that plain agents solve a single caller question when they can
  write code. Across a spread of tasks, repos and repeats, plain agents with
  read-only tools are wrong on about 1 in 8 Go questions, and more than a third
  of those wrong lists are presented as complete (7 of 19; the rest were
  flagged as incomplete or not answered). With GridSeak, no Go answer was
  wrong in 162 runs.
- Cost per correct answer fell 2.3× (Go) and 3.1× (TS); time 2.3× and 2.6×;
  tokens 2.8× and 3.1×. Sonnet + GridSeak gave the cheapest correct answers on
  every repo.
- The audit is the product surface: a per-repo, per-model card with
  compiler-checked answers, silent-miss counts and cost per correct answer.

## Spend

About $250 at API prices for all audits behind this doc, including the
discarded first TypeScript runs (Claude Code's per-run figure), billed to the
claude.ai subscription.

## Limits

One task family where agents fail (transitive callers); edits are easy.
Read-only tools: agents that may write helper programs close much of the gap at
several times the time (`19`). TypeScript has one run per cell on two repos.
Kubernetes transitive keys were not re-checked against go/types. Pass-rate
ranges per cell are wide (Wilson 95%: 54/54 → 93–100%; 162/162 → 98–100%).
