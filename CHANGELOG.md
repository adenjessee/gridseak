# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once we cut 1.0. Before 1.0, we may break APIs in minor versions; we
will call those out explicitly here when they happen.

## [Unreleased]

## [0.1.3] — 2026-10-14

Python and Rust join Go and TypeScript at the compiler tier, each with an
independent answer-key checker, and health metrics count direct calls.

### Added

- Python at the compiler tier (scip-python, pinned 0.6.6). Class
  instantiation reaches `__init__`/`__new__` along the MRO; redeclared names
  (`@overload`, property setters, platform branches) no longer create false
  edges; star-imported names resolve to the right class; abstract and
  `Protocol` calls are dispatch. Calls the index cannot type are recorded,
  and `gridseak audit` skips targets they could affect. Answer keys match a
  mypy-based oracle and a test-suite runtime witness on Click, Flask and
  httpx (`tests/recall-audit/py-oracle/`, evidence 21: with GridSeak 45/45
  exact vs 39/45 alone).
- Rust at the compiler tier: `rust-analyzer scip` plus the reference sweep,
  with the in-process rust-analyzer resolver as a visible fallback.
  Owner-qualified method names, trait calls through `dyn`/generics as
  dispatch, closures credited to the enclosing fn. On GridSeak's own code
  every call edge now comes from the compiler (was 18%), and a rustc-MIR
  oracle (`tests/recall-audit/rust-oracle/`) finds 99.9% direct-call recall
  at 99.4% precision.
- `gridseak audit --plan <tasks.json>` reuses an earlier plan so repeats
  merge into one card.
- Claim check at the end of an agent turn: `gridseak gate --stop`, wired as
  the Claude Code plugin's `Stop` hook. When the agent's final message
  claims completeness ("updated all callers"), the working tree is diffed
  against `HEAD`; for every function removed, renamed or re-signatured
  (Go, Rust, TypeScript/JavaScript, Python), each graph caller must sit in a
  changed file and no longer mention a removed name. Compiler-witnessed
  misses block the stop and hand the agent the callers with file:line;
  tree-sitter-only misses and `git grep -w` text matches outside the graph
  (Go `_examples/`, untyped Python calls) are shown without blocking. Never
  blocks twice in a row; no claim, no scan or any error returns `{}`. On
  chi a partial `RouteContext` rename lists all 4 missed callers (`go vet`
  shows 2); on Flask a partial `ensure_sync` rename lists the 7 functions
  holding all 8 leftover references.

### Changed

- Health metrics count direct calls. Cycles, tangle, fan-in / hotspots,
  blast radius, call depth, coupling, cohesion, layers and API surface now
  leave out Call edges the compiler index tagged as interface/trait dispatch
  (fanned out to every implementation) or as a function used as a value.
  Dead code still counts those uses as callers, so trait impls and callbacks
  are not reported dead. Reports carry the
  `metrics_edge_basis_direct_calls_v1` caveat and
  `integrity_status.metrics_edge_basis` (excluded edge counts, plus cycles
  that close only through dispatch / value uses, listed as information).
  `metrics_edge_basis = "all_uses"` in an analysis config restores the old
  counting. On GridSeak itself this removes three fake cycles where
  `DisclosedSemanticResolver` forwards to the resolver it wraps. The
  `gridseak graph` fan-in, hotspots, module-coupling and cycles queries use
  the same basis.
- Indexers installed through npx are pinned: scip-python 0.6.6,
  scip-typescript 0.4.0 (the versions the answer keys were validated with).
- The audit recognises plain-worded incompleteness warnings ("I couldn't
  produce the complete list") as flagged, not silent, misses.

### Fixed

- Scans skip nested git checkouts (agent worktrees, submodules, cloned
  repos) and `.claude/` / `.gridseak/`. Claude Code agent worktrees were
  being indexed as extra copies of the repo.
- A scan restricted to some languages drops the parse-DB rows of languages
  it leaves out, so an earlier full scan's heuristic edges no longer linger.
- Cached files are re-resolved, and the analysis rebuilt, when a file's
  resolution tier changes.

## [0.1.2] — 2026-10-12

Compiler-grade call graph, measured against the compilers themselves,
and answers that say when they are complete.

### Added

- `gridseak audit`: grades a coding agent on this repository against
  compiler truth. It plans "every transitive caller of X" questions (small to
  large) from the call graph, runs them through headless Claude Code alone and
  with GridSeak's MCP tools, and writes a report card (`report.html`,
  `report.md`, `results.json`): exact answers, silent misses (wrong lists
  presented as complete), minutes, tokens and cost per correct answer.
  `--reps`, `--compare` (one card for several models), `--plan` (reuse a plan),
  `--family edits` (Go interface changes graded by hidden compile-time
  assertions, `go build` and `go vet` in a throwaway copy). Refuses to run when
  the answer key would not be fully compiler-backed. On Caddy, chi and
  Kubernetes, agents with GridSeak answered 90/90 exactly against 78/90 alone.
- Claude Code plugin: `/gridseak:audit` and an `audit-my-agent` skill, so
  "audit my agent on this repo" starts the flow (plan, cost, confirmation,
  report). Marketplace manifest at `.claude-plugin/marketplace.json`.
- MCP tools accept `path/to/file.ts:123` and `path/to/file.ts::name` symbols,
  so common names (`fetch`) resolve.
- `gridseak pr-check`: compares the call graph before and after a change
  and flags callers left pointing at a removed or renamed symbol. 0 false
  alarms on 113 merged pull requests; 46 of 46 seeded breaks caught.
- Blast-radius certificate (MCP): `certificate.complete` is true only when
  every call edge came from the compiler index, no source file changed
  since the scan, and the walk hit neither the depth limit nor the row
  cap. Otherwise `reasons` names each failed check.
- Call edges record how the callee is used (`call`, `dispatch` through an
  interface, `value` passed or stored). `blast-radius --uses` and the MCP
  `uses` parameter take `all` (default), `calls` or `static`.
- `GRIDSEAK_SKIP_IMPORT_HEURISTICS` opts out of the heuristic import pass
  on very large repositories.
- Evidence docs 13–19 under `docs/workstreams/master-plan/evidence/` and
  the recall-audit and graph-benchmark harnesses under `tests/`.

### Changed

- The compiler tier reads every SCIP reference occurrence instead of
  mapping call sites one at a time. Caller recall against `go/types` is
  100% on chi, Caddy and Kubernetes (3.5M lines, indexed in 11.5 min);
  97.4–98.5% of TypeScript functions have every caller listed against
  `tsc` (zod, hono).
- Go methods are named `Receiver.Method`, so methods and functions with
  the same name no longer collide.
- Top-level uses (Go `var x = f()`, TypeScript module scope) are credited
  to the enclosing declaration, so those callees no longer look unused.
- scip-go pinned to 0.2.7 (0.1.25 crashed on Caddy). Python repositories
  index without a root marker file.
- MCP: caller, callee and blast-radius rows carry file and line; symbols
  resolve from Go import paths and `file::Name`; stale-snapshot responses
  cap the listed paths instead of listing every changed file.
- `graph blast-radius` help text now says it walks upstream (callers).

### Fixed

- A cached SCIP index is reused only when its fingerprint matches this exact
  workspace and its sources. A copied, moved or stale index is refused (it
  previously produced an empty graph for Kubernetes from Caddy's index), and
  any fallback to tree-sitter edges is printed and recorded as the report
  caveat `compiler_tier_unavailable_v1`. Existing caches are rebuilt once.
- TypeScript: function nodes for class fields, `#private`, string-named and
  keyword-named methods; no id collisions between identical bodies in
  different classes; `super()`, `new X()` to an inherited constructor and
  `this["name"]()` are calls; top-level code is no longer listed as a
  function caller (MCP rows now carry `kind` and split out
  `top_level_callers`).

- Import resolution on Kubernetes-size Go repositories no longer scans the
  whole graph per import (children index plus a path cache).

## [0.1.1] — 2026-09-16

Cursor edit-boundary gate and stranger-proof setup. This is the first
CLI that has `gridseak gate`. `cli-v0.1.0` does not.

### Added

- `gridseak gate` for Claude `PreToolUse` and Cursor `preToolUse`
  (`Write|StrReplace|Delete|Shell`) plus `beforeShellExecution`.
- Same-name preferred-path deny (Go interface `Use` vs `Mux.Use`).
- `setup --verify` rejects a gateless or non-absolute hook binary and
  an empty Claude `PreToolUse` array.
- `scripts/stranger-repro.sh` — fresh-HOME tarball install + NewRouter
  fixture deny (not `curl | sh`).
- Root `LICENSE` (MIT OR Apache-2.0).

### Changed

- Cursor 3.20.21 honours three hook layers: user `hooks.json`, plugin
  manifests, and `~/.claude/settings.json`.
- Workspace version `0.1.0` → `0.1.1`.

## [0.1.0] — 2026-06-03

First public open-source release: the fourteen-tool agent-first MCP surface
and a CLI with the same coverage, promoted from the private `0.1.0-rc1`
after dogfooding. Dual-licensed MIT/Apache-2.0. No paid SKU, no telemetry,
no network calls from the on-machine binaries.

### Added

- Public install via GitHub-native `curl -fsSL
  https://raw.githubusercontent.com/adenjessee/gridseak/main/scripts/install/install.sh
  | bash` with SHA256-pinned manifest on GitHub Releases (`cli-v*` tags).
- [`docs/05-deployment/RELEASE.md`](docs/05-deployment/RELEASE.md) —
  CLI vs legacy engine release channels.
- Segmented incremental analysis (S2-γ): per-segment extraction with an
  L1/L2 merge trust ladder keyed on a structure fingerprint.
- `gridseak_graph_file_blast_radius` MCP tool — file-level reverse
  reachability ("if I change this file, what breaks?"), the whole-file
  companion to the symbol-level `gridseak_graph_blast_radius`.
- `gridseak_route` MCP tool — intent meta-router that maps a
  natural-language goal to the GridSeak tool to call next. The MCP
  surface is now **fourteen tools**.

### Changed

- Workspace version `0.1.0-rc1` → **`0.1.0`** for public OSS launch.
- Canonical clone URL: `github.com/adenjessee/gridseak`.
- Install docs: primary = `curl | sh`; `cargo install --path gridseak-cli
  --locked` is CLI-only and **requires `--locked`** (an unbounded
  `tree-sitter` floor in the `tree-sitter-rust` grammar breaks unlocked
  resolution until a coordinated grammar bump).
- Platform matrix: macOS (arm64 + Intel), Linux x86_64, and Windows
  x86_64 at launch; Linux arm64 not yet.

### Fixed

- Layer-2 (rust-analyzer) semantic resolver no longer panics with rowan
  `Bad offset` when a computed source offset overruns the file. The
  adapter now bounds-checks the offset at the `FilePosition` boundary and
  falls back to the heuristic resolver, so a single bad reference can no
  longer abort an entire `gridseak scan`.

---

## [0.1.0-rc1] — 2026-05-24 (private release candidate, tagged on origin only)

First release candidate. Twelve-tool agent-first MCP surface plus a
CLI with the same coverage. No paid SKU, no telemetry, no upload.

### Added

- **Twelve-tool MCP server** registered as `gridseak` via
  `gridseak setup`. Tool descriptions are written for the agent (symptom-led
  openings, trigger phrases per tool, deterministic + 0-token markers,
  tier_legend in every response). Tools:
  - `gridseak_context_for_llm` — one-shot cold-start bundle
  - `gridseak_status`, `gridseak_scan`
  - `gridseak_get_recommendations`, `gridseak_explain_finding`,
    `gridseak_get_findings`
  - `gridseak_graph_blast_radius`, `gridseak_graph_callers`,
    `gridseak_graph_callees`, `gridseak_graph_slice`,
    `gridseak_graph_module_coupling`, `gridseak_graph_cycles`
- **`gridseak setup`** multi-IDE wiring (Cursor + Windsurf
  auto-write; Claude Code + Codex print copy-paste instructions).
  Auto-writes the Cursor rule (`~/.cursor/rules/gridseak.mdc`) that
  teaches the agent when to call each tool.
- **`gridseak setup --verify`** post-install sanity check (mcp.json
  references binary, binary is on PATH, rule file present).
- **`trace-internal` Cargo feature** in `gridseak-cli`. Default
  (public) builds expose only the analysis MCP surface; the visual
  view's MCP wiring is gated behind this feature and is out of scope
  for v0.1.0.

### Changed

- MCP response envelope is now consistent across all twelve tools:
  `{ result, next_tool, evidence: "deterministic_local_analysis",
  tier_legend: {…} }`. The `evidence` marker is the agent-planner
  bait — Cursor's planner reads it as "cheap and trustworthy" and
  prefers GridSeak over re-grepping.
- Slimmed the MCP tool surface from ~26 to 12. Dropped legacy
  aliases (`gridseak_get_top_recommendations`,
  `gridseak_get_latest_scan`, etc.) that duplicated the kept tools
  and confused agent planners.
- Cursor rule renamed from `.cursor/rules/shadow-mode.mdc` to
  `.cursor/rules/gridseak.mdc` and rewritten to focus on the twelve
  analysis tools with concrete trigger phrases per tool.
- CLI command renamed: `setup-cursor` → `setup` (the analysis MCP is
  now the default `setup` target; the visual view's setup is gated
  behind `trace-internal`).

### Removed

- The **Tauri desktop shell** (`desktop/`). The desktop pilot's
  license-activation deep link, telemetry signing, and license refresh
  task are all retired. The replacement surface is the `gridseak` CLI
  + MCP server, which provides every non-license capability the
  shell exposed.
- The **`graphengine-license`** crate. Implemented Ed25519 JWT
  verification and feature-flag gating for the retired paid desktop.
  Open-sourcing the analyzer is the wrong context to gate it
  on-machine.
- The **`graphengine-license-mint`** crate. The CLI that minted JWTs
  for the retired license system.
- The **`graphengine-telemetry`** crate. Tamper-evident signed event
  emitter for the retired paid desktop. The new product policy
  forbids on-machine telemetry; there is nothing left to emit.
- The dead-code file `graphengine-infra/src/config.rs` — an orphan
  module hard-coding a path under the retired desktop layout.
- `.github/workflows/desktop-release.yml` and the entire
  `scripts/release/` directory — release pipeline for the retired
  Tauri shell. CLI install scripts live in `scripts/install/`.

### Security

- **Audit of cryptographic material in git history.** As part of
  retiring the license system, we audited every PEM / signing key
  ever committed to this repo. The summary:

  | File | Was in git history? | Notes |
  |---|---|---|
  | `desktop/src-tauri/keys/license_private_DEV_ONLY.pem` | No | Generated locally at build time. Never tracked. |
  | `desktop/src-tauri/keys/telemetry_seed.bin` | No | Generated locally at build time. Never tracked. |
  | `desktop/src-tauri/keys/license_public.pem` | No | Local-only public key file. Never tracked. |
  | `graphengine-license-mint/tests/fixtures/golden_keypair_DEV_ONLY.pem` | **Yes**, at commit `411f8b6` | Ed25519 test-fixture keypair used for the JWT-mint roundtrip test. Never used to sign any production artifact. |
  | `graphengine-license-mint/tests/fixtures/golden_keypair_DEV_ONLY.pub` | **Yes**, at commit `411f8b6` | Public half of the same fixture keypair. |

  The two fixture PEM files remain recoverable from history. This is
  acceptable because:

  1. They were created as test fixtures, never used in any signed
     binary or signed license.
  2. The license system that would have given them operational
     meaning is retired in this same release.

  No rotation is meaningful and no history rewrite is planned.

### Telemetry, network, and data — what is unchanged

- **Zero telemetry.** No code path in the on-machine binaries
  reaches the network. You can verify yourself with
  `rg -n 'reqwest|hyper|ureq' gridseak-cli/src/`.
- **MCP is stdio.** The server talks JSON-RPC over stdin/stdout
  with your IDE. No sockets are opened.
- **No anonymous metrics.** Not even "version installed."
