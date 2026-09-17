# EXECUTION PROTOCOL — standing context for any agent executing a master-plan file

Read this WHOLE file before touching code. It applies to every plan
(01–09) in this folder. The plan file you were given defines WHAT to do;
this file defines HOW to work. When they conflict, the plan file wins.

## What GridSeak is (30-second context)

A local, deterministic structural-truth engine for codebases: tree-sitter
parse → semantic resolution → SQLite call/import graph → health analysis
→ CLI + 14-tool MCP server for AI agents. Every edge carries
`Provenance { source, confidence }`. The product's core value is
CALIBRATED HONESTY: it must never claim structure it cannot prove, and
it must disclose when it is guessing. Every change you make must
preserve or strengthen that property.

## Workspace map (the crates you will touch most)

- `graphengine-parsing/` — parsing, resolution, LSP/semantic adapters,
  SQLite storage, domain types (`src/domain/provenance.rs`, `edge.rs`).
- `graphengine-analysis/` — health report, metrics, confidence caveats.
- `gridseak-cli/` — the `gridseak` binary AND the shipped MCP server
  (`src/main.rs`), renderers in `src/render/`.
- `gridseak-engine-runner/` — scan pipeline orchestration (CLI and MCP
  share this path).
- `graphengine-ra-ide-adapter/` — in-process rust-analyzer (Rust Layer-2).
- `docs/workstreams/master-plan/` — the plans, this protocol, and
  `evidence/` where you record results.

## Toolchain and commands (use EXACTLY these)

- Rust toolchain: `cargo +1.91` for every build/test/clippy invocation.
- Full gate: `cargo +1.91 test --workspace` (must be green before you
  declare any step done).
- Targeted: `cargo +1.91 test -p <crate> --test <file>` while iterating.
- Lint: `cargo +1.91 clippy -p <crate>` on touched crates; fix new
  warnings you introduced; do not mass-fix pre-existing ones.
- Format: `cargo +1.91 fmt` before finishing.

## Git discipline

- Work on the current feature branch; NEVER push, never force-anything.
- Commit only when your plan step's acceptance criteria pass, with a
  message naming the plan: e.g. `plan-01: add ResolutionDisclosure to
  scan report`. One logical change per commit.
- Never commit: secrets, `~/Library` artifacts, scan databases, or
  files outside the repo.

## Working rules (non-negotiable)

1. SCOPE: do only what your assigned plan step says. If you notice an
   unrelated bug, record it in `evidence/<plan>-notes.md` and move on.
2. NO SILENT FAILURE: if a command fails, paste the actual error into
   your evidence notes and either fix it (if in scope) or stop and
   report. Never fake, skip, or approximate a verification step.
3. NO INVENTED NUMBERS: any metric you report must come from a command
   output, a test result, or a report JSON — cite the artifact path or
   scan id next to the number.
4. CLEAN ARCHITECTURE: new logic goes in purpose-built modules; define
   ports (traits) before adapters; no file grows past ~500 lines when
   logic can be split; no copy-paste between adapters — extract shared
   modules.
5. ADDITIVE SERIALIZATION: new fields on persisted types use
   `#[serde(default)]`; never break old parse.db reading. New enum
   variants must handle the exhaustive-match sites the plan names.
6. TESTS ARE THE DEFINITION OF DONE: every behavior change lands with a
   test that fails before and passes after. Snapshot/fixture tests must
   not require network or optional tools unless explicitly gated.
7. PLAN FILES ARE READ-ONLY to you, except: ticking acceptance-criteria
   checkboxes you have PROVEN, and appending to `evidence/` files.
8. STOP CONDITIONS: stop and report (do not improvise) when — an
   acceptance criterion cannot be met as written; a dependency plan's
   output is missing; a change would touch >3 crates when the plan
   implies fewer; anything requires OWNER accounts/credentials.

## Evidence and reporting

- Each work session appends to
  `docs/workstreams/master-plan/evidence/<NN>-notes.md`: date, step,
  what was done, commands run, results (pasted), open questions.
- Completed plan steps: record before/after numbers in the plan's
  designated results file (e.g. `evidence/01-results.md`).
- Your final message each session: what passed, what failed (verbatim),
  what remains, next recommended step. No overstated success — the
  master plan's standing rules (00-MASTER_PLAN.md §7) bind you.

## Verification habits for this codebase

- After touching resolution/provenance: run
  `cargo +1.91 test -p graphengine-parsing` AND rebuild the CLI and run
  a real scan: `cargo +1.91 build -p gridseak-cli --release &&
  ./target/release/gridseak scan . 2>&1 | tail -30`, then check the
  report JSON's `resolution_quality` block (path is printed by the scan;
  reports live under `~/Library/Application Support/com.gridseak.desktop/project-reports/`).
- After touching the MCP surface: run the scan through the MCP tool if
  available in your session, or at minimum the CLI path, and confirm
  response shape.
- The baseline truth for comparisons is
  `evidence/2026-07-01-dogfood-baseline.md` (scan `7d3f8035…`,
  `tier: syntactic_only`, `high_ratio_on_calls: 0.143`).
