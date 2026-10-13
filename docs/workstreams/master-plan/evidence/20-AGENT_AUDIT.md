# 20 — `gridseak audit`: agents graded against compiler truth on five repos

Date: 2026-10-12/13. Command: `gridseak audit` (`gridseak-cli/src/audit.rs`,
`audit_edit.rs`, `audit/scope.rs`). Raw results, tasks, answer keys and every
agent reply are kept per audit directory (`results.json`, `tasks.json`,
`t*-<arm>-r*.json`).

## What the audit does

- Plans tasks from the scan's call graph: "list every function that
  transitively calls X through direct calls", small (8–30 callers), medium
  (31–120) and large (121–400). Refuses to run unless every call edge came from
  the compiler index; states in each question which code the compiler covers.
- Runs each task in a fresh headless Claude Code session, alone ("plain") and
  with GridSeak's MCP tools ("gridseak"), read-only tools plus the language
  toolchain. Grades recall and precision against the key; pass = recall ≥ 0.95
  and precision ≥ 0.9. A miss is **silent** when the reply presents the list as
  complete.
- Edit family (`--family edits`, Go): change an interface method's signature
  and update every implementation and caller; graded by hidden compile-time
  assertions, `go build` and `go vet` in a throwaway copy.

## Are the answer keys right?

Checked against independent oracles built on each language's own type checker:

| Repo | Keys checked | Exact |
|---|---:|---:|
| Caddy (Go, go/types) | 12 targets (two plans) | 12/12 |
| chi (Go, go/types) | 8 targets (two plans) | 8/8 |
| zod (TS, TypeScript checker) | 15 → 17 candidate targets | ~~15/15~~ invalid, see correction; 17/17 after fix |
| hono (TS, TypeScript checker) | 73 candidate targets | ~~73/73~~ 71/73 with the fixed oracle; 73/73 after fix |
| Kubernetes (Go) | not re-checked transitively (direct callers matched go/types file-for-file in `18`) | — |

Getting there fixed real defects: TS function nodes for class fields, private
and keyword-named methods; node-id collisions between identical method bodies;
calls misread as value uses; `super()` edges; and the question scope (Go
`_examples/`, files the TS project excludes, files tree-sitter only partly
parses).

## Results — caller questions

Go (Caddy ×3 repeats, chi, Kubernetes; 6 tasks per repo per model):

| Model | Alone: exact | Alone: silent misses | + GridSeak: exact | + GridSeak: silent | $ per correct, alone → GridSeak | Minutes, alone → GridSeak |
|---|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 | 25/30 | 0 | **30/30** | 0 | 1.86 → 0.90 | 99 → 51 |
| Sonnet 5.5 | 24/30 | 5 | **30/30** | 0 | 0.45 → 0.14 | 50 → 18 |
| Opus 5.5 | 29/30 | 1 | **30/30** | 0 | 0.66 → 0.27 | 58 → 26 |
| **All** | **78/90** | **6** | **90/90** | **0** | **0.98 → 0.44** | **208 → 96** |

Tokens processed: 59.3M alone vs 22.5M with GridSeak (2.6× fewer).

**Correction (2026-10-13): the TypeScript results below were graded against wrong answer keys.** The first TS oracle shared two blind spots with GridSeak — string-keyed element-access calls (`this["~validate"](x)`) and `new X()` reaching an inherited constructor — so both agreed on keys that missed real callers (zod `isAsync`: 236 true callers, the key had 92). Both are fixed and the keys now match the fixed oracle exactly (zod 17/17, hono 73/73). The TS runs are being re-done; the numbers below are kept only as a record and must not be quoted.

~~TypeScript (zod, hono; Sonnet and Opus): alone 15/22 (7 silent), with
GridSeak 18/22 (3 silent); $0.80 → $0.42 per correct answer. The GridSeak-arm
misses trace to three product gaps, being fixed: MCP rows included synthetic
`<file scope>` pseudo-callers (an agent listed them); the tools could not
resolve a class-field function with a common name (`fetch`), so the agent fell
back to grep; and string-keyed element-access calls (`this["~validate"](x)`)
are not recorded as calls — Opus spotted this one itself and flagged its
answer as unverified. One hono miss was the agent merging two distinct test
functions with the same name.~~

## Results — edit tasks (Go, Caddy)

4 interface changes (1–12 implementations), plain arm: Haiku, Sonnet and Opus
each 4/4 (build, vet, signature all pass), $0.07–0.55 per task. Consistent with
`19`: compile-checked edits are easy for current agents; they are a baseline,
not where agents fail.

## What this changes

- `19` found that plain agents solve single caller questions when they can
  write code. Across a spread of tasks and repos, plain agents are wrong on
  about 1 in 7 Go questions, and **half of those wrong answers are presented as
  complete**. With GridSeak no Go answer was wrong.
- Model choice matters as much as tools: Sonnet + GridSeak gave the cheapest
  correct answers on every Go repo ($0.04–0.16 each), while plain Haiku was the
  most expensive and least accurate.
- The audit itself is the product surface: a per-repo, per-model card with
  compiler-checked answers, silent-miss counts and cost per correct answer.

## Spend

About $139 at API prices for the audits in this doc (Claude Code's per-run
figure), billed to the claude.ai subscription.

## Limits

One task family where agents fail (transitive callers); edits are easy. Go
results rest on 3 repos; TypeScript on 2 repos and 2 models. Caddy has 3
repeats per cell; other repos 1. Read-only tools (agents could not write
helper programs; `19` showed that closes much of the gap at 3–7× the time).
Kubernetes transitive keys were not re-checked against go/types. Pass-rate
ranges per cell are wide (Wilson 95%, e.g. 30/30 → 89–100%).
