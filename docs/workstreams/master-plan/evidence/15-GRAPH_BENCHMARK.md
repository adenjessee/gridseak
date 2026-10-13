# 15 — Code-graph accuracy: Graphify vs. GridSeak

Date: 2026-10-11. Harness: `tests/graph-benchmark/`. Oracles: Go `go/types`,
TypeScript checker (`tests/recall-audit/`). Same corpora, same pins, same
scoring for both tools.

- Graphify `graphifyy` 0.9.86 (PyPI), `graphify update .` — its local,
  no-LLM tree-sitter code pass. Edges counted: `relation == "calls"`.
- GridSeak at this commit, SCIP compiler tier.
- Unit: (calling function → called function), matched by declaration line.

| | chi (Go) Graphify | chi (Go) GridSeak | zod (TS, `src/`) Graphify | zod (TS, `src/`) GridSeak |
|---|---:|---:|---:|---:|
| Call edges found (of truth) | **48.2%** | **100%** | 53.7% | 81.2%* |
| Call edges found, only where the tool has nodes at both ends | 48.2% | 100% | **73.3%** | **100%** |
| Function-value uses found | 0% | 100% | 0% | 67.6%* |
| Used functions shown as having ≥ 1 caller | **61.7%** | **100%** | **60.2%** | **99.1%** |
| Used functions with *every* caller listed | 50.6% | 100% | 40.7% | 71.2%* |
| Extra edges with no explanation | 46 | 0 | 64 | 35 |

\* Declaration-line matching artifact: all 100 GridSeak zod misses here are
truth edges whose caller or callee line has no node *starting* on that line
(multi-line declarations, nested callbacks); 0 are missing edges. With
position-based matching (`13-RECALL_AUDIT.md`) GridSeak zod call recall is
99.7% and every-caller-listed is 98.5%.

**Plain reading:** asked "is this function used?", the local Graphify graph
answers "nobody calls it" for about **4 in 10 functions that are in fact
called** (chi 38%, zod 40%). An agent or reviewer that trusts the map would
treat those as dead code.

Graphify's other relations do not fix this: adding `references` lowers
precision (chi 82.5% → 65.8%) without adding recall; adding `method`
(class→method containment) inflates "has a caller" to ~90% but drops
precision to 37–45% because containment is not a call.

## Extension: caddy and hono (2026-10-12)

caddy v2.11.7 (112k LOC Go, scip-go 0.2.7) and hono v4.13.13 (66k LOC TS,
`src/`), same oracles and scoring.

| | caddy Graphify | caddy GridSeak | hono Graphify | hono GridSeak |
|---|---:|---:|---:|---:|
| Call edges found (of truth) | **38.7%** | **100%** | 48.1% | 77.6%* |
| Call edges found where the tool has nodes at both ends | — | 100% | 89.7% (653 of 1,217 edges mappable) | **100%** (945 of 1,217) |
| Used functions shown as having ≥ 1 caller | **60.9%** | **100%** | 82.9% | **97.0%** |
| Used functions with every caller listed | 48.2% | 100% | 64.2% | 90.0%* |
| Extra edges with no explanation | 322 | 0 (695 interface dispatch) | 157 | 60 |

\* declaration-line matching artifact, as for zod; position-based hono call
recall is 99.9% and every-caller-listed 97.4%.

Across four repos, Graphify's local graph shows 17–39% of used functions as
having no caller.

## Caveats

- Four repos, two languages. Graphify claims ~40 languages; not measured here.
- Graphify Cloud (paid) advertises cross-repo links and formal verification;
  only the open-source local code pass was measured.
- Graphify's value proposition is token savings and navigation, not
  exhaustive caller lists; this benchmark measures the latter.
