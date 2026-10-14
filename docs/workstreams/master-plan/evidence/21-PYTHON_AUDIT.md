# 21 — `gridseak audit` on Python: Click, Flask, httpx

Date: 2026-10-13. Same method as `20`: "list every function that transitively
calls X through direct calls", asked to Claude Haiku, Sonnet and Opus 5.5
alone and with GridSeak's MCP tools, read-only tools, one run per cell.
Results and keys: `tests/audit-results/{click,flask,httpx}/`.

## Answer keys

Python is indexed by scip-python (pinned 0.6.6), which is built on Pyright, so
the keys were checked against oracles on different engines
(`tests/recall-audit/py-oracle/`):

- **Static oracle** on mypy 2.4.0 (with Jedi for calls mypy types as `Any`).
  Every eligible target matches exactly: Click 53/53, Flask 20/20, httpx 14/14;
  all 15 planned tasks exact.
- **Runtime witness**: each repo's own test suite run under a tracing plugin.
  No runtime caller of any planned target is missing from its key.

Getting there fixed GridSeak's Python graph: class instantiation now reaches
`__init__`/`__new__` along the MRO; redeclared names (`@overload`, property
setters, platform branches) no longer create false edges; star-imported names
resolve to the right class (717 call sites in httpx); abstract and `Protocol`
calls are dispatch. Calls the index cannot resolve (untyped receivers such as
pytest fixtures) are recorded, and the audit skips any target they could
affect.

Pins: click `2247b35`, flask `d086db8`, httpx `b5addb6`.

## Results

| | Agent alone | Agent + GridSeak |
|---|---:|---:|
| Exactly right | 39 / 45 | **45 / 45** |
| Wrong, presented as complete | 5 | **0** |
| Cost per correct answer | $0.77 | **$0.27** |
| Time | 82 min | **29 min** |
| Tokens | 16.2M | **6.2M** |

| Model | Alone | + GridSeak | $ per correct (alone → GridSeak) |
|---|---:|---:|---:|
| Haiku 5.5 | 12/15 (2 silent) | 15/15 | 1.88 → 0.53 |
| Sonnet 5.5 | 14/15 (1 silent) | 15/15 | 0.20 → 0.07 |
| Opus 5.5 | 13/15 (2 silent) | 15/15 | 0.35 → 0.21 |

Per repo (all models): Click 16/18 → 18/18, Flask 15/18 → 18/18, httpx 8/9 →
9/9.

## Limits

One run per cell. Three repos. Python keys exclude targets whose callers
include calls the index can't type (many targets in test-heavy code were
skipped for that reason: Click 170, Flask 52, httpx 112 candidates). Silent
misses use the corrected detector from `20`. The Python graph fixes are on
the development branch and ship in the next release.
