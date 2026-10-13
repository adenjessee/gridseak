# Agent report card: caddy

2026-10-12 · Claude Code · claude-haiku-5-5, claude-sonnet-5-5, claude-opus-5-5 · read-only tools + language toolchain

Haiku 5.5 alone was exact on 14 of 18; with GridSeak, 18 of 18. Opus 5.5 alone was exact on 18 of 18; with GridSeak, 18 of 18. Sonnet 5.5 alone was exact on 15 of 18; with GridSeak, 18 of 18. Cheapest correct answers: Sonnet 5.5 + GridSeak at $0.16 each.

Answer key: GridSeak compiler-indexed call graph (5942 edges, 100% from the compiler index).

| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Haiku 5.5 alone | 14/18 | 78% (55–91%) | 0 | 4 | 63.9 | $30.82 | $2.20 | 19.26M |
| Haiku 5.5 + GridSeak | 18/18 | 100% (82–100%) | 0 | 0 | 27.6 | $15.11 | $0.84 | 7.81M |
| Opus 5.5 alone | 18/18 | 100% (82–100%) | 0 | 0 | 40.7 | $13.63 | $0.76 | 12.70M |
| Opus 5.5 + GridSeak | 18/18 | 100% (82–100%) | 0 | 0 | 16.9 | $5.28 | $0.29 | 2.70M |
| Sonnet 5.5 alone | 15/18 | 83% (61–94%) | 2 | 1 | 37.0 | $7.91 | $0.53 | 11.57M |
| Sonnet 5.5 + GridSeak | 18/18 | 100% (82–100%) | 0 | 0 | 12.1 | $2.95 | $0.16 | 2.60M |

| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |
|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.61 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.54 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.6 | $0.81 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.31 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.0 | $0.51 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.5 | $0.24 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.10 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.12 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.07 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.07 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.08 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.2 | $0.04 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.18 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.24 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.17 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.14 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.5 | $0.13 |
| t1 (callers) | `Encode.Provision` (modules/caddyhttp/encode/encode.go:71) | 14 | 3 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.13 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.7 | $0.44 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.33 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.35 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.36 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.5 | $0.37 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.31 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.09 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.08 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.10 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.2 | $0.06 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.08 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.2 | $0.05 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.14 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.12 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.13 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.15 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.10 |
| t2 (callers) | `newBufferedResponseWriter` (modules/caddyhttp/caddyauth/caddyauth.go:223) | 14 | 3 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.6 | $0.13 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 3.0 | $1.48 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.58 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 6.8 | $3.19 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.0 | $0.46 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 4.6 | $2.15 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.8 | $0.43 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.17 | 1.00 | 0.5 | $0.10 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.3 | $0.09 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.5 | $0.34 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.12 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.8 | $0.32 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.4 | $0.09 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.3 | $0.46 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.17 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Opus 5.5 alone | 1 | PASS | 0.95 | 1.00 | 1.3 | $0.58 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.4 | $0.18 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.6 | $0.56 |
| t3 (callers) | `adminHandler.getOrigin` (admin.go:978) | 41 | 12 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.1 | $0.41 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 2.3 | $1.21 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.4 | $0.81 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 2.4 | $1.24 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.0 | $0.90 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 3.5 | $1.64 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.8 | $0.99 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.25 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.14 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 0.96 | 1.1 | $0.25 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.5 | $0.13 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.0 | $0.21 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.5 | $0.12 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.2 | $0.48 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.6 | $0.19 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.41 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.24 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 1.3 | $0.43 |
| t4 (callers) | `Upstream.healthy` (modules/caddyhttp/reverseproxy/hosts.go:259) | 44 | 5 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.7 | $0.19 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Haiku 5.5 alone | 1 | ERROR | 0.00 | 0.00 | 7.7 | $4.13 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 4.1 | $2.30 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Haiku 5.5 alone | 1 | MISS (flagged) | 0.42 | 1.00 | 6.8 | $3.02 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.0 | $0.59 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Haiku 5.5 alone | 2 | MISS (flagged) | 0.19 | 1.00 | 4.9 | $2.07 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.6 | $0.82 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Sonnet 5.5 alone | 1 | PASS | 0.99 | 1.00 | 5.1 | $1.24 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.8 | $0.20 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Sonnet 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.8 | $1.09 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 0.9 | $0.23 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Sonnet 5.5 alone | 2 | PASS | 0.99 | 0.98 | 4.6 | $1.10 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 0.9 | $0.20 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Opus 5.5 alone | 1 | PASS | 0.99 | 1.00 | 3.5 | $1.32 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.40 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 4.7 | $1.73 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.1 | $0.40 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 4.4 | $1.50 |
| t5 (callers) | `GetVar` (modules/caddyhttp/vars.go:452) | 129 | 5 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.3 | $0.41 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Haiku 5.5 alone | 1 | MISS (flagged) | 0.15 | 1.00 | 3.0 | $1.29 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.7 | $1.55 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Haiku 5.5 alone | 1 | PASS | 1.00 | 1.00 | 8.0 | $3.49 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Haiku 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 4.6 | $2.19 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Haiku 5.5 alone | 2 | PASS | 1.00 | 1.00 | 5.5 | $2.83 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Haiku 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 2.3 | $1.41 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Sonnet 5.5 alone | 1 | MISS (silent) | 0.03 | 1.00 | 2.3 | $0.36 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.9 | $0.50 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Sonnet 5.5 alone | 1 | MISS (flagged) | 0.03 | 1.00 | 1.3 | $0.19 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Sonnet 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 1.8 | $0.38 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Sonnet 5.5 alone | 2 | PASS | 1.00 | 1.00 | 9.7 | $1.96 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Sonnet 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 1.8 | $0.33 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 5.0 | $1.14 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.5 | $0.64 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Opus 5.5 alone | 1 | PASS | 1.00 | 1.00 | 7.9 | $3.21 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Opus 5.5 + GridSeak | 1 | PASS | 1.00 | 1.00 | 2.2 | $0.59 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Opus 5.5 alone | 2 | PASS | 1.00 | 1.00 | 4.3 | $0.96 |
| t6 (callers) | `isOpenCurlyBrace` (caddyconfig/caddyfile/lexer.go:351) | 293 | 9 | Opus 5.5 + GridSeak | 2 | PASS | 1.00 | 1.00 | 2.3 | $0.57 |

A miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.

Reproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).
