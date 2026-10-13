# Agent report card: chi

2026-10-12 · Claude Code · claude-haiku-5-5, claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Haiku 5.5 alone was exact on 18 of 18; with GridSeak, 18 of 18. Opus 5.5 alone was exact on 15 of 18; with GridSeak, 18 of 18. Sonnet 5.5 alone was exact on 16 of 18; with GridSeak, 18 of 18. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.04 each.

Answer key: GridSeak compiler-indexed call graph (554 edges, 100% from the compiler index).

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 alone | 18/18 | 100% (82–100%) | 0 | 0 | 38.5 | $18.21 | $1.01 | 8.85M |
| Haiku 5.5 + GridSeak | 18/18 | 100% (82–100%) | 0 | 0 | 16.1 | $7.90 | $0.44 | 3.93M |
| Opus 5.5 alone | 15/18 | 83% (61–94%) | 3 | 0 | 15.4 | $4.71 | $0.31 | 3.19M |
| Opus 5.5 + GridSeak | 18/18 | 100% (82–100%) | 0 | 0 | 6.8 | $2.25 | $0.12 | 1.34M |
| Sonnet 5.5 alone | 16/18 | 89% (67–97%) | 2 | 0 | 13.4 | $2.40 | $0.15 | 2.70M |
| Sonnet 5.5 + GridSeak | 18/18 | 100% (82–100%) | 0 | 0 | 3.3 | $0.79 | $0.04 | 0.64M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.56 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.44 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.37 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.39 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.9 | $0.44 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.17 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.11 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.10 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.1 | $0.09 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.3 | $0.06 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.2 | $0.03 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.14 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.15 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.16 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.20 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.12 |
| t1 (callers) | `cW` (middleware/terminal.go:55) | 9 | 3 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.3 | $0.09 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.53 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.0 | $0.49 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.65 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.59 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 3.3 | $1.44 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.2 | $0.49 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.12 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.12 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.6 | $0.09 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.2 | $0.03 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.23 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.09 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Opus 5.5 alone | 1 | MISS (silent) | 0.91 | 1.00 | 0.6 | $0.16 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.09 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.8 | $0.16 |
| t2 (callers) | `Mux.With` (mux.go:242) | 11 | 3 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.12 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.49 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.39 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.44 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.22 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.9 | $0.44 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.6 | $0.23 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.09 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.09 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.5 | $0.07 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.11 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.08 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.14 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.08 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Opus 5.5 alone | 2 | MISS (silent) | 0.87 | 1.00 | 0.7 | $0.12 |
| t3 (callers) | `nodes.findEdge` (tree.go:821) | 15 | 5 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.2 | $0.07 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.8 | $0.91 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.57 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 2.5 | $1.14 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.15 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.6 | $0.83 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.7 | $0.33 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.13 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.14 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.8 | $0.13 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.31 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.10 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.21 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.17 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.8 | $0.23 |
| t4 (callers) | `Context.Reset` (context.go:82) | 20 | 2 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.3 | $0.09 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.8 | $2.14 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.7 | $0.79 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 2.2 | $1.14 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.5 | $0.71 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 3.7 | $1.77 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.3 | $0.13 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 0.97 | 0.8 | $0.17 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.05 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 0.97 | 0.7 | $0.14 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.05 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 0.97 | 0.9 | $0.17 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.2 | $0.05 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.31 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.20 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.32 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.20 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.1 | $0.35 |
| t5 (callers) | `Mux.Use` (mux.go:100) | 30 | 2 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.6 | $0.22 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Haiku 5.5 alone | 1 | PASS | 0.97 | 1.00 | 3.1 | $1.39 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.3 | $1.04 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 5.0 | $2.14 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.31 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 2.9 | $1.41 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.0 | $0.46 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.94 | 0.97 | 1.1 | $0.20 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.05 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.91 | 1.00 | 1.4 | $0.24 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.05 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.2 | $0.24 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.2 | $0.05 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Opus 5.5 alone | 1 | MISS (silent) | 0.94 | 1.00 | 1.5 | $0.46 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.11 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Opus 5.5 alone | 1 | PASS | 0.97 | 1.00 | 2.3 | $0.70 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.11 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.8 | $0.47 |
| t6 (callers) | `node.walk` (tree.go:670) | 34 | 7 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.3 | $0.10 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
