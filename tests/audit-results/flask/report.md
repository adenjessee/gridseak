# Agent report card: flask

2026-10-13 · Claude Code · claude-haiku-5-5, claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Haiku 5.5 alone was exact on 4 of 6; with GridSeak, 6 of 6. Opus 5.5 alone was exact on 5 of 6; with GridSeak, 6 of 6. Sonnet 5.5 alone was exact on 6 of 6; with GridSeak, 6 of 6. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.05 each.

Answer key: GridSeak compiler-indexed call graph (1342 edges, 100% from the compiler index); targets whose callers share a name with any of the 991 calls the compiler could not resolve (untyped receivers) are not asked.

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 alone | 4/6 | 67% (30–90%) | 2 | 0 | 10.4 | $4.78 | $1.20 | 1.97M |
| Haiku 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 4.1 | $2.21 | $0.37 | 0.97M |
| Opus 5.5 alone | 5/6 | 83% (44–97%) | 1 | 0 | 4.0 | $0.98 | $0.20 | 0.53M |
| Opus 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 3.0 | $1.01 | $0.17 | 0.58M |
| Sonnet 5.5 alone | 6/6 | 100% (61–100%) | 0 | 0 | 4.0 | $0.64 | $0.11 | 0.72M |
| Sonnet 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 1.2 | $0.30 | $0.05 | 0.23M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `generator` (src/flask/helpers.py:126) | 10 | 4 | Haiku 5.5 alone | 1 | MISS (silent) | 0.90 | 1.00 | 0.9 | $0.52 |
| t1 (callers) | `generator` (src/flask/helpers.py:126) | 10 | 4 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.43 |
| t1 (callers) | `generator` (src/flask/helpers.py:126) | 10 | 4 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.08 |
| t1 (callers) | `generator` (src/flask/helpers.py:126) | 10 | 4 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t1 (callers) | `generator` (src/flask/helpers.py:126) | 10 | 4 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.11 |
| t1 (callers) | `generator` (src/flask/helpers.py:126) | 10 | 4 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.22 |
| t2 (callers) | `add_app_template_test` (src/flask/sansio/blueprints.py:532) | 11 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.36 |
| t2 (callers) | `add_app_template_test` (src/flask/sansio/blueprints.py:532) | 11 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.26 |
| t2 (callers) | `add_app_template_test` (src/flask/sansio/blueprints.py:532) | 11 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.05 |
| t2 (callers) | `add_app_template_test` (src/flask/sansio/blueprints.py:532) | 11 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t2 (callers) | `add_app_template_test` (src/flask/sansio/blueprints.py:532) | 11 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.09 |
| t2 (callers) | `add_app_template_test` (src/flask/sansio/blueprints.py:532) | 11 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.12 |
| t3 (callers) | `add_app_template_filter` (src/flask/sansio/blueprints.py:476) | 11 | 2 | Haiku 5.5 alone | 1 | MISS (silent) | 0.91 | 1.00 | 0.8 | $0.31 |
| t3 (callers) | `add_app_template_filter` (src/flask/sansio/blueprints.py:476) | 11 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.31 |
| t3 (callers) | `add_app_template_filter` (src/flask/sansio/blueprints.py:476) | 11 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.06 |
| t3 (callers) | `add_app_template_filter` (src/flask/sansio/blueprints.py:476) | 11 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t3 (callers) | `add_app_template_filter` (src/flask/sansio/blueprints.py:476) | 11 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.08 |
| t3 (callers) | `add_app_template_filter` (src/flask/sansio/blueprints.py:476) | 11 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.11 |
| t4 (callers) | `make_response` (src/flask/app.py:1224) | 11 | 4 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.3 | $0.52 |
| t4 (callers) | `make_response` (src/flask/app.py:1224) | 11 | 4 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.45 |
| t4 (callers) | `make_response` (src/flask/app.py:1224) | 11 | 4 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.06 |
| t4 (callers) | `make_response` (src/flask/app.py:1224) | 11 | 4 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t4 (callers) | `make_response` (src/flask/app.py:1224) | 11 | 4 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.13 |
| t4 (callers) | `make_response` (src/flask/app.py:1224) | 11 | 4 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.14 |
| t5 (callers) | `render_template_string` (src/flask/templating.py:151) | 12 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.7 | $0.72 |
| t5 (callers) | `render_template_string` (src/flask/templating.py:151) | 12 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.30 |
| t5 (callers) | `render_template_string` (src/flask/templating.py:151) | 12 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.09 |
| t5 (callers) | `render_template_string` (src/flask/templating.py:151) | 12 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.1 | $0.04 |
| t5 (callers) | `render_template_string` (src/flask/templating.py:151) | 12 | 2 | Opus 5.5 alone | 1 | MISS (silent) | 0.83 | 1.00 | 0.4 | $0.12 |
| t5 (callers) | `render_template_string` (src/flask/templating.py:151) | 12 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.16 |
| t6 (callers) | `__init__` (src/flask/app.py:308) | 80 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.9 | $2.35 |
| t6 (callers) | `__init__` (src/flask/app.py:308) | 80 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.46 |
| t6 (callers) | `__init__` (src/flask/app.py:308) | 80 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 0.96 | 1.7 | $0.29 |
| t6 (callers) | `__init__` (src/flask/app.py:308) | 80 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.10 |
| t6 (callers) | `__init__` (src/flask/app.py:308) | 80 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.9 | $0.45 |
| t6 (callers) | `__init__` (src/flask/app.py:308) | 80 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.26 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
