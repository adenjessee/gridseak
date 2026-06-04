# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once we cut 1.0. Before 1.0, we may break APIs in minor versions; we
will call those out explicitly here when they happen.

## [Unreleased]

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
