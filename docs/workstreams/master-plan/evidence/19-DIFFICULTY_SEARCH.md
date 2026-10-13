# 19 — Difficulty search: Families 1 and 2 (result)

Date: 2026-10-12. Goal: find generated tasks with deterministic graders
where models pass 2–70%.
Harness, tasks, truths and every raw run: `tests/difficulty/`.

## Setup

- Claude Code headless (`claude -p --restricted`), fresh copy-on-write clone
  per run, same prompt per task. Models Opus 5.5 and Haiku 5.5.
- **Family 1 — interface change.** "Add `ctx context.Context` as the first
  parameter of `I.M`; update every implementation, fake and caller." Graded
  by hidden compile-time assertions (`var _ I = T{}` for every implementation,
  written after the run) plus `go build` / `go vet`. 2 caddy + 4 Kubernetes
  interfaces with up to 5 implementations in 5 packages.
- **Family 2 — exhaustive transitive callers.** "List every function that
  transitively calls X through direct calls (not interface dispatch, not
  function values), including tests." Truth: the go/types oracle's reverse
  closure. Pass = recall ≥ 0.95 and precision ≥ 0.9. 4 caddy targets with
  25, 60, 157 and 282 transitive callers.
- Arms: **A** plain, read-only tools plus `go`; **D** A plus GridSeak MCP
  (`blast_radius`, `callers`), told to use it first; **W** A plus permission
  to write and run its own helper programs in `./.scratch/`.

## Product change made for Family 2

Call edges now record how the callee is used: `call`, `dispatch` (through an
interface method) or `value` (passed or stored). `blast_radius` takes
`uses: all | calls | static`. With `static`, GridSeak's closure equals the
go/types oracle exactly on all 4 targets (recall and precision 1.000).
Caddy: 5,096 call, 695 dispatch, 151 value edges.

## Result

Family 1 (one run per cell): **12/12 pass** (Opus 6/6, Haiku 6/6). Too easy;
no arm comparison was run.

Family 2, one run per cell:

| Model | Arm | Pass | API-price $ | Wall time |
|---|---|---:|---:|---:|
| Opus 5.5 | A plain | 4/4 | 4.72 | 52 min |
| Opus 5.5 | D GridSeak | 4/4 | 3.22 | 34 min |
| Haiku 5.5 | A plain | 3/4 | 9.82 | 25 min |
| Haiku 5.5 | D GridSeak | 4/4 | 6.50 | 12 min |

Haiku on the 282-caller task, 4 runs per arm:

| Arm | Pass | Mean $ | Mean time | Slowest run |
|---|---:|---:|---:|---:|
| A plain, read-only | 2/4 | 2.34 | 6.0 min | 8 min |
| W plain, may write code | 4/4 | 2.40 | 14.0 min | 36 min |
| D GridSeak | 4/4 | 2.52 | 4.9 min | 6.4 min |

**Plain reading:** Opus answers a 282-function transitive closure correctly
with grep. Haiku fails it half the time when it cannot write code: in the one
failed run whose reply was kept it said it needed a type-aware analysis it
could not run, and returned 2 verified callers as a lower bound. Allowed to
write code, Haiku built a go/types call-graph walker (the same method as the
oracle) and was exact 4/4, in two runs checking across several GOOS/GOARCH
targets. GridSeak's remaining edge is time and predictability (4–6 min every
run vs 5–36 min), not accuracy or cost.

## What this means for the pivot

- Neither family hits the 2–70% band for a realistically equipped agent. The
  only in-band result (Haiku 2/4) comes from denying the agent the ability to
  write code, which is not how agents are used.
- Consistent with P2 (`17`) and the Kubernetes scale test (`18`): on questions
  about the code as it is, a capable agent with a compiler rebuilds what
  GridSeak computes, on demand. Answer accuracy is not the product.
- The untried direction in the plan is checking an agent's *output*
  (changes before merge) against compiler truth, where a wrong change costs
  more than a slow answer.
- In D runs the agent called GridSeak 2–4 times, then spent most of its turns
  re-verifying by hand. Agents do not yet trust a tool's answer enough to stop.

## Follow-up: certified answers (same day)

`blast_radius` now returns a `certificate`: `complete` is true only when every
call edge in the scan came from the compiler index, no source file changed
since the scan, and the walk hit neither the depth limit nor the row cap;
otherwise `reasons` names each failed check. Rows carry repo-relative file and
line. Haiku, 282-caller task:

| Arm | Pass | Mean $ | Mean time | Mean turns |
|---|---:|---:|---:|---:|
| D GridSeak (before) | 4/4 | 2.52 | 4.9 min | 20 |
| T GridSeak, certified | 4/4 | 1.57 | 3.1 min | 20.5 |
| B GridSeak, scan without compiler index | 0/1 | 1.44 | 2.9 min | 21 |

- Cost fell 38% and time 37% with the same accuracy, but turns did not fall:
  agents still spot-checked (closures, interface exclusion, build-tagged
  files). The saving came from fewer output tokens (no per-caller lookups).
- On the uncertified scan the agent read "0 of 2189 call edges are
  compiler-verified", did not use the result, and reported its answer as
  incomplete rather than presenting 18 callers as the full list.
- 4 runs (T) and 1 run (B); Haiku only.

## Spend

API-price equivalent (Claude Code's per-run figure), billed to the claude.ai
subscription, not API credits: Family 1 $7.50, Family 2 $48.76; total about
$56 of the plan's $50–150 Phase 1 budget. `tests/difficulty/spend.py` keeps
the ledger.

## Limits

One run per cell except the 282-caller task (4 per arm). Go only; one repo
for Family 2. Only Opus and Haiku (no Sonnet). Replies were kept only from
the Haiku repeats onward, so the first failed run's reason is unknown. The
"read-only" condition was a harness choice inherited from P2, not a property
of the task.
