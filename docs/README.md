# GridSeak — Documentation Index

GridSeak is a deterministic structural-knowledge layer for AI-era
codebases. The primary surface is the **MCP server** (fourteen tools,
local, 0 LLM tokens) that your coding agent calls. The **CLI** is the
secondary surface for scripting, CI, and direct human use. Both ship as
the single `gridseak` binary built from `gridseak-cli`.

The docs in this folder are engineering / product docs. The top-level
user-facing docs live at the repo root:

- [`README.md`](../README.md) — install, MCP surface overview, quickstart
- [`BUILD.md`](../BUILD.md) — build from source, reproducible-build status
- [`LIMITATIONS.md`](../LIMITATIONS.md) — what GridSeak does **not** do
- [`CHANGELOG.md`](../CHANGELOG.md) — release notes
- [`CONTRIBUTING.md`](../CONTRIBUTING.md) — how to send a PR
- [`SECURITY.md`](../SECURITY.md) — how to report a vulnerability
- [`THIRD-PARTY.md`](../THIRD-PARTY.md) — dependency attribution

---

## 03-specs — What the system computes

| Doc | Purpose |
|---|---|
| [GE_ANALYZE_FULL_SPECIFICATION.md](./03-specs/GE_ANALYZE_FULL_SPECIFICATION.md) | The keystone spec for the `ge-analyze` engine. |
| [GE_ANALYZE_EXTENDED_SPECIFICATION.md](./03-specs/GE_ANALYZE_EXTENDED_SPECIFICATION.md) | Extended capabilities on top of the keystone. |
| [STRUCTURAL_HEALTH_COMPLETE_VISION.md](./03-specs/STRUCTURAL_HEALTH_COMPLETE_VISION.md) | Structural-health metric measurement, scoring, extensions. |
| [METRICS_REFERENCE.md](./03-specs/METRICS_REFERENCE.md) | Complete list of metrics measured by `ge-analyze`. Cited from `LIMITATIONS.md` and the CLI metrics renderer. |
| [SIZE_BIAS_AND_NORMALIZATION.md](./03-specs/SIZE_BIAS_AND_NORMALIZATION.md) | Size-bias diagnosis per metric plus the exact normalisation formulae. |
| [CLASSIFICATION_SYSTEM_DESIGN.md](./03-specs/CLASSIFICATION_SYSTEM_DESIGN.md) | File/folder classification design. |
| [PERFORMANCE_TESTING_FRAMEWORK.md](./03-specs/PERFORMANCE_TESTING_FRAMEWORK.md) | Performance-testing framework and benchmarking methodology. |

Some specs in `03-specs/` describe paused or pre-pivot surfaces
(`ge-template`, desktop view-roots). Each such doc says so inline —
verify against the current MCP / CLI surface before treating it as
load-bearing.

## 04-architecture — How the system is built

| Doc | Purpose |
|---|---|
| [EDGE_TAXONOMY.md](./04-architecture/EDGE_TAXONOMY.md) | The authoritative edge-kind taxonomy (`Call`, `Import`, `Type`, `Uses`, `Contains`, `Extends`, `Implements`, `Framework`, `Declarative`). |
| [LSP_INTEGRATION_PLAN.md](./04-architecture/LSP_INTEGRATION_PLAN.md) | LSP integration for semantic accuracy alongside Tree-sitter. |
| [adr/](./04-architecture/adr/) | Architecture decision records. |

## 05-deployment — How it runs on a user's machine

| Doc | Purpose |
|---|---|
| [GRIDSEAK_CLI_MCP_SETUP.md](./05-deployment/GRIDSEAK_CLI_MCP_SETUP.md) | CLI + MCP setup walkthrough. |
| [CURSOR_MCP_MIGRATION.md](./05-deployment/CURSOR_MCP_MIGRATION.md) | Migration guide for older Cursor MCP installs. |

## workstreams — Active engineering effort

Each workstream folder is self-contained (reference + runbook + plan +
results for one effort).

| Workstream | What |
|---|---|
| [apex/](./workstreams/apex/) | Salesforce Apex integration. |
| [universal-fidelity/](./workstreams/universal-fidelity/) | Universal-fidelity layered architecture (Layer 0–4). |
| [layer2-adapters/](./workstreams/layer2-adapters/) | Per-language Layer-2 (semantic) adapter implementations. |
| [proof-foundation-gap/](./workstreams/proof-foundation-gap/) | Pre-registered experiment testing engine predictions vs observed codebase behaviour. |
| [runtime-oracle/](./workstreams/runtime-oracle/) | Runtime-oracle workstream (early stub). |
| [adversarial-corpus/](./workstreams/adversarial-corpus/) | Adversarial-corpus workstream (early stub). |
| [ground-truth-datasets/](./workstreams/ground-truth-datasets/) | Ground-truth-dataset workstream (early stub). |

---

## Crate-level docs (outside `docs/`)

| Crate | Docs |
|---|---|
| `graphengine-parsing/` | [README](../graphengine-parsing/README.md), [CONTRIBUTING](../graphengine-parsing/CONTRIBUTING.md), [CHANGELOG](../graphengine-parsing/CHANGELOG.md) |
| `graphengine-infra/` | [ENGINE_ADAPTER_IMPLEMENTATION_SUMMARY](../graphengine-infra/ENGINE_ADAPTER_IMPLEMENTATION_SUMMARY.md), [TEST_COVERAGE_SUMMARY](../graphengine-infra/TEST_COVERAGE_SUMMARY.md) |
