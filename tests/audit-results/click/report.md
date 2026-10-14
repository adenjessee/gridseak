# Agent report card: click

2026-10-13 · Claude Code · claude-haiku-5-5, claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Haiku 5.5 alone was exact on 5 of 6; with GridSeak, 6 of 6. Opus 5.5 alone was exact on 6 of 6; with GridSeak, 6 of 6. Sonnet 5.5 alone was exact on 5 of 6; with GridSeak, 6 of 6. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.10 each.

Answer key: GridSeak compiler-indexed call graph (3240 edges, 100% from the compiler index); targets whose callers share a name with any of the 870 calls the compiler could not resolve (untyped receivers) are not asked.

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 alone | 5/6 | 83% (44–97%) | 0 | 1 | 30.3 | $13.41 | $2.68 | 6.38M |
| Haiku 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 7.7 | $4.15 | $0.69 | 2.10M |
| Opus 5.5 alone | 6/6 | 100% (61–100%) | 0 | 0 | 9.4 | $2.83 | $0.47 | 1.89M |
| Opus 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 4.6 | $1.47 | $0.25 | 0.79M |
| Sonnet 5.5 alone | 5/6 | 83% (44–97%) | 1 | 0 | 9.0 | $1.70 | $0.34 | 2.04M |
| Sonnet 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 2.6 | $0.61 | $0.10 | 0.35M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `getchar` (src/click/termui.py:957) | 10 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.0 | $0.59 |
| t1 (callers) | `getchar` (src/click/termui.py:957) | 10 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.36 |
| t1 (callers) | `getchar` (src/click/termui.py:957) | 10 | 2 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.70 | 1.00 | 0.4 | $0.06 |
| t1 (callers) | `getchar` (src/click/termui.py:957) | 10 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.09 |
| t1 (callers) | `getchar` (src/click/termui.py:957) | 10 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.13 |
| t1 (callers) | `getchar` (src/click/termui.py:957) | 10 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.13 |
| t2 (callers) | `__init__` (src/click/utils.py:216) | 10 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.45 |
| t2 (callers) | `__init__` (src/click/utils.py:216) | 10 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.21 |
| t2 (callers) | `__init__` (src/click/utils.py:216) | 10 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.06 |
| t2 (callers) | `__init__` (src/click/utils.py:216) | 10 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t2 (callers) | `__init__` (src/click/utils.py:216) | 10 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.10 |
| t2 (callers) | `__init__` (src/click/utils.py:216) | 10 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.10 |
| t3 (callers) | `_format_default` (src/click/termui.py:128) | 14 | 4 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 9.1 | $3.98 |
| t3 (callers) | `_format_default` (src/click/termui.py:128) | 14 | 4 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.33 |
| t3 (callers) | `_format_default` (src/click/termui.py:128) | 14 | 4 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.13 |
| t3 (callers) | `_format_default` (src/click/termui.py:128) | 14 | 4 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t3 (callers) | `_format_default` (src/click/termui.py:128) | 14 | 4 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.33 |
| t3 (callers) | `_format_default` (src/click/termui.py:128) | 14 | 4 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.17 |
| t4 (callers) | `format_pos` (src/click/_termui_impl.py:195) | 15 | 4 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.3 | $0.72 |
| t4 (callers) | `format_pos` (src/click/_termui_impl.py:195) | 15 | 4 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.83 |
| t4 (callers) | `format_pos` (src/click/_termui_impl.py:195) | 15 | 4 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.14 |
| t4 (callers) | `format_pos` (src/click/_termui_impl.py:195) | 15 | 4 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t4 (callers) | `format_pos` (src/click/_termui_impl.py:195) | 15 | 4 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.20 |
| t4 (callers) | `format_pos` (src/click/_termui_impl.py:195) | 15 | 4 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.15 |
| t5 (callers) | `_infer_flag_kind` (src/click/core.py:3249) | 72 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 7.2 | $3.11 |
| t5 (callers) | `_infer_flag_kind` (src/click/core.py:3249) | 72 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.58 |
| t5 (callers) | `_infer_flag_kind` (src/click/core.py:3249) | 72 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 0.97 | 2.0 | $0.36 |
| t5 (callers) | `_infer_flag_kind` (src/click/core.py:3249) | 72 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.09 |
| t5 (callers) | `_infer_flag_kind` (src/click/core.py:3249) | 72 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.7 | $0.63 |
| t5 (callers) | `_infer_flag_kind` (src/click/core.py:3249) | 72 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.23 |
| t6 (callers) | `decorator` (src/click/decorators.py:217) | 353 | 5 | Haiku 5.5 alone | 1 | ERROR | 0.00 | 0.00 | 10.8 | $4.56 |
| t6 (callers) | `decorator` (src/click/decorators.py:217) | 353 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 3.6 | $1.85 |
| t6 (callers) | `decorator` (src/click/decorators.py:217) | 353 | 5 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.6 | $0.94 |
| t6 (callers) | `decorator` (src/click/decorators.py:217) | 353 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.5 | $0.31 |
| t6 (callers) | `decorator` (src/click/decorators.py:217) | 353 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 5.1 | $1.44 |
| t6 (callers) | `decorator` (src/click/decorators.py:217) | 353 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.4 | $0.69 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
