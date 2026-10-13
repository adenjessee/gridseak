# Agent report card: zod

2026-10-12 · Claude Code · claude-haiku-5-5, claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Haiku 5.5 alone was exact on 3 of 3; with GridSeak, 3 of 3. Opus 5.5 alone was exact on 2 of 3; with GridSeak, 3 of 3. Sonnet 5.5 alone was exact on 2 of 3; with GridSeak, 3 of 3. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.12 each.

Answer key: GridSeak compiler-indexed call graph (1785 edges, 100% from the compiler index); the 84 files outside the compiler project are excluded from questions and keys.

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 alone | 3/3 | 100% (44–100%) | 0 | 0 | 14.3 | $6.38 | $2.13 | 3.41M |
| Haiku 5.5 + GridSeak | 3/3 | 100% (44–100%) | 0 | 0 | 7.7 | $3.54 | $1.18 | 1.55M |
| Opus 5.5 alone | 2/3 | 67% (21–94%) | 1 | 0 | 3.6 | $0.98 | $0.49 | 0.56M |
| Opus 5.5 + GridSeak | 3/3 | 100% (44–100%) | 0 | 0 | 3.3 | $1.14 | $0.38 | 0.82M |
| Sonnet 5.5 alone | 2/3 | 67% (21–94%) | 1 | 0 | 6.4 | $1.16 | $0.58 | 1.56M |
| Sonnet 5.5 + GridSeak | 3/3 | 100% (44–100%) | 0 | 0 | 1.3 | $0.35 | $0.12 | 0.23M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `addIssueToContext` (src/helpers/parseUtil.ts:72) | 29 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.8 | $0.86 |
| t1 (callers) | `addIssueToContext` (src/helpers/parseUtil.ts:72) | 29 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.5 | $0.73 |
| t1 (callers) | `addIssueToContext` (src/helpers/parseUtil.ts:72) | 29 | 2 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.93 | 1.00 | 0.5 | $0.07 |
| t1 (callers) | `addIssueToContext` (src/helpers/parseUtil.ts:72) | 29 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.07 |
| t1 (callers) | `addIssueToContext` (src/helpers/parseUtil.ts:72) | 29 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.28 |
| t1 (callers) | `addIssueToContext` (src/helpers/parseUtil.ts:72) | 29 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.15 |
| t2 (callers) | `handleResult` (src/types.ts:92) | 82 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.4 | $2.10 |
| t2 (callers) | `handleResult` (src/types.ts:92) | 82 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.9 | $0.85 |
| t2 (callers) | `handleResult` (src/types.ts:92) | 82 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.5 | $0.31 |
| t2 (callers) | `handleResult` (src/types.ts:92) | 82 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.14 |
| t2 (callers) | `handleResult` (src/types.ts:92) | 82 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 2.0 | $0.53 |
| t2 (callers) | `handleResult` (src/types.ts:92) | 82 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.26 |
| t3 (callers) | `~validate` (src/types.ts:268) | 147 | 4 | Haiku 5.5 alone | 1 | PASS | 0.99 | 1.00 | 8.1 | $3.42 |
| t3 (callers) | `~validate` (src/types.ts:268) | 147 | 4 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 4.3 | $1.96 |
| t3 (callers) | `~validate` (src/types.ts:268) | 147 | 4 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.4 | $0.78 |
| t3 (callers) | `~validate` (src/types.ts:268) | 147 | 4 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.14 |
| t3 (callers) | `~validate` (src/types.ts:268) | 147 | 4 | Opus 5.5 alone | 1 | MISS (silent) | 0.01 | 1.00 | 0.5 | $0.17 |
| t3 (callers) | `~validate` (src/types.ts:268) | 147 | 4 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.1 | $0.73 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
