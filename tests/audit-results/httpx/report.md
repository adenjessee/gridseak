# Agent report card: httpx

2026-10-13 · Claude Code · claude-haiku-5-5, claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Haiku 5.5 alone was exact on 3 of 3; with GridSeak, 3 of 3. Opus 5.5 alone was exact on 2 of 3; with GridSeak, 3 of 3. Sonnet 5.5 alone was exact on 3 of 3; with GridSeak, 3 of 3. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.07 each.

Answer key: GridSeak compiler-indexed call graph (2364 edges, 100% from the compiler index); targets whose callers share a name with any of the 105 calls the compiler could not resolve (untyped receivers) are not asked.

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 alone | 3/3 | 100% (44–100%) | 0 | 0 | 9.9 | $4.39 | $1.46 | 1.99M |
| Haiku 5.5 + GridSeak | 3/3 | 100% (44–100%) | 0 | 0 | 3.1 | $1.60 | $0.53 | 0.66M |
| Opus 5.5 alone | 2/3 | 67% (21–94%) | 1 | 0 | 2.8 | $0.74 | $0.37 | 0.36M |
| Opus 5.5 + GridSeak | 3/3 | 100% (44–100%) | 0 | 0 | 1.8 | $0.63 | $0.21 | 0.37M |
| Sonnet 5.5 alone | 3/3 | 100% (44–100%) | 0 | 0 | 2.5 | $0.44 | $0.15 | 0.37M |
| Sonnet 5.5 + GridSeak | 3/3 | 100% (44–100%) | 0 | 0 | 0.9 | $0.22 | $0.07 | 0.12M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `flush` (httpx/_decoders.py:317) | 16 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.56 |
| t1 (callers) | `flush` (httpx/_decoders.py:317) | 16 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.45 |
| t1 (callers) | `flush` (httpx/_decoders.py:317) | 16 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.11 |
| t1 (callers) | `flush` (httpx/_decoders.py:317) | 16 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.05 |
| t1 (callers) | `flush` (httpx/_decoders.py:317) | 16 | 3 | Opus 5.5 alone | 1 | MISS (silent) | 0.88 | 1.00 | 0.6 | $0.16 |
| t1 (callers) | `flush` (httpx/_decoders.py:317) | 16 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.12 |
| t2 (callers) | `json` (httpx/_models.py:831) | 64 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.7 | $0.76 |
| t2 (callers) | `json` (httpx/_models.py:831) | 64 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.47 |
| t2 (callers) | `json` (httpx/_models.py:831) | 64 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.17 |
| t2 (callers) | `json` (httpx/_models.py:831) | 64 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.08 |
| t2 (callers) | `json` (httpx/_models.py:831) | 64 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.25 |
| t2 (callers) | `json` (httpx/_models.py:831) | 64 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.23 |
| t3 (callers) | `__init__` (httpx/_transports/default.py:280) | 75 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 7.1 | $3.06 |
| t3 (callers) | `__init__` (httpx/_transports/default.py:280) | 75 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.5 | $0.68 |
| t3 (callers) | `__init__` (httpx/_transports/default.py:280) | 75 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.16 |
| t3 (callers) | `__init__` (httpx/_transports/default.py:280) | 75 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.09 |
| t3 (callers) | `__init__` (httpx/_transports/default.py:280) | 75 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.5 | $0.33 |
| t3 (callers) | `__init__` (httpx/_transports/default.py:280) | 75 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.27 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
