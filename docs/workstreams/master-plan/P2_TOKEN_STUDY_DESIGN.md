# P2 — Do code maps save an agent tokens and time? (study design)

Status: DESIGN, awaiting approval to spend API/plan usage. 2026-10-11.

## Question

On the structural questions coding agents ask while working, does giving
Claude Code a code map reduce tokens, time and cost, and is the answer still
right? Viral tools claim 71.5x / 80% savings without a correctness check;
independent tests found ~8%. Nobody has published tokens *and* accuracy
against a compiler-grade truth.

## Arms (same model, same prompt, fresh session per run)

| Arm | What the agent has |
|---|---|
| A. Baseline | Claude Code default tools (grep, glob, read, bash) |
| B. Graphify | A + `graphify-out/graph.json` and the `graphify` CLI (`explain`, `path`, `query`) |
| C. GridSeak | A + GridSeak MCP, restricted to 3 tools: callers, blast radius / affected tests, verify-claim |

Headless: `claude -p --output-format json` (usage and timing are in the
JSON), `--max-turns 40`, an `--mcp-config` only in arm C, no CLAUDE.md,
working copy reset between runs.

## Corpora

chi (Go) and zod (TypeScript) at the pinned commits — the only repos with
an independent type-checker oracle today (`tests/recall-audit/`).

## Tasks (30, fixed before running, half per corpus)

| Kind | n | Truth from the oracle | Why it matters |
|---|---:|---|---|
| "List every function that calls X" | 10 | exact caller set | refactors, reviews |
| "Can X be deleted without breaking anything in this repo?" | 10 | used / unused (mix includes value-only uses, interface dispatch, same-name twins, test-only uses, truly dead code) | deletions, cleanup |
| "Which test functions exercise X, directly or transitively?" | 10 | test functions on reverse call paths | P1: half of red CI rounds are test failures found late |

Agents end with a fenced JSON answer, which is parsed and scored.

## Metrics

- Tokens: input (split cached / uncached), output; dollar cost at list price.
- Wall time, number of tool calls, turns.
- Correctness: caller precision/recall/F1; delete verdict accuracy; test-set
  precision/recall.
- Headline: **cost per correct answer** per arm.

## Pre-registered hypotheses (published before the run, reported either way)

- H1: C uses ≥ 30% fewer tokens than A on caller and test tasks, with recall
  no lower than A.
- H2: C's delete verdicts are more accurate than A's and B's.
- H3: B uses fewer tokens than A but has lower caller recall than A or C.
- H0 is a publishable result too: "maps don't save much once the agent can
  grep" would also be news.

## Runs and cost

- Pilot: 3 tasks × 3 arms × 1 run = 9 runs, to verify parsing and tool use.
- Main: 30 tasks × 3 arms × 3 runs = 270 runs.
- Estimate (upper bound, Opus 5.5 list $4/$20 per M): ~100k input + ~3k
  output per run ≈ $0.46 uncached → pilot ≈ $5, main ≈ $125. Caching and a
  cheaper model lower this; on a Claude Code subscription it consumes plan
  usage instead. Exact cost is reported after the pilot before continuing.

## Timeline

Day 1: harness + task file + pilot. Day 2: main run. Day 3: analysis,
chart, write-up with the rerun command.

## Threats to validity

Two repos, two languages; one model; tool-description wording can steer use;
headless behavior differs from IDE use; the Graphify arm uses its local code
pass only.

## v2 (pre-registered 2026-10-12, before any v2 run)

Pilot result (v1): 9/9 correct in every arm; agents never opened the map;
$0.016-0.157 per question. Tasks were too easy (unique names, small repos).

v2 changes:
- Corpora add caddy v2.11.7 (112k LOC Go) and hono v4.13.13 (TS; root
  tsconfig replaced by the spec config covering `src/`, identical for all arms).
- 15 tasks (`tests/p2-token-study/tasks_v2.json`): callers of methods whose
  names are defined up to 102 times; files calling interface/class methods
  with 6-8 same-name definitions; deletions whose truth is **the compiler**
  (delete, `go build && go vet` / `tsc --noEmit` new errors), including
  interface-satisfaction, value-only and package-level uses; one test task
  whose truth is **per-test runtime coverage**.
- Arm D = arm C plus an instruction to use the GridSeak tools first.
- 2 runs per task per arm = 120 runs, Opus 5.5, `--restricted`.

Hypotheses (unchanged in spirit, now testable):
- H1: on caller/file tasks, C or D reach higher recall than A at equal or
  lower cost.
- H2: on deletion tasks, all arms' accuracy is bounded by interface
  contracts; GridSeak's "no callers" is wrong for interface methods
  (measured: 20/30 zero-caller caddy methods break the build when deleted).
- H3: unprompted (C), the agent rarely calls GridSeak tools; prompted (D)
  changes cost and accuracy.
- H4: B (Graphify) shows no accuracy gain over A.
