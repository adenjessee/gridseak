<img width="2172" height="724" alt="ChatGPT Image Jun 5, 2026, 04_24_23 PM" src="https://github.com/user-attachments/assets/5ca0a621-241f-4fdd-b2cd-d3a92f9466b7" />

# GRIDSEAK - YOUR SOFTWARE ARCHITECTURE SHOULD BE BEAUTIFUL

**Local call-graph intelligence for your codebase and your AI.**

GridSeak scans your repo, builds a deterministic import/call graph, and
exposes it through **fourteen MCP tools** (Cursor, Claude Code, Codex,
Windsurf) plus a **`gridseak` CLI**. Your agent can answer structural
questions with facts instead of grepping:

- *Who calls this?* · *What breaks if I change it?* · *Are there cycles?*
- *What should we refactor first?*

Everything runs on your machine. No cloud, no upload, no LLM tokens spent
on discovery.

<img width="848" height="818" alt="Screenshot 2026-06-05 at 4 40 23 PM" src="https://github.com/user-attachments/assets/9c3c977c-80bf-436e-9198-c2dfe1d00103" />


**Languages:** Rust, TypeScript, JavaScript, Python, Go, Java, C#, and
Salesforce Apex (plus Visualforce pages). See
[Supported languages](#supported-languages) for extensions and detail.

**Quick start:** [`60-second install`](#60-second-install) →
`gridseak setup` → ask your agent *"what's risky to refactor here?"*

## What you see

```
$ gridseak scan .
[scan] parsing 312 files, 4 languages
[scan] analysing call graph (8421 nodes, 19302 edges)
[scan] done in 31s

GridSeak — gridseak-graphengine
Health 71/100 · 312 files · 4 langs · 18 priorities (3 critical, 7 major)

Top priorities (deterministic_local_analysis · 0 LLM tokens):
  1. Module coupling: graphengine-analysis ⇄ graphengine-parsing (412 call edges)
  2. Dead code candidate: `legacy_grep_fallback` (low confidence — verify by hand)
  3. Cycle: gridseak-cli → graphengine-diagnostic → gridseak-cli (depth 3)
  ...

Tier legend
  Tier 0 (tree-sitter parsed)    — deterministic, fast
  Tier 1 (filtered grep)         — may include false positives
  Tier 3 (LSP-verified)          — deterministic, slower

Confidence caveats
  - "Dead code" findings are heuristic; quote with low confidence.
  - "Module coupling" counts include Tier 1 edges; declare the tier.

Next: gridseak setup   (wires the MCP server into your IDE)
      gridseak context --for-llm    (compact bundle for any LLM)
```

## Supported languages

`gridseak scan` auto-detects languages from file extensions (see
`graphengine-parsing/configs/*.yaml`). Override with `--lang` or
`--languages`.

| Language | Extensions | Tier 0 (tree-sitter) | Tier 3 (LSP-verified) |
| --- | --- | --- | --- |
| **Rust** | `.rs` | Full symbol + import/call graph | **Yes** — `rust-analyzer` |
| **TypeScript** | `.ts`, `.tsx`, `.mts`, `.cts` | Full | Planned (`tsserver`) |
| **JavaScript** | `.js`, `.jsx`, `.mjs`, `.cjs` | Full | Planned |
| **Python** | `.py` | Skeleton — symbols + imports; thinner call graph than Rust/TS | Planned |
| **Go** | `.go` | Parsed (import edges verified) | If `gopls` is installed |
| **Java** | `.java` | Parsed | If `jdtls` is installed |
| **C#** | `.cs` | Parsed | If OmniSharp is installed |
| **Apex** (Salesforce) | `.cls`, `.trigger`, `.apxc` | Full + framework resolver | Optional (`apex-jorje`) |
| **Visualforce** | `.page` | Discovery only — folded into the Apex scan | — |

**Not supported today:** C, C++, Kotlin, Ruby, PHP, and anything else
without a config under `graphengine-parsing/configs/`. Those files are
not discovered; there is no tree-sitter pass and no automatic coverage
for them.

**Tier 1 (filtered grep):** heuristic name matches across the repo when
a symbol is known from another language's parse. Useful but noisy —
every MCP response labels the tier; see [`LIMITATIONS.md`](LIMITATIONS.md).

## What the agent calls (the MCP surface)

GridSeak registers as an MCP server when you run `gridseak setup`.
From that moment on, your agent has fourteen tools available. Each one
is written so the agent knows *when* to call it, not just how. Every
response carries an `evidence: "deterministic_local_analysis"` marker
and a `tier_legend` — Cursor's planner reads those fields as "cheap
and trustworthy" and prefers us over re-grepping the repo.

| Tool | When the agent calls it |
| --- | --- |
| `gridseak_context_for_llm` | **First call** on a new conversation. One-shot bundle: summary + metrics + priorities + caveats + artifact paths + `next_tool` hints. |
| `gridseak_status` | Cheap probe — health + scan metadata. |
| `gridseak_scan` | Fresh parse + analysis. Only when `status` reports no recent scan. |
| `gridseak_get_recommendations` | "What should we refactor first?" / "Where's risky?" Ranked deterministic priorities. |
| `gridseak_explain_finding` | Drill into a priority by `finding_id`. Narrative + suggested action. |
| `gridseak_get_findings` | Raw unranked list, filterable by severity. |
| `gridseak_graph_blast_radius` | "If I change X, what breaks?" Transitive **upstream** callers (reverse BFS), depth-bounded. |
| `gridseak_graph_callers` / `_callees` | "Who calls X?" / "What does X call?" Direct only. |
| `gridseak_graph_slice` | Full upstream+downstream neighborhood (heavier). |
| `gridseak_graph_module_coupling` | Top tightly-coupled module pairs. |
| `gridseak_graph_cycles` | Call-graph cycles (a non-empty result is a layering smell). |

Every response carries `evidence`, `tier_legend`, and `next_tool`.
Agents that pass tier annotations + confidence caveats through to the
user get caught lying less often.

## Why it exists

Modern dev with AI feels productive because changes ship. What gets
lost is the structural ground truth — *Naur's "theory of the
program"* — that used to live in the senior engineer's head. The
agent rebuilds a partial theory every turn, and you spend the day
proof-reading it against code reality.

GridSeak gives that proof-reading job a surface:

- A deterministic call/import graph the agent can quote — no
  summarization, no hallucination, no upload.
- An honest evidence model: every edge the agent quotes is labeled
  with how it was discovered (Tier 0 tree-sitter, Tier 1 grep, Tier 3
  LSP-verified), so you know which lines to trust and which to verify
  by hand.
- A confidence-caveat protocol: when GridSeak says "low confidence"
  about a metric, the agent quotes that caveat verbatim. Agents that
  paraphrase it away get caught lying.

See [`LIMITATIONS.md`](LIMITATIONS.md) for what GridSeak **does not**
do — the single most important page for trust in a tool of this kind.

## 60-second install

**Primary (recommended):** installs `gridseak`, sidecar analyzers, and
configs into `~/.gridseak/bin`. SHA256-verified against the public manifest.

```sh
# macOS (Apple Silicon + Intel) and Linux x86_64:
curl -fsSL https://raw.githubusercontent.com/adenjessee/gridseak/main/scripts/install/install.sh | bash
# Windows:
iwr https://raw.githubusercontent.com/adenjessee/gridseak/main/scripts/install/install.ps1 -useb | iex
```

Shorter `gridseak.com` URLs work when that host mirrors the release; GitHub
is the canonical source today.

**Supported at v0.1.0:** macOS (aarch64 + x86_64) and Windows (x86_64).
Linux tarballs are not shipped yet — `install.sh` will fail with an explicit
"no artifact" message on Linux until we add them.

**From source (full workspace build):**

```sh
git clone https://github.com/adenjessee/gridseak-graphengine
cd gridseak-graphengine
cargo build --release -p gridseak-cli
# binaries land in target/release/ — see BUILD.md for PATH + sidecars
```

`cargo install --path gridseak-cli` installs the **CLI binary only** — not
`graphengine-parsing`, `ge-analyze`, or `configs/`. Use `curl | sh` or a
full workspace build for a working scan.

Then wire it into your IDE(s):

```sh
gridseak setup                # writes mcp.json + Cursor rule
gridseak setup --verify       # post-install sanity check
```

- **Cursor** + **Windsurf** — fully automated. `gridseak setup` writes
  `mcp.json` and (for Cursor) the rule file that teaches the agent
  when to call each of the fourteen MCP tools.
- **Claude Code** — prints the official `claude mcp add` command.
- **Codex** — prints the TOML snippet to paste into `~/.codex/config.toml`.

Restart your IDE. From a fresh chat, ask: *"what's risky to refactor
here?"* — your agent should call `gridseak_get_recommendations` within
its first two tool calls. If it doesn't, run `gridseak setup --verify`.

## v0.1.0 scope (honest disclosure)

Pre-1.0, solo-maintained, spare time. **Use at your own risk** — same
bargain as any indie OSS CLI you `curl | sh`. GridSeak is not a smarter
agent; it is the layer where structural claims become checkable.

- **Perspective implemented:** Reach (1–3 hop directed neighborhood).
- **Perspectives planned:** Hierarchy (v0.2), Change (v0.3).
- **Languages:** see [Supported languages](#supported-languages) above.
  Production-grade Tier 3 today is **Rust only**; other parsers may emit
  Tier 3 edges when their language server is installed, but treat those as
  best-effort until we ship them on the v1.x roadmap.
- **Install platforms (v0.1.0):** macOS (Apple Silicon + Intel), Linux
  x86_64, and Windows x86_64 via the install scripts on GitHub Releases
  (see [`docs/05-deployment/INSTALL.md`](docs/05-deployment/INSTALL.md)).
- **Telemetry:** **None.** Everything stays in `.gridseak/` on your
  machine. There is nothing to opt out of because nothing is being
  collected. You can verify this yourself: `rg -n 'reqwest|hyper|ureq'
  gridseak-cli/src/` shows zero outbound HTTP clients in the CLI.

## Repo layout

This is a Cargo workspace. Everything in it is open-source under
`MIT OR Apache-2.0`.

```
gridseak-cli/                `gridseak` binary — CLI + MCP server + setup
gridseak-engine-runner/      shared scan pipeline
gridseak-local-store/        per-project .gridseak/ filesystem layout
graphengine-parsing/         deterministic parser (tree-sitter + LSP-grade)
graphengine-analysis/        analysis passes (cycles, blast radius, hotspots)
graphengine-diagnostic/      Fix-First scoring + recommendations
graphengine-mcp/             MCP transport + tool registry
graphengine-progress/        structured progress event vocabulary
graphengine-infra/           SQLite adapter
graphengine-ra-ide-adapter/  rust-analyzer IDE-grade resolution
graphengine-git-signals/     temporal coupling from git co-change
docs/                        deployment + roadmap + evidence
```

The `graphengine-*` crates are internal libraries (the analysis stack);
users don't depend on them directly. The `gridseak-cli` crate is the
public binary.

## Build from source

```sh
git clone https://github.com/adenjessee/gridseak-graphengine
cd gridseak-graphengine
cargo build --release         # the public surface
cargo test --workspace        # ~370 tests, ~50s
```

Reproducible builds: see [`BUILD.md`](BUILD.md).

## Determinism gates

Two in-tree tests guarantee analyzer output stability; they run on
every `cargo test --workspace` and require no external fixtures:

- `cargo test -p graphengine-analysis --test determinism_integration`
  — asserts byte-identical normalised JSON across two `ge-analyze`
  runs on the same parse DB.
- `cargo test -p gridseak-engine-runner --test parity` — asserts CLI
  and any other consumer produce identical output on the same
  fixture.

## License

Licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([`LICENSE-MIT`](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option. The dual-license is the Rust ecosystem standard;
recipients pick the one that fits their downstream constraints.

The maintainer commits not to relicense this tree to more restrictive terms.
See [`LICENSE-COMMITMENT.md`](LICENSE-COMMITMENT.md).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the
Apache-2.0 license, shall be dual-licensed as above, without any
additional terms or conditions.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Solo-maintained — small, focused
PRs with tests are the fastest path to merge.

## Security

Pre-1.0, local-only, no security team. See [`SECURITY.md`](SECURITY.md) —
use a GitHub Security Advisory only for serious exploit-class issues;
everything else is a normal issue.
