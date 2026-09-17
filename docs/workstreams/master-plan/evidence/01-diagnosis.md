# Plan 01 — T0 diagnosis (SymbolIndex mapping loss)

**Date:** 2026-07-02  
**Branch:** `plan-01-truth-recovery`  
**Command:** `./target/release/graphengine-parsing --configs-dir graphengine-parsing/configs parse --root . --lang rust --db /tmp/tr_rust.sqlite --lsp-policy fast --no-incremental` (stderr suppressed; telemetry from graph metadata)

## Confirmed: Layer-2 runs; loss is post-adapter SymbolIndex mapping

- `resolution_disclosure_rust`: `tier_used=layer2`, `high_edges=14634`
- `resolution_layer2_telemetry_json` (post-instrumentation):

```json
{
  "call_refs_seen": 41074,
  "high_resolutions": 14634,
  "medium_resolutions": 5,
  "no_target_misses": 26263,
  "adapter_errors": 172,
  "edges_emitted": 5305,
  "symbol_caller_misses": 10,
  "symbol_callee_misses": 4545
}
```

## Dominant miss category: **CalleeNotFound** (>50% of mapping loss)

| Bucket | Count | Share of `(high+medium) − edges_emitted` |
|--------|------:|------------------------------------------:|
| `symbol_caller_misses` | 10 | ~0.1% |
| `symbol_callee_misses` | 4,545 | **~86%** |
| Self-loop / other drops (residual) | ~4,786 | ~13.9% |

**Adapter conversion** `edges_emitted / (high+medium)` = **5305 / 14639 ≈ 0.362** (baseline UF-FU-012: **0.635**).

## Interpretation

1. Layer-2 adapter initialises and resolves call sites (`high+medium` ≫ 0).
2. The majority of post-adapter loss is **`find_callee_for`** failing to bind rust-analyzer `goto_definition` targets to tree-sitter `Function` nodes — consistent with UF-FU-012 / `rust-layer2-symbol-index-loss.md` (impl skew, macro sites, path/line misalignment).
3. A large share of adapter `Ok(Some(_))` results still cannot map because the **callee symbol is absent from the tree-sitter symbol list** (proc-macro / external crate — UF-FU-003 ceiling; plan 02 scope).
4. **`RUST_LOG` / `gridseak scan` do not surface Layer-2 init** (D1/D2): direct parser binary required; metadata keys above are the durable diagnosis surface.

## Next fix target (T1)

Improve `find_callee_for` line/name/proximity matching and path-normalised file lookup; do **not** chase proc-macro expansion in plan 01.
