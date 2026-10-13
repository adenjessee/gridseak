# Agent report card: hono

2026-10-12 · Claude Code · claude-haiku-5-5, claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Haiku 5.5 alone was exact on 4 of 6; with GridSeak, 5 of 6. Opus 5.5 alone was exact on 5 of 6; with GridSeak, 6 of 6. Sonnet 5.5 alone was exact on 3 of 6; with GridSeak, 6 of 6. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.06 each.

Answer key: GridSeak compiler-indexed call graph (3014 edges, 100% from the compiler index); the 137 files outside the compiler project are excluded from questions and keys; targets whose callers could reach the 8 partly parsed file(s) are not asked.

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 alone | 4/6 | 67% (30–90%) | 1 | 1 | 29.9 | $14.49 | $3.62 | 9.96M |
| Haiku 5.5 + GridSeak | 5/6 | 83% (44–97%) | 1 | 0 | 13.7 | $6.87 | $1.37 | 4.51M |
| Opus 5.5 alone | 5/6 | 83% (44–97%) | 1 | 0 | 14.8 | $4.41 | $0.88 | 5.46M |
| Opus 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 3.0 | $1.01 | $0.17 | 0.58M |
| Sonnet 5.5 alone | 3/6 | 50% (19–81%) | 3 | 0 | 11.0 | $2.38 | $0.79 | 3.76M |
| Sonnet 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 1.5 | $0.38 | $0.06 | 0.29M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `resumeWithContextValues` (src/jsx/context.ts:117) | 12 | 4 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.5 | $0.86 |
| t1 (callers) | `resumeWithContextValues` (src/jsx/context.ts:117) | 12 | 4 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.66 |
| t1 (callers) | `resumeWithContextValues` (src/jsx/context.ts:117) | 12 | 4 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.11 |
| t1 (callers) | `resumeWithContextValues` (src/jsx/context.ts:117) | 12 | 4 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t1 (callers) | `resumeWithContextValues` (src/jsx/context.ts:117) | 12 | 4 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.19 |
| t1 (callers) | `resumeWithContextValues` (src/jsx/context.ts:117) | 12 | 4 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.13 |
| t2 (callers) | `checkUserAgentEquals` (src/helper/adapter/index.ts:86) | 12 | 6 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 2.5 | $1.19 |
| t2 (callers) | `checkUserAgentEquals` (src/helper/adapter/index.ts:86) | 12 | 6 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.55 |
| t2 (callers) | `checkUserAgentEquals` (src/helper/adapter/index.ts:86) | 12 | 6 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.83 | 1.00 | 0.8 | $0.15 |
| t2 (callers) | `checkUserAgentEquals` (src/helper/adapter/index.ts:86) | 12 | 6 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t2 (callers) | `checkUserAgentEquals` (src/helper/adapter/index.ts:86) | 12 | 6 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.29 |
| t2 (callers) | `checkUserAgentEquals` (src/helper/adapter/index.ts:86) | 12 | 6 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.12 |
| t3 (callers) | `fetch` (src/hono-base.ts:483) | 26 | 3 | Haiku 5.5 alone | 1 | MISS (silent) | 0.92 | 1.00 | 7.6 | $3.43 |
| t3 (callers) | `fetch` (src/hono-base.ts:483) | 26 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.1 | $1.19 |
| t3 (callers) | `fetch` (src/hono-base.ts:483) | 26 | 3 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.92 | 1.00 | 3.8 | $0.73 |
| t3 (callers) | `fetch` (src/hono-base.ts:483) | 26 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.07 |
| t3 (callers) | `fetch` (src/hono-base.ts:483) | 26 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 5.9 | $1.64 |
| t3 (callers) | `fetch` (src/hono-base.ts:483) | 26 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.20 |
| t4 (callers) | `encodeBase64` (src/utils/encode.ts:15) | 32 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 3.0 | $1.39 |
| t4 (callers) | `encodeBase64` (src/utils/encode.ts:15) | 32 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.45 |
| t4 (callers) | `encodeBase64` (src/utils/encode.ts:15) | 32 | 3 | Sonnet 5.5 alone | 1 | PASS | 0.97 | 1.00 | 1.0 | $0.20 |
| t4 (callers) | `encodeBase64` (src/utils/encode.ts:15) | 32 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.08 |
| t4 (callers) | `encodeBase64` (src/utils/encode.ts:15) | 32 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.34 |
| t4 (callers) | `encodeBase64` (src/utils/encode.ts:15) | 32 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.17 |
| t5 (callers) | `tryDecode` (src/utils/url.ts:81) | 42 | 5 | Haiku 5.5 alone | 1 | PASS | 0.95 | 1.00 | 7.4 | $3.71 |
| t5 (callers) | `tryDecode` (src/utils/url.ts:81) | 42 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.5 | $1.09 |
| t5 (callers) | `tryDecode` (src/utils/url.ts:81) | 42 | 5 | Sonnet 5.5 alone | 1 | PASS | 0.98 | 1.00 | 2.7 | $0.67 |
| t5 (callers) | `tryDecode` (src/utils/url.ts:81) | 42 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.07 |
| t5 (callers) | `tryDecode` (src/utils/url.ts:81) | 42 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 3.4 | $1.20 |
| t5 (callers) | `tryDecode` (src/utils/url.ts:81) | 42 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.18 |
| t6 (callers) | `escapeToBuffer` (src/utils/html.ts:90) | 51 | 6 | Haiku 5.5 alone | 1 | MISS (flagged) | 0.82 | 0.95 | 7.8 | $3.92 |
| t6 (callers) | `escapeToBuffer` (src/utils/html.ts:90) | 51 | 6 | Haiku 5.5 + GridSeak | 1 | MISS (silent) | 0.82 | 0.95 | 5.9 | $2.93 |
| t6 (callers) | `escapeToBuffer` (src/utils/html.ts:90) | 51 | 6 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.69 | 0.80 | 2.1 | $0.52 |
| t6 (callers) | `escapeToBuffer` (src/utils/html.ts:90) | 51 | 6 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.07 |
| t6 (callers) | `escapeToBuffer` (src/utils/html.ts:90) | 51 | 6 | Opus 5.5 alone | 1 | MISS (silent) | 0.80 | 0.95 | 2.5 | $0.75 |
| t6 (callers) | `escapeToBuffer` (src/utils/html.ts:90) | 51 | 6 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.21 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
