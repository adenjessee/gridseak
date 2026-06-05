# GridSeak — Limitations

This document lists what GridSeak **does not** do, **cannot** do
today, and the cases where its output is approximate rather than
authoritative.

If our competitors do not have a document like this, that is a signal
about how seriously they take your trust. If you find a limitation we
should add here, please open an issue or PR — we want this list to be
exhaustive, not flattering.

## Languages

### Tree-sitter import edges (Tier 0)

Tier 0 edges — the ones you should trust most outside of
LSP-verified Tier 3 — are available for:

- **Rust** (`use` statements, `pub use` re-exports)
- **TypeScript / JavaScript** (`import` statements, `require()` calls
  when they are the direct argument)
- **Python** (`import` and `from … import …`)

For other languages we **fall back to Tier 1 (filtered grep)**.
GridSeak (CLI and MCP) always tells you which tier you're looking
at — there is no "secret degraded mode." Every MCP response carries
a `tier_legend` field; quote it when stating a structural fact.

### Tier 1 filtered grep (all languages)

Tier 1 edges are computed by grepping for the symbol name with these
filters applied:

- Word boundaries (so `foo` does not match `foobar`)
- Common-name filter (so `new`, `get`, `set`, `data`, `value`, `i`,
  `index` etc. do not produce a thousand spurious edges)
- Saturation cap (if a symbol appears in more than 50 files in
  a project, we report the cap reached and do not enumerate every
  match)

This means Tier 1 can have **false positives** (a symbol name that
happens to appear in a string literal or a comment) and **false
negatives** (an aliased import the grep cannot follow). The
`tier_legend` in every MCP response calls these edges out as
"may include false positives" — quote that caveat when stating a
Tier 1 fact.

### LSP-grade verification (Tier 3)

Tier 3 (`━━` thick bright-green) is only available when the relevant
language server is installed and reachable, and currently only for
Rust (via `rust-analyzer`). Tier 3 for TypeScript (`tsserver`) and
Python (`pyright` / `pylsp`) are on the v1.x roadmap.

When Tier 3 is not available, you still get Tier 0 / 1 — you just
do not get the highest-strength edges. The CLI's report and the MCP
response's `tier_legend` both note when Tier 3 is unavailable.

### LLM-reviewed edges (Tier 2)

Tier 2 (LLM-reviewed) is not part of v0.1.0. It requires an IDE-side
MCP `sampling` capability that we will use when the visual view ships
publicly in a later release. Until then, GridSeak emits Tiers 0, 1,
and 3 only — fully deterministic.

## Scale

- We have validated on repositories up to **~500k lines of code** and
  **~50k symbols**. Beyond that we have not tested and you may see
  performance degrade.
- Graph queries (blast radius, slice) are depth-capped (default 3 hops,
  soft cap ~200 nodes) so an agent doesn't accidentally pull in the
  entire transitive closure of a popular symbol.
- The full repo scan is single-process and not parallelized across
  machines. Distributed scanning is a future Team-tier (paid SaaS)
  feature; this repo is the single-machine surface.

## Incremental scanning (S1)

GridSeak **incrementally re-parses changed files** on rescan. Each
discovered file is blake3-hashed at scan time; unchanged files reuse
their cached extraction slice from the project's persistent parse DB
(`{cache_dir}/parse-dbs/{project_id}/parse.sqlite`). The progress
line reports the outcome:

```text
[gridseak] incremental: 461 files (459 cached, 2 reparsed)
```

**What is deterministic and correct:**

- Unchanged files skip re-extraction. Changed and newly-added files
  are re-extracted; deleted files are pruned from the graph and cache.
- **Cross-file edges stay honest** because we cache unresolved
  *references*, not resolved *edges*. Resolution runs end-to-end on
  the merged reference set every scan — an unchanged caller that
  references a symbol added in a changed file gets a fresh edge.
- Pre-extract graph pruning removes stale node rows for changed or
  deleted files before re-extraction (node ids change when file
  content changes).

**What is approximate or incomplete:**

- The **analysis pass still runs in full** every scan. Parse-layer
  incrementalism reduces subprocess work on warm rescans; total
  `gridseak scan` wall-clock may still be dominated by analysis on
  large repos.
- **`apex_class_symbols` rows are not pruned** when an Apex file is
  deleted. The main graph and `file_cache` are pruned; Apex class
  symbol rows for `.cls` / `.trigger` paths are removed by API name
  on incremental prune (S2-γ). Other Apex layouts may still linger
  until a schema bump or manual cache clear.
- Use **`gridseak scan --no-incremental`** to force a full reparse
  (skips cache reuse). Pair with deleting the parse DB under
  `{cache_dir}/parse-dbs/` if you suspect corruption.
- **Stale apex `file_cache` slices** from before the managed-package
  fix (pre–S2-γ) can leave orphan per-file graph fragments. Run
  **`gridseak scan . --no-incremental`** once after upgrading, or
  delete `{cache_dir}/parse-dbs/{project_id}/` and rescan.

See [`docs/02-strategy/S1_INCREMENTAL_SCANNING_DESIGN.md`](docs/02-strategy/S1_INCREMENTAL_SCANNING_DESIGN.md)
for the full design rationale.

## Incremental analysis (S2-β)

GridSeak separates three concepts agents must not conflate:

1. **Snapshot** — the graph and report from the last completed `scan_id`
   (working tree at scan completion).
2. **Readiness** — whether analysis segments have finished (`analysis_complete`
   on MCP envelopes). Graph tools may work while analysis is still partial;
   `gridseak_get_recommendations` is blocked until analysis completes.
3. **Workspace delta** — files changed on disk since the scan (`workspace_delta`
   on every MCP envelope). If a structural query targets a dirty file, MCP
   returns **`STALE_SNAPSHOT`** (hard error) — run `gridseak_scan`, do not grep.

**Analysis modes** (progress line + `parse_meta.analysis_status` + `analysis_provenance`):

- **L0 / Zero reuse** — no parse delta; cached report reused (< 5 s analysis phase).
- **L1 / Segmented sync** — small delta, FP-topology unchanged; global segments reused, complexity rerun; caveat `incremental_analysis_segments_merged_v1`.
- **L2 / Segmented sync** — small delta, call graph changed; wiring detectors rerun; caveat `incremental_analysis_structure_changed_v1`.
- **L3 / Full** — large delta or `--full-analysis`; same as cold analysis.

Optional: `gridseak analyze --background` pre-warms segment cache **after** a successful `gridseak scan`. Running analyze without a fresh scan can serve stale graph/stats — see LIMITATIONS.

Deterministic routing: `gridseak route "<question>"` or MCP `gridseak_route`.
Install routing triggers via `gridseak setup` (`gridseak-routing.mdc`).

See [`docs/02-strategy/S2_INCREMENTAL_ANALYSIS_DESIGN.md`](docs/02-strategy/S2_INCREMENTAL_ANALYSIS_DESIGN.md).

## Correctness

### What we believe is deterministic

- **Tier 0 edges** are deterministic given the same source tree.
- **Tier 1 edges** are deterministic given the same source tree.
- **Tier 3 edges** are deterministic given the same source tree
  **and** the same `rust-analyzer` version. Different `rust-analyzer`
  versions may resolve differently.
- All analyses (cycles, blast radius, hotspots, dead code, etc.) are
  deterministic given the same input — we have a
  `determinism_integration` test that gates this.

### What we believe is approximate

- **Confidence scores** on analyses (e.g., "cycle confidence: medium")
  are heuristic. Read the [confidence caveats](docs/03-specs/METRICS_REFERENCE.md)
  block in the scan output for the per-metric confidence story. The
  MCP response embeds these caveats; agents are instructed to quote
  them verbatim.

### Known correctness gaps

- **Generic-heavy Rust** (deeply parametric trait code) may produce
  Tier 3 edges that look correct but miss instantiation-specific
  facts. We are tracking this in the universal-fidelity sprint.
- **Macro-generated code in Rust** is mostly invisible to tree-sitter
  (Tier 0) and grep (Tier 1). Tier 3 (LSP) sees post-expansion code
  and can produce confusing edges when you expect to see pre-macro
  code.
- **Dynamic dispatch** (trait objects, function pointers, dynamic
  imports) is approximate by definition. We do not attempt
  whole-program devirtualization in GridSeak.

## Telemetry, network, and data

- **No telemetry.** Zero events leave your machine from any
  on-machine binary in this repo. You can verify with `rg -n
  'reqwest|hyper|ureq' .` — the only outbound HTTP code path lives
  in the optional `gridseak.com/install.sh` wrapper script, which
  is itself fully visible bash.
- **No code is uploaded.** The MCP server runs over stdio with the
  IDE on the same machine; no network sockets are opened by the MCP
  server.
- **No anonymous metrics.** Not even "version installed." We honestly
  do not know how many people use this; we accept that trade-off
  for the trust contract.
- **No automatic updates.** You update when you want to. Run
  `cargo install --path gridseak-cli --locked` to update from source,
  or re-download the installer.

## Operating system support

- **macOS** (Apple Silicon and Intel): Tier-1 supported.
- **Linux** (x86_64, glibc 2.31+): Tier-1 supported.
- **Windows**: Builds and runs but is less tested.
- **WSL2**: works.

## Privacy of the local ledger

The `.gridseak/` directory in each scanned project contains scan
artifacts, MCP-call audit logs, and per-project tuning state.

**These files contain your code, your timestamps, and your scan
artifacts.** They never leave your machine, but they are unencrypted.
If your machine is shared, treat `.gridseak/` as sensitive the same
way you treat your `.git/` directory or your `node_modules/`.

We intentionally chose plain JSONL over a database for one reason:
you can read it with `cat`. The cost is that we cannot pretend it is
encrypted at rest.

## Free vs paid

- **The whole on-machine experience is free and MIT-licensed.**
  There is no "pro version" of any single-machine surface and no
  "premium analysis pass" gated by a license check. Single-machine
  analysis is commoditizing; gating it is the wrong move.
- **What is paid is the SaaS layer:** multi-repo Team-tier sync,
  CI integration, judgment synchronization between teammates,
  enterprise SAML/SSO, audit log retention. None of that code is
  in this repo; it lives in a separate hosted service.
- **You can absolutely run a personal multi-machine setup** by
  rsync'ing the `.gridseak/` directories yourself. That is fully
  supported. The paid SaaS is for *teams* who need
  conflict-resolution, RBAC, and an audit trail.

## What this tool will never become

These are decisions, not gaps. We have considered each and
deliberately rejected it. See the breakout plan §12 for the longer
form.

- **It will not become a "platform"** with plugins, marketplaces, or a
  shell embedding other tools.
- **It will not call home from the on-machine binary.** Ever.
- **It will not auto-update.** Ever.
- **It will not generate code or refactor for you.** That is the
  agent's job. We render and verify; we do not write.
- **It will not surface a "score"** as the primary output. Aggregate
  scores hide the underlying facts and tempt people to game them.
  We show counts, edges, and confidences.
- **It will not require an account** to use the open-source binary.

## How to challenge this document

If a limitation is missing: open an issue with label `docs: limitations`.
I'd rather list the gap here than have you discover it the hard way.

If a statement is too pessimistic (we say "don't" but we do): same label,
`docs: limitations`.

If a statement is too optimistic (we say "do" but we don't): label `bug` —
that's a trust bug, not just docs.
