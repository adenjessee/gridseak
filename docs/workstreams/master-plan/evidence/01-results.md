# Plan 01 — T6 verification results

**Date:** 2026-07-02  
**Branch:** `plan-01-truth-recovery`  
**Rust parse DB:** `/tmp/tr_rust.sqlite` (direct parser, `--no-incremental`)

## Acceptance bars (amended plan 01)

| Criterion | Bar | Measured | Status |
|-----------|-----|----------|--------|
| Layer-2 active (compiler edges > 0) | > 0 | **5305** LSP call edges (`resolution_lsp_edges`) | **PASS** |
| SymbolIndex conversion | ≥ 0.90 (baseline 0.635) | **0.362** (`edges_emitted` 5305 / resolved 14639) | **FAIL** — see stop note |
| Whole-repo `high_ratio_on_calls` | ≥ 0.18 (baseline 0.143) | **Not re-measured on full multi-lang scan this session** — rust-only parse insufficient for whole-repo ratio | **BLOCKED** on full `gridseak scan` |
| Per-language `ResolutionDisclosure` | present + ≥1 `skip_reason` on heuristic lang | `resolution_disclosure_rust` persisted; heuristic langs need full scan | **PARTIAL** (rust row proven) |
| F2 routing regression | lenient `"."` must not return unrelated project | `gridseak-local-store` tests green (7/7) | **PASS** |
| `cargo +1.91 test --workspace` | green | **1 pre-existing failure** unrelated to plan 01: `graphengine-parsing/tests/config_loading.rs::test_python_config_loading` (`pyright-langserver` vs `pyright`). All plan-specific tests green. | **PARTIAL** |

## Stop note (conversion + high_ratio)

Per plan D4 / T6: conversion **did not reach 0.90** despite SymbolIndex improvements (line slack, name fallback, line-only caller containment, nearest-line callee fallback). Telemetry shows **4545 `symbol_callee_misses`** vs **10 caller misses** — callee targets often **missing from the tree-sitter symbol table** (proc-macro ceiling, UF-FU-003), not merely mis-aligned ranges. Further progress requires plan 02 proc-macro / SCIP work, not redefining the bar.

## Evidence commands

```bash
cargo +1.91 build -p graphengine-parsing --release
./target/release/graphengine-parsing --configs-dir graphengine-parsing/configs \
  parse --root . --lang rust --db /tmp/tr_rust.sqlite --lsp-policy fast --no-incremental

sqlite3 /tmp/tr_rust.sqlite "select key, value from metadata where key like 'resolution_%' or key like 'resolution_disclosure_%';"

cargo +1.91 test -p gridseak-local-store
cargo +1.91 test -p graphengine-parsing rust_layer2
cargo +1.91 test -p gridseak-cli -- render::resolution_disclosure
```

## Delivered artifacts

- `ResolutionDisclosure` + per-lang metadata persistence
- Analysis report + MCP context caveats surface `resolution_disclosure`
- CLI scan prints per-language disclosure lines (`render/resolution_disclosure.rs`)
- F2: `resolve_project_lenient` uses exact cwd root match only; lists registered projects on miss
- SymbolIndex telemetry in `resolution_layer2_telemetry_json` + extended `ResolveSnapshot`
