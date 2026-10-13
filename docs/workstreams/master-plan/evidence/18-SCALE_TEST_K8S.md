# 18 — Scale test: Kubernetes (3.5M lines of Go)

Date: 2026-10-12. Harness, tasks and raw runs: `tests/p2-token-study/`
(`run_scale.py`, `tasks_scale.json`, `results/scale_k8s/`).

## Can GridSeak index it?

Kubernetes v1.36.0, 3,546,649 lines of Go outside `vendor/`.

| Step | Result |
|---|---|
| scip-go 0.2.7 index | 102 s, 3.3 GB RSS, 268 MB index |
| GridSeak full scan (release build) | 11.5 min, 6.6 GB RSS; 172,602 nodes, 258,385 edges; 100% of call edges compiler-tier |
| Callers of v1 `GetPodCondition` vs go/types | 23/23 files |
| Callers of v1 `IsPodReady` vs go/types | 27/27 files (GridSeak found one real caller the oracle missed: `test/e2e/network/service.go`) |

Getting there required four fixes, all committed: memoized fuzzy path
lookups (the reference sweep rescanned every file per miss), a submodule
index for glob imports, an opt-out for heuristic import bindings
(`GRIDSEAK_SKIP_IMPORT_HEURISTICS`, O(files × imported symbols) — still to
be rebuilt), and Go import-path symbol lookup
(`k8s.io/kubernetes/pkg/api/v1/pod.GetPodCondition`).

## Does it help the agent?

4 tasks (2 "which files call X", 2 "rename X and fix every caller"), two
same-named functions in different packages for each target, 2 runs per arm,
Opus 5.5, fresh copy-on-write clone per run. Edits are verified by
`go build` over the root module plus `go vet` (type-checks tests) of every
affected package, and by checking the other same-named function is untouched.

| Arm | Correct | Mean cost | Mean wall time | GridSeak answers received |
|---|---:|---:|---:|---:|
| A plain Claude Code | 8/8 | $0.164 | 62 s | — |
| D GridSeak first | 8/8 | $0.268 | 50 s | 8/8 |

Both arms were always right. With a correct GridSeak answer in hand, the
agent still re-checked with grep and used the compiler as its loop, so D
cost 63% more and was 19% faster. Total cost $3.46 (+$2.00 in 8 invalid D
runs, below).

## Invalid first D attempt (kept in `results/scale_k8s_invalid_D/`)

The first 8 D runs never received a GridSeak answer: run copies made with
`cp -cR` reset every file's mtime, GridSeak's freshness check marked all
12k+ files dirty, and both `workspace_delta` and `incremental_plan` listed
every path, producing a ~2.4 MB response that Claude Code rejected. Fixed in
the harness (`cp -cpR`) and in the product (paths capped at 25 plus a
count; callers now carry file and line).

## Reading

Across 3.5M lines the result matches P2 (≤ 112k lines): the agent's own
grep + compiler loop is already exact on typed code, so a precomputed graph
adds cost, not accuracy, for single renames and caller lists. Untested:
untyped languages at this scale, cross-repository changes, and long
sessions where the agent's context fills.
