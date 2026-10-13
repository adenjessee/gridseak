# Agent report card: chi

2026-10-12 · Claude Code · claude-haiku-5-5, claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Haiku 5.5 alone was exact on 6 of 6; with GridSeak, 6 of 6. Opus 5.5 alone was exact on 5 of 6; with GridSeak, 6 of 6. Sonnet 5.5 alone was exact on 5 of 6; with GridSeak, 6 of 6. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.04 each.

Answer key: GridSeak compiler-indexed call graph (554 edges, 100% from the compiler index).

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 alone | 6/6 | 100% (61–100%) | 0 | 0 | 12.7 | $6.01 | $1.00 | 2.97M |
| Haiku 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 7.5 | $3.73 | $0.62 | 1.77M |
| Opus 5.5 alone | 5/6 | 83% (44–97%) | 1 | 0 | 4.6 | $1.56 | $0.31 | 1.05M |
| Opus 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 2.3 | $0.72 | $0.12 | 0.47M |
| Sonnet 5.5 alone | 5/6 | 83% (44–97%) | 1 | 0 | 4.5 | $0.82 | $0.16 | 0.95M |
| Sonnet 5.5 + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 1.1 | $0.25 | $0.04 | 0.21M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.56 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.44 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.11 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.14 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.15 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.53 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.0 | $0.49 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.12 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.23 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.09 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.49 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.39 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.09 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.11 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.08 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.8 | $0.91 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.57 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.13 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.31 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.10 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.8 | $2.14 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.7 | $0.79 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 0.97 | 0.8 | $0.17 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.05 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.31 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.20 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Haiku 5.5 alone | 1 | PASS | 0.97 | 1.00 | 3.1 | $1.39 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.3 | $1.04 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.94 | 0.97 | 1.1 | $0.20 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.05 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Opus 5.5 alone | 1 | MISS (silent) | 0.94 | 1.00 | 1.5 | $0.46 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.11 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
