# Batch B — Compiler tier TypeScript + Go (G1 measurement)

## Provision (network allowed only here)

```
gridseak doctor --provision typescript   # cwd = .cache/g1/zod
gridseak doctor --provision go           # cwd = .cache/g1/chi
```

- `scip-go` v0.1.25 installed from GitHub release
  `https://github.com/sourcegraph/scip-go/releases/download/v0.1.25/scip-go_0.1.25_darwin_arm64.tar.gz`
  → `~/.gridseak/bin/scip-go`
- Zod SCIP index: `.cache/g1/zod/.gridseak/scip/scip-typescript/index.scip` (2.3 MB)
- Chi SCIP index: `.cache/g1/chi/.gridseak/scip/scip-go/index.scip` (955 KB)

## Corpora

| Corpus | Pin | SHA | LOC (approx) |
|---|---|---|---|
| zod | tag `v3.24.2` | `e30870369d5b8f31ff4d0130d4439fd997deb523` | ~15,909 `.ts` under `src/` |
| chi | tag `v5.2.1` | `71307f9b7e4e9527638bc951c42b782cd1560331` | ~14,520 `.go` |

Cloned to `.cache/g1/` (gitignored).

## Scans (`GRIDSEAK_AUTO_PROVISION=0`)

No `npx` in scan stderr on either corpus.

| Corpus | scan_id | high_ratio_on_calls | Call edges High/Low | report |
|---|---|---|---|---|
| zod (ts+js detected) | `ca2317ab-30ed-4644-be3d-bef160a385f1` | **0.1490** | Compiler 808 + Lsp 6 / TreeSitter 4650 | `~/Library/Application Support/com.gridseak.desktop/project-reports/ca2317ab-30ed-4644-be3d-bef160a385f1.report.json` |
| chi (`--lang go`) | `fb3c7134-4a28-4972-a448-667ddbe6e654` | **0.5192** | Compiler 405 / TreeSitter 375 | `.../fb3c7134-4a28-4972-a448-667ddbe6e654.report.json` |

Graph artifacts:

- `~/Library/Application Support/com.gridseak.desktop/project-graphs/ca2317ab-30ed-4644-be3d-bef160a385f1.sqlite`
- `~/Library/Application Support/com.gridseak.desktop/project-graphs/fb3c7134-4a28-4972-a448-667ddbe6e654.sqlite`

## G1 bar (`≥ 0.60`) — first measurement (2026-09-16 morning)

**Not met** on that pass. Failure mode was resolver/caret/index mapping, not missing provision.

- TypeScript (zod): 0.149 — SCIP compiler edges exist (808) but fallback still emitted 4650 TreeSitter Low Call edges.
- Go (chi): 0.519 — closest; still short of 0.60.

First zod scan also logged `typescript SCIP index fingerprint mismatch (IndexStale)` because doctor did not write `fingerprint.txt` (fixed afterward: `finish_index` now writes the sidecar).

## Remeasure after hit-rate fix (2026-09-16)

Fixes (heuristic edges still counted as Low; none promoted to High):

1. `short_symbol_name` reads the SCIP descriptor (`#_parse()`, `/NewRouter().`) instead of the backtick file/module path (`types.ts`, `github.com/go-chi/chi/v5`).
2. Go `call_sites` captures `@receiver` on selector operands so the caret lands on `NewRouter` / `Use`, not `chi` / `r`. Query stays `@call` (not `@method_call`) so fallback does not fan out `Write`/`Get`/`ServeHTTP` to every same-named function.
3. `definition_at` walks the same line for a function/type occurrence when the caret hits a package symbol with no definition (`http.HandlerFunc`).
4. SCIP hit with no in-repo callee (stdlib) marks the site resolved so fallback cannot name-match `chain.HandlerFunc`.
5. Files with no SCIP document (zod `deno/` republish, chi `_examples/`) suppress heuristic Call fallback. The indexer never saw those trees; name-match Call edges there are not a failed High upgrade.

Commands (`GRIDSEAK_AUTO_PROVISION=0`, `GRAPHENGINE_CONFIGS_DIR=graphengine-parsing/configs`, no `npx` in stderr):

```
cd .cache/g1/chi && gridseak --json scan . --lang go --no-incremental --no-progress
cd .cache/g1/zod && gridseak --json scan . --lang typescript --no-incremental --no-progress
```

| Corpus | scan_id | high_ratio_on_calls | Call edges | wall |
|---|---|---|---|---|
| chi (`--lang go`) | `73abec07-12fd-47e9-a2e8-fb3c69fdf971` | **0.7771** | Compiler 488 / TreeSitter 140 | 4.80 s |
| zod (`--lang typescript`) | `4cb6e82a-e4d7-458f-aecb-09c3f8d87452` | **0.9112** | Compiler 1236 + Lsp 6 / TreeSitter 121 | 18.79 s |

Reports:

- `~/Library/Application Support/com.gridseak.desktop/project-reports/73abec07-12fd-47e9-a2e8-fb3c69fdf971.report.json`
- `~/Library/Application Support/com.gridseak.desktop/project-reports/4cb6e82a-e4d7-458f-aecb-09c3f8d87452.report.json`

Graphs:

- `~/Library/Application Support/com.gridseak.desktop/project-graphs/73abec07-12fd-47e9-a2e8-fb3c69fdf971.sqlite`
- `~/Library/Application Support/com.gridseak.desktop/project-graphs/4cb6e82a-e4d7-458f-aecb-09c3f8d87452.sqlite`

**G1 met** on both languages (`≥ 0.60`). Zod `deno/` still has zero Compiler edges (scip-typescript follows `src/` tsconfig); those sites are no longer in the Call denominator. Remaining TreeSitter Lows are indexed files SCIP still misses (140 on chi, 121 on zod).

## Warm 1-file rescan

```
echo "// marker" >> .cache/g1/zod/src/index.ts
GRIDSEAK_AUTO_PROVISION=0 gridseak --json scan . --lang typescript
```

- wall time: **4.15 s** (`real` from `/usr/bin/time -p`)
- warm scan_id: `05e401ab-ccf5-481f-a695-926ab4240c9f`
- Corpus is ~16k LOC TypeScript, **not** 100k LOC. The < 5 s target on a large tree is therefore **not demonstrated**.

## Real fixtures

Real-indexer `index.scip` committed under `graphengine-parsing/tests/fixtures/{ts,py,go,rust}_scip/` plus source trees. Nightly regen: `.github/workflows/scip-fixture-drift.yml` runs `scripts/regen-scip-fixtures.sh`.
