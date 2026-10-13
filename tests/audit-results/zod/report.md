# Agent report card: zod

2026-10-12 · Claude Code · claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Opus 5.5 alone was exact on 3 of 5; with GridSeak, 4 of 5. Sonnet 5.5 alone was exact on 4 of 5; with GridSeak, 4 of 5. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.16 each.

Answer key: GridSeak compiler-indexed call graph (1713 edges, 100% from the compiler index); the 84 files outside the compiler project are excluded from questions and keys.

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Opus 5.5 alone | 3/5 | 60% (23–88%) | 2 | 0 | 9.8 | $3.06 | $1.02 | 2.65M |
| Opus 5.5 + GridSeak | 4/5 | 80% (38–96%) | 0 | 1 | 11.6 | $3.18 | $0.80 | 2.68M |
| Sonnet 5.5 alone | 4/5 | 80% (38–96%) | 1 | 0 | 5.4 | $1.08 | $0.27 | 1.17M |
| Sonnet 5.5 + GridSeak | 4/5 | 80% (38–96%) | 1 | 0 | 2.6 | $0.65 | $0.16 | 0.70M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `_addCheck` (src/types.ts:1095) | 25 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.12 |
| t1 (callers) | `_addCheck` (src/types.ts:1095) | 25 | 2 | Sonnet 5.5 + GridSeak | 1 | MISS (silent) | 1.00 | 0.76 | 0.6 | $0.15 |
| t1 (callers) | `_addCheck` (src/types.ts:1095) | 25 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.32 |
| t1 (callers) | `_addCheck` (src/types.ts:1095) | 25 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.44 |
| t2 (callers) | `addIssueToContext` (src/helpers/parseUtil.ts:72) | 29 | 2 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.93 | 1.00 | 0.5 | $0.09 |
| t2 (callers) | `addIssueToContext` (src/helpers/parseUtil.ts:72) | 29 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.08 |
| t2 (callers) | `addIssueToContext` (src/helpers/parseUtil.ts:72) | 29 | 2 | Opus 5.5 alone | 1 | PASS | 0.97 | 1.00 | 0.8 | $0.22 |
| t2 (callers) | `addIssueToContext` (src/helpers/parseUtil.ts:72) | 29 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.17 |
| t3 (callers) | `toString` (src/helpers/errorUtil.ts:5) | 33 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.16 |
| t3 (callers) | `toString` (src/helpers/errorUtil.ts:5) | 33 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.13 |
| t3 (callers) | `toString` (src/helpers/errorUtil.ts:5) | 33 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.30 |
| t3 (callers) | `toString` (src/helpers/errorUtil.ts:5) | 33 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.20 |
| t4 (callers) | `processCreateParams` (src/types.ts:130) | 62 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 0.98 | 1.7 | $0.32 |
| t4 (callers) | `processCreateParams` (src/types.ts:130) | 62 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.12 |
| t4 (callers) | `processCreateParams` (src/types.ts:130) | 62 | 3 | Opus 5.5 alone | 1 | MISS (silent) | 0.92 | 0.97 | 2.2 | $0.69 |
| t4 (callers) | `processCreateParams` (src/types.ts:130) | 62 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.24 |
| t5 (callers) | `isAsync` (src/helpers/parseUtil.ts:189) | 92 | 4 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.7 | $0.39 |
| t5 (callers) | `isAsync` (src/helpers/parseUtil.ts:189) | 92 | 4 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.17 |
| t5 (callers) | `isAsync` (src/helpers/parseUtil.ts:189) | 92 | 4 | Opus 5.5 alone | 1 | MISS (silent) | 1.00 | 0.39 | 5.0 | $1.54 |
| t5 (callers) | `isAsync` (src/helpers/parseUtil.ts:189) | 92 | 4 | Opus 5.5 + GridSeak | 1 | MISS (flagged) | 1.00 | 0.39 | 8.8 | $2.13 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
