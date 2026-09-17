# 01 — TRUTH RECOVERY: single-session task breakdown

Status: v2 (2026-07-01), amended per plan-owner + UF-FU-012. Branch:
`plan-01-truth-recovery`. Each task is one Composer session (~one PR,
≤ 6 files). Follow `EXECUTION_PROTOCOL.md` (`cargo +1.91`, additive
serde, tests-are-done). **Read `evidence/01-notes.md` first.**

## Orientation (plan in 5 lines)

1. Self-scan reports `syntactic_only` at `high_ratio_on_calls = 0.143`
   despite Rust Layer-2 existing — UF-FU-012 confirms the adapter
   resolves but **36% SymbolIndex mapping loss** drops edges before the
   graph (conversion baseline 0.635).
2. The skip is **silent**: no per-language disclosure of which tier ran
   or why semantic resolution didn't.
3. MCP default `project: "."` silently returns the most-recent scan
   (F2 wrong-project bug).
4. Fix: close SymbolIndex mapping loss (conversion ≥ 0.90), add
   `ResolutionDisclosure` per language, fix F2 routing.
5. **Plan 01 exit bars** (amended; 0.60 deferred to G1/plan 02): Layer-2
   compiler edges > 0; conversion ≥ 0.90; whole-repo `high_ratio_on_calls`
   ≥ 0.18; disclosure + F2 regression test.

## Task dependency graph

```
T0 (diagnose) ──► T1 (fix resolver)
                       │
                       └► T2 (disclosure type + parser persist)
                              └► T3 (analysis report + MCP surface)
                                     └► T4 (CLI per-language line)
T5 (MCP F2 routing)  ── independent, runs in parallel ──►
                                                          T6 (verify + results)
(T6 is blocked by T1, T3, T4, T5)
```

Parallelism: **T5** shares no files with T0–T4 and can start immediately
in parallel. T2 may start once T1's fix is committed (it needs the
resolver to actually run to produce a non-trivial `tier_used`).

---

## T0 — Diagnose why Rust Layer-2 does not run on the scan path

- **Advances:** plan Step 1. **Acceptance criterion:** "Diagnosis
  document exists with log evidence naming root cause A/B/C."
- **Type:** read-only investigation + one new evidence doc. **No code
  changes.** Composer-runnable.
- **Files touched (1):** create
  `docs/workstreams/master-plan/evidence/01-diagnosis.md`.

### Correction to the plan (drift D1 + D2)

The plan's Step-1 command
`RUST_LOG=… ./target/release/gridseak scan . 2>&1 | tee …` will **not**
surface the Layer-2 init line. `gridseak scan` spawns the parser as a
subprocess per language (`gridseak-engine-runner/src/lib.rs`
`run_parser_for_language`), and the parser
(`graphengine-parsing/src/main.rs` lines 166–180) ignores `RUST_LOG` —
its filter is `warn` unless `--verbose` (then `debug`). The
`rust_layer2::init` and factory-fallback lines are `info!`, so they are
suppressed under a normal scan.

### Steps

1. Confirm the built parser has the feature (drift D6):
   `cargo +1.91 build -p graphengine-parsing --release`
   then `./target/release/graphengine-parsing --version` and, to prove
   the feature is compiled, that `build_rust_semantic_resolver` is the
   `#[cfg(feature = "rust-layer2")]` arm — grep the build with
   `cargo +1.91 build -p graphengine-parsing --release -v 2>&1 | rg 'feature="rust-layer2"'`
   (or inspect that no `--no-default-features` is used).
2. Reproduce the resolver path **directly against the parser binary**
   with logging on:
   ```
   ./target/release/graphengine-parsing --verbose parse \
     --root . --lang rust --db /tmp/tr_rust.sqlite \
     --lsp-policy fast --no-incremental 2>/tmp/tr_rust.stderr
   ```
   Then classify by grepping `/tmp/tr_rust.stderr`:
   - init present? `rg 'rust Layer-2 adapter loaded' /tmp/tr_rust.stderr`
   - fallback taken? `rg 'Rust Layer-2 adapter unavailable' /tmp/tr_rust.stderr`
   - resolve summary? `rg 'rust Layer-2 resolve done' /tmp/tr_rust.stderr`
     (fields: `call_refs=… high=… medium=… misses=… errors=… edges=…`)
3. Also capture the **real scan-path** evidence: run
   `./target/release/gridseak scan .` once, find the scan id it prints,
   and read
   `~/Library/Application Support/com.gridseak.desktop/project-reports/<scan_id>.parse.rust.stderr`
   (the runner tees the parser subprocess stderr there). Note whether
   the init/fallback lines appear at all (they will be absent because
   the runner does not pass `--verbose` — that absence is itself
   evidence for the "silent" claim).
4. Map to the plan's branches and write `evidence/01-diagnosis.md`:
   - **A** — no `rust Layer-2 adapter loaded` line even with `--verbose`
     → factory never constructed the adapter. Inspect
     `factory.rs::build_rust_semantic_resolver` (lines 227–256) and
     whether `RustLayer2SemanticResolver::new` returned `Err` (workspace
     root / `Cargo.toml` / sysroot). Check the `Rust Layer-2 adapter
     unavailable ({err})` line for the actual error.
   - **B** — init present but `resolve done: … high=0` → adapter runs,
     resolves nothing (workspace-root shape, path mismatch in
     `SymbolIndex::find_callee_for` / `path_matches`, or the multi-lang
     `hints.language != "rust"` short-circuit in `rust_layer2.rs` line
     299).
   - **C** — init + non-zero `high=` in the parser log, but the report
     JSON `call_edges_by_confidence.high` stays low → edges lost between
     resolver output and analysis aggregation (check that Layer-2's
     `Lsp/High` edges survive `GraphBuilder::build_from_results` and the
     `min_confidence` filter, and the analyzer's
     `call_edges_by_confidence` inputs in `graph_prep.rs`).
- **Verify (observable):** `evidence/01-diagnosis.md` exists, names A/B/C
  with pasted stderr excerpts and the parser's
  `rust Layer-2 resolve done: …` line (or its documented absence).

---

## T1 — Fix the root cause so Rust Layer-2 emits High call edges in the scan path

- **Blocked by:** T0. **Advances:** plan Step 2. **Acceptance:** feeds
  the G1 fidelity criterion (verified in T6).
- **Type:** minimal, clean fix of exactly the A/B/C cause from T0. Keep
  factory-routing changes isolated in **one commit** so `git revert`
  restores old behavior (plan Rollback/risk).
- **Files (likely ≤ 4, cause-dependent):**
  `graphengine-parsing/src/application/use_cases/parse_repo/factory.rs`
  (`build_rust_semantic_resolver`, lines 227–256);
  `graphengine-parsing/src/infrastructure/semantic/rust_layer2.rs`;
  `graphengine-ra-ide-adapter/src/resolver.rs` (only if the adapter's
  `from_workspace_root` is the failing point); a targeted test file
  under `graphengine-parsing/tests/`.

### Known candidate fixes (pick per T0; do not shotgun)

- **Cause A (factory `Err` → silent subprocess-LSP fallback):** the
  `match RustLayer2SemanticResolver::new(&ws_path)` arm at factory.rs
  238–255 logs the failure at `info!` and falls back. If `new` fails
  because `ws_path` is a directory shape rust-analyzer can't load, fix
  the workspace-root derivation (it currently uses
  `workspace_root.to_file_path()` else `current_dir()`). The parser is
  invoked with `--root <canonical repo>` and passes
  `url::Url::from_file_path(canonical_root)` (main.rs 343), so `ws_path`
  should be the repo root — confirm `RustAnalyzerSemanticResolver::
  from_workspace_root` accepts a dir containing the workspace
  `Cargo.toml`. **Gotcha:** do not turn the `Err` into a scan-fatal
  error — the plan requires the scan to still complete; instead the fix
  is to make `new` succeed, and the *disclosure* (T2) is what makes any
  remaining fallback loud.
- **Cause B (runs, resolves 0):** inspect `find_callee_for` /
  `path_matches` (rust_layer2.rs 477–574) for the extractor's stored
  path shape vs. `ra_ap_ide`'s VFS absolute paths on THIS repo, and the
  `hints.language` guard (line 298–305). Add a fixture-backed test
  mirroring `tests/rust_layer2_integration.rs`
  (`rust_layer2_resolves_main_to_callee_and_marks_site`, lines 37–148)
  but exercising the path-shape that failed.
- **Cause C (edges lost downstream):** trace an `Lsp/High` Call edge from
  `resolve_one` (rust_layer2.rs 279–286) through `GraphBuilder` and the
  `min_confidence` gate; ensure High edges are never dropped by the
  Medium default (`Confidence::Medium` from main.rs 298–308 is the
  *threshold*, and `High >= Medium`, so this is unlikely — verify).
- **Gotchas:** `Provenance` is `Copy`; `Confidence` derives
  `PartialOrd` (provenance.rs). `Edge::new` rejects `Call` self-loops
  (rust_layer2.rs 255–273 already handles this) — don't reintroduce
  them. Layer-2 is `!Sync` and wrapped in a `Mutex`; never hold the lock
  across `.await`.
- **Verify (observable):** re-run the T0 direct-parser command; the
  parser log line `rust Layer-2 resolve done:` shows `high=` in the
  thousands (not 0), and `edges=` non-trivial. `cargo +1.91 test -p
  graphengine-parsing` green; the new/updated fixture test passes.
  `cargo +1.91 clippy -p graphengine-parsing` no new warnings.

---

## T2 — Define `ResolutionDisclosure` + `SkipReason`; persist per-language from the parser

- **Blocked by:** T1 (needs the resolver actually running to record a
  real `tier_used`). **Advances:** plan Step 3.1 + 3.2 (producer half).
- **Files (≤ 6):** new
  `graphengine-parsing/src/application/resolution_disclosure.rs`;
  `graphengine-parsing/src/application/mod.rs` (declare module);
  `graphengine-parsing/src/application/ports.rs` (carry disclosure out
  of resolution — e.g. add `disclosure: Option<ResolutionDisclosure>`
  to `ResolutionStatsSummary`, additive);
  `graphengine-parsing/src/application/use_cases/parse_repo/factory.rs`
  and/or `.../semantic/rust_layer2.rs` +
  `.../infrastructure/lsp/resolver.rs` (populate `tier_attempted /
  tier_used / skip_reason`);
  `graphengine-parsing/src/application/use_cases/parse_repo/pipeline/orchestrator.rs`
  (persist to metadata).

### Type shape (paste target)

```rust
// resolution_disclosure.rs
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolutionDisclosure {
    pub language: String,
    pub tier_attempted: ResolutionTierKind,
    pub tier_used: ResolutionTierKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<SkipReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionTierKind { Layer2, SubprocessLsp, Heuristic }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    AdapterInitFailed,
    ServerMissing,
    LanguageNotRouted,
    NoReferences,
    PolicyDisabled,
}
```

### Notes / gotchas

- **Drift D5 (critical):** the parse DB is shared across language passes
  and metadata rows are unkeyed, so writing a single
  `resolution_disclosure` key would be clobbered by the last language.
  Persist **per-language**: in `orchestrator.rs` (next to the existing
  `graph.metadata.insert("resolution_lsp_edges", …)` block, ~lines
  489–516), write `resolution_disclosure_<language>` =
  `serde_json::to_string(&disclosure)`. Use the orchestrator's
  `language` variable for the key suffix.
- `SkipReason` is a **closed enum, no free-text**. Every match on it
  must be exhaustive; the plan names the initial variants above — add
  none speculatively.
- Populate the producer honestly: the Layer-2 arm records
  `tier_attempted = Layer2`; on the factory's `Err(...)` fallback
  (factory.rs 246) record `tier_used = SubprocessLsp` (or `Heuristic`
  if the LSP is also unavailable) with `skip_reason =
  AdapterInitFailed`. The `hints.language != "rust"` short-circuit
  (rust_layer2.rs 299) maps to `LanguageNotRouted`. Zero references maps
  to `NoReferences`.
- Additive serde only (`#[serde(default)]` on the new field wherever it
  lands on a persisted type); do not break old parse.db reads.
- **Verify (observable):** unit test in `resolution_disclosure.rs`
  round-trips each variant through serde and pins the snake_case wire
  strings (mirror the pattern at ports.rs
  `coverage_gap_serde_roundtrips_with_tagged_wire_format`). After a
  parser run, the key is present:
  `sqlite3 /tmp/tr_rust.sqlite "select key,value from graph_metadata where key like 'resolution_disclosure_%';"`
  returns a `rust` row with `tier_used=layer2`. `cargo +1.91 test -p
  graphengine-parsing` green.
  (Confirm the metadata table/column names via
  `graphengine-parsing/src/infrastructure/storage/` — the analyzer reads
  them through `graph::read_metadata`.)

---

## T3 — Surface disclosure in the analysis report and MCP responses

- **Blocked by:** T2. **Advances:** plan Step 3.2 (consumer half).
  **Acceptance:** "Every scan report + MCP response contains per-language
  `ResolutionDisclosure`; a language with no semantic resolution has a
  machine-readable `SkipReason`."
- **Files (≤ 5):**
  `graphengine-analysis/src/health/report.rs` (add
  `resolution_disclosure: Vec<ResolutionDisclosure>` to
  `ResolutionQuality`, struct at lines 838–860, `#[serde(default,
  skip_serializing_if = "Vec::is_empty")]`);
  `graphengine-analysis/src/health/pipeline/segments/graph_prep.rs`
  (read the per-language keys — extend the pattern of
  `load_lsp_resolution_telemetry` at lines 253–291; enumerate keys via a
  store helper that does `SELECT key,value FROM graph_metadata WHERE key
  LIKE 'resolution_disclosure_%'`, deserialize each into a
  `ResolutionDisclosure`); a store helper in
  `graphengine-parsing/src/infrastructure/storage/` if one doesn't
  already expose prefix reads; `gridseak-cli/src/context_command.rs`
  and/or the MCP scan/status handlers in `gridseak-cli/src/main.rs`
  (include the disclosure array in the response envelope).

### Notes / gotchas

- The analysis crate must not import `graphengine-parsing`'s application
  layer if that creates a cycle — mirror how `FallbackReasonCounts` is
  shared today (it lives in `graphengine-parsing::application::
  lsp_telemetry` and is re-read by analysis via serde from metadata
  JSON). Cleanest: **deserialize the JSON into an analysis-local mirror
  struct** with the identical field names/serde, exactly as
  `lsp_resolution_telemetry` is handled — do not add a new crate
  dependency edge just for this type. Confirm which direction the
  existing dependency runs before choosing (drift D5 / notes).
- Additive: old reports without the field deserialize fine because of
  `#[serde(default)]`; new field is skipped when empty so pre-fix
  fixtures stay byte-stable where that matters.
- **Verify (observable):** after a full `gridseak scan .`, the report
  JSON has
  `resolution_quality.resolution_disclosure` as a non-empty array with
  one entry per parsed language; the `rust` entry has
  `tier_used: "layer2"`, and at least one heuristic-only language
  (e.g. `python` if pyright is absent) has a `skip_reason` such as
  `"server_missing"`. `cargo +1.91 test -p graphengine-analysis` green.

---

## T4 — CLI `gridseak scan` prints one disclosure line per language

- **Blocked by:** T3. **Advances:** plan Step 3.3.
- **Files (≤ 3):** `gridseak-cli/src/scan_command.rs` (read the report's
  `resolution_quality.resolution_disclosure` after the scan completes);
  a small new renderer helper under `gridseak-cli/src/render/`
  (keep formatting out of `scan_command.rs`; a purpose-built
  `render/resolution_disclosure.rs` follows the DRY/modular rule); wire
  it into the existing render module (`render/mod.rs`).
- **Format (plan-specified):**
  `rust: semantic (layer2, 4,812 high edges)` and
  `python: heuristic only (skip: ServerMissing pyright-langserver)`.
  Map `ResolutionTierKind::Layer2 | SubprocessLsp` → the word
  `semantic`; `Heuristic` → `heuristic only`; render `skip_reason` in
  the parenthetical when present. High-edge count comes from the
  report's `call_edges_by_confidence.high` (or a per-language count if
  T2/T3 chose to persist one — prefer the value that is actually
  available; note which).
- **Gotcha:** `gridseak scan` renders from the report JSON on disk, not
  from an in-process graph (D1) — the disclosure must have made it into
  the report via T3 for this line to render. Do not recompute anything
  in the CLI.
- **Verify (observable):** `./target/release/gridseak scan .` prints one
  line per language matching the format; `rust:` shows `semantic
  (layer2, …)`. `cargo +1.91 test -p gridseak-cli` green.

---

## T5 — Fix MCP default-project routing (defect F2) + regression test

- **Blocked by:** nothing (runs in parallel with T0–T4). **Advances:**
  plan Step 4. **Acceptance:** "MCP default-project routing can no longer
  return an unrelated repo; regression test proves it."
- **Files (≤ 4):** `gridseak-local-store/src/store.rs`
  (`resolve_project_lenient`, lines 396–420, and its existing test at
  lines 1528–1544); optionally the MCP error surface in
  `gridseak-cli/src/main.rs` (call sites already use lenient at 1388,
  1485, 1549, 1890, 2259, 2297 — the improved error propagates
  automatically); optionally `default_project_ref` doc comment
  (main.rs 1194–1202) to stop advertising the removed fallback.

### The fix (drift D7)

`resolve_project_lenient` currently, on an implicit ref (`""`, `"."`,
`"./"`) whose cwd has no project, falls back to
`latest_scanned_project()` (lines 412–414) — the exact silent
wrong-project behavior (pg-meta). Replace that fallback with an
**actionable error that lists the known projects** instead of silently
picking one:

- Keep the cwd attempt (`resolve_project(trimmed)`).
- On miss, do **not** call `latest_scanned_project()`. Instead build an
  error naming the registered projects (`self.list_projects()` exists,
  lines 524–537) so the agent/user can pass an explicit ref, e.g.:
  `"no GridSeak project at the current directory. Known projects: <name>
  (<id>), … . Pass an explicit project or run `gridseak scan .` here."`
- **Gotcha (existing test):** the test at store.rs 1528–1544 asserts the
  latest-scan fallback *succeeds* for implicit refs — it MUST be
  rewritten to assert the new error. Its `assert!(store.resolve_project(
  "definitely-not-a-project").is_err())` half (strict path) stays as-is.
- Consider whether any legitimate non-MCP caller depends on the old
  lenient fallback: `analyze_command.rs` line 30 uses lenient. Confirm
  its behavior is acceptable (it should be — an analyze against no cwd
  project should also error, not silently analyze a stranger repo). Note
  the decision in the PR.

### Regression test (new)

Add to `gridseak-local-store/src/store.rs` tests: register **two**
projects (neither at the test's cwd), complete a scan for each, then
assert `resolve_project_lenient(".")` (and `""`, `"./"`) returns `Err`
whose message contains **both** project names — proving it neither
silently picks the most-recent nor an arbitrary one.

- **Verify (observable):** `cargo +1.91 test -p gridseak-local-store`
  green, including the new test `resolve_project_lenient_errors_...`;
  the old fallback test compiles under its new assertions. `cargo +1.91
  test --workspace` still green (no other caller relied on the fallback).

---

## T6 — Plan-complete verification + `evidence/01-results.md`

- **Blocked by:** T1, T3, T4, T5. **Advances:** plan Step 5 and re-checks
  **every** acceptance criterion.
- **Files (1 + tick):** create
  `docs/workstreams/master-plan/evidence/01-results.md`; you MAY tick the
  acceptance checkboxes in `01-TRUTH_RECOVERY.md` **only for criteria you
  proved** (EXECUTION_PROTOCOL §7 permits ticking proven boxes).
- **⚠ Flag — not fully Composer-headless:** criterion 2/3 require the
  **MCP** path (`gridseak_scan` / `gridseak_context_for_llm` tools),
  which need a live agent MCP session, not a headless shell. Composer can
  fully verify the **CLI** path; the MCP re-verification (same numbers
  arrive through MCP, disclosure present) should be run by an
  agent/OWNER with the MCP server connected. Record both; do not claim
  the MCP half from CLI output alone.

### Checklist to record in `evidence/01-results.md` (before → after)

1. **Diagnosis exists** (T0): link `evidence/01-diagnosis.md`, name the
   A/B/C cause.
2. **Fidelity** (T1): run `./target/release/gridseak scan .`; read the
   report JSON `resolution_quality.measured_fidelity`. Assert
   `tier != "syntactic_only"` **and** `high_ratio_on_calls >= 0.60`.
   **Drift D3/D4:** there is no `"semantic"` tier value — report the
   literal string (`heuristic_primary` at 0.60–0.80, `authoritative` at
   ≥ 0.80). If the whole-repo ratio stalls below 0.60 despite Rust going
   authoritative, STOP and report the measured number and the
   all-languages-ratio explanation (D4) — do not redefine the gate.
   Baseline to beat: `tier=syntactic_only`, `high_ratio=0.143`
   (`high 3772 / med 97 / low 22440`).
3. **Disclosure present** (T3): report JSON
   `resolution_quality.resolution_disclosure` non-empty, one entry per
   language, ≥ 1 machine-readable `skip_reason`; and the MCP scan/status
   response carries the same array (agent-run half).
4. **F2 fixed** (T5): paste the passing regression test name and output.
5. **Full gate:** `cargo +1.91 test --workspace` green (paste tail);
   `cargo +1.91 clippy` no new warnings on touched crates;
   `cargo +1.91 fmt` clean.
6. **CLI disclosure line** (T4): paste the per-language scan output.
- **Verify (observable):** `evidence/01-results.md` contains every
  before/after number cited to a command/scan-id/report path (no
  invented numbers, EXECUTION_PROTOCOL §3), and states honestly which
  criteria passed, which are agent-only, and any that could not be met.

---

## Cross-cutting reminders for every task

- **Scope:** do only your task's plan step. Unrelated bugs → append to
  `evidence/01-notes.md` and move on (EXECUTION_PROTOCOL §1).
- **No silent failure:** paste real errors into evidence; never fake a
  verification.
- **Clean architecture:** new logic in purpose-built modules
  (`resolution_disclosure.rs`, `render/resolution_disclosure.rs`); no
  file grows past ~500 lines when it can be split; no copy-paste between
  the Layer-2 and subprocess-LSP disclosure producers — share the type.
- **Additive serialization:** every new persisted field is
  `#[serde(default)]`; new enum variants (only the named `SkipReason`
  set) handled at all exhaustive-match sites.
- **No OWNER credentials** are required by any task. The only
  non-Composer step is the **MCP re-verification** in T6 (needs a
  connected MCP agent session), flagged above.
