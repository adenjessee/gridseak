# Agent report card: hono

2026-10-12 · Claude Code · claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Opus 5.5 alone was exact on 5 of 6; with GridSeak, 6 of 6. Sonnet 5.5 alone was exact on 3 of 6; with GridSeak, 4 of 6. Cheapest correct answers: Opus 5.5 + GridSeak at $0.46 each.

Answer key: GridSeak compiler-indexed call graph (3012 edges, 100% from the compiler index); the 137 files outside the compiler project are excluded from questions and keys; targets whose callers could reach the 8 partly parsed file(s) are not asked.

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Opus 5.5 alone | 5/6 | 83% (44–97%) | 1 | 0 | 18.2 | $5.40 | $1.08 | 6.34M |
| Opus 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 8.4 | $2.74 | $0.46 | 3.04M |
| Sonnet 5.5 alone | 3/6 | 50% (19–81%) | 3 | 0 | 10.3 | $2.52 | $0.84 | 4.23M |
| Sonnet 5.5 + GridSeak | 4/6 | 67% (30–90%) | 2 | 0 | 4.4 | $1.02 | $0.25 | 1.61M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `resumeWithContextValues` (src/jsx/context.ts:117) | 12 | 4 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.92 | 1.00 | 0.7 | $0.13 |
| t1 (callers) | `resumeWithContextValues` (src/jsx/context.ts:117) | 12 | 4 | Sonnet 5.5 + GridSeak | 1 | MISS (silent) | 0.92 | 1.00 | 0.2 | $0.04 |
| t1 (callers) | `resumeWithContextValues` (src/jsx/context.ts:117) | 12 | 4 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.21 |
| t1 (callers) | `resumeWithContextValues` (src/jsx/context.ts:117) | 12 | 4 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.13 |
| t2 (callers) | `checkUserAgentEquals` (src/helper/adapter/index.ts:86) | 12 | 6 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.83 | 1.00 | 0.6 | $0.13 |
| t2 (callers) | `checkUserAgentEquals` (src/helper/adapter/index.ts:86) | 12 | 6 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t2 (callers) | `checkUserAgentEquals` (src/helper/adapter/index.ts:86) | 12 | 6 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.0 | $0.36 |
| t2 (callers) | `checkUserAgentEquals` (src/helper/adapter/index.ts:86) | 12 | 6 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.21 |
| t3 (callers) | `fetch` (src/hono-base.ts:483) | 26 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 2.7 | $0.61 |
| t3 (callers) | `fetch` (src/hono-base.ts:483) | 26 | 3 | Sonnet 5.5 + GridSeak | 1 | MISS (silent) | 0.88 | 1.00 | 2.5 | $0.56 |
| t3 (callers) | `fetch` (src/hono-base.ts:483) | 26 | 3 | Opus 5.5 alone | 1 | PASS | 0.96 | 1.00 | 5.4 | $1.50 |
| t3 (callers) | `fetch` (src/hono-base.ts:483) | 26 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 5.1 | $1.64 |
| t4 (callers) | `encodeBase64` (src/utils/encode.ts:15) | 32 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.0 | $0.18 |
| t4 (callers) | `encodeBase64` (src/utils/encode.ts:15) | 32 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.12 |
| t4 (callers) | `encodeBase64` (src/utils/encode.ts:15) | 32 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 2.0 | $0.54 |
| t4 (callers) | `encodeBase64` (src/utils/encode.ts:15) | 32 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.28 |
| t5 (callers) | `tryDecode` (src/utils/url.ts:81) | 42 | 5 | Sonnet 5.5 alone | 1 | PASS | 0.98 | 1.00 | 2.7 | $0.71 |
| t5 (callers) | `tryDecode` (src/utils/url.ts:81) | 42 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.07 |
| t5 (callers) | `tryDecode` (src/utils/url.ts:81) | 42 | 5 | Opus 5.5 alone | 1 | PASS | 0.98 | 1.00 | 4.7 | $1.28 |
| t5 (callers) | `tryDecode` (src/utils/url.ts:81) | 42 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.20 |
| t6 (callers) | `escapeToBuffer` (src/utils/html.ts:90) | 51 | 6 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.76 | 0.74 | 2.5 | $0.75 |
| t6 (callers) | `escapeToBuffer` (src/utils/html.ts:90) | 51 | 6 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.18 |
| t6 (callers) | `escapeToBuffer` (src/utils/html.ts:90) | 51 | 6 | Opus 5.5 alone | 1 | MISS (silent) | 0.76 | 0.78 | 4.2 | $1.51 |
| t6 (callers) | `escapeToBuffer` (src/utils/html.ts:90) | 51 | 6 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.29 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
