# Agent report card: gascity

2026-10-13 · Claude Code · agent default · read-only tools + language toolchain

The agent alone was exact on 5 of 6; with GridSeak, 6 of 6. Cheapest correct answers: Agent + GridSeak at $0.45 each.

Answer key: GridSeak compiler-indexed call graph (351261 edges, 100% from the compiler index); the 371 files outside the compiler project are excluded from questions and keys; targets whose callers could reach the 2 partly parsed file(s) are not asked.

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Agent alone | 5/6 | 83% (44–97%) | 1 | 0 | 17.9 | $7.40 | $1.48 | 10.21M |
| Agent + GridSeak | 6/6 | 100% (61–100%) | 0 | 0 | 6.6 | $2.71 | $0.45 | 1.36M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `ProxiedStore.adoptPin` (internal/beads/proxied_guard_tick.go:794) | 25 | 7 | Agent alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.42 |
| t1 (callers) | `ProxiedStore.adoptPin` (internal/beads/proxied_guard_tick.go:794) | 25 | 7 | Agent + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.38 |
| t2 (callers) | `parseBeadList` (internal/beads/exec/exec.go:169) | 29 | 3 | Agent alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.33 |
| t2 (callers) | `parseBeadList` (internal/beads/exec/exec.go:169) | 29 | 3 | Agent + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.25 |
| t3 (callers) | `isUnavailableSocketError` (internal/runtime/subprocess/subprocess.go:959) | 41 | 4 | Agent alone | 1 | PASS | 1.00 | 1.00 | 1.4 | $0.52 |
| t3 (callers) | `isUnavailableSocketError` (internal/runtime/subprocess/subprocess.go:959) | 41 | 4 | Agent + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.29 |
| t4 (callers) | `currentSessionAssigneeIdentities` (cmd/gc/idle_nudge.go:418) | 58 | 5 | Agent alone | 1 | MISS (silent) | 0.93 | 1.00 | 1.8 | $0.79 |
| t4 (callers) | `currentSessionAssigneeIdentities` (cmd/gc/idle_nudge.go:418) | 58 | 5 | Agent + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.34 |
| t5 (callers) | `effectExecutor.submit` (cmd/gc/reconcile_session_effects.go:179) | 164 | 8 | Agent alone | 1 | PASS | 0.99 | 1.00 | 5.2 | $1.81 |
| t5 (callers) | `effectExecutor.submit` (cmd/gc/reconcile_session_effects.go:179) | 164 | 8 | Agent + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.0 | $0.74 |
| t6 (callers) | `ReadProviderFileRaw` (internal/sessionlog/reader.go:241) | 174 | 8 | Agent alone | 1 | PASS | 1.00 | 1.00 | 7.8 | $3.52 |
| t6 (callers) | `ReadProviderFileRaw` (internal/sessionlog/reader.go:241) | 174 | 8 | Agent + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.4 | $0.71 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
