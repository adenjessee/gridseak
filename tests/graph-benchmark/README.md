# Code-graph accuracy benchmark

Scores any code graph's "who calls this?" answers against each language's own
type checker (oracles in `tests/recall-audit/`). Same corpora and rules for
every tool. Results: `docs/workstreams/master-plan/evidence/15-GRAPH_BENCHMARK.md`.

```sh
tests/recall-audit/run.sh                      # builds oracles + GridSeak graphs
uv tool install graphifyy==0.9.86              # or any tool that emits a graph
(cd <copy-of-chi> && graphify update .)        # code-only, local, no LLM
python3 tests/graph-benchmark/score.py target/recall-audit/chi.truth.json <copy-of-chi>/graphify-out/graph.json calls
python3 tests/graph-benchmark/score.py target/recall-audit/chi.truth.json target/recall-audit/data-chi/project-graphs/<id>.sqlite x "" .cache/g1/chi
# zod: build declaration-level truth by keeping rows with `caller_line` from the TS oracle, scope `src/`
```

`score.py <truth> <graph.json|graph.sqlite> <relations> [scope] [repo root for sqlite]`
