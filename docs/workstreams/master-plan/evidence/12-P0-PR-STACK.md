# P0 — three reviewable PRs (do not dump the dirty tree)

Source branch: `plan-01-truth-recovery` plus the working tree that
landed SCIP/G1, agent tools, and the unskippable gate. Target: `main`.

Do **not** open one 180-file PR. Stack these three, in order.

Hold (omit from all three): Apex god-file splits, observatory / video
engine, vector-doc / type→pgvector work, and the previous stale
`10-H-results.md` text (rewritten in this batch to match G1 evidence).

## PR 1 — already-committed plan-01

Opened: https://github.com/adenjessee/gridseak-graphengine/pull/25

The 16 commits already on `plan-01-truth-recovery` that are not SCIP,
not agent-ab, and not the gate. Open this first **if CI is green**.

Includes foundation recovery that is already history: parsing
disclosure, health pipeline, local store, CLI scan/report/setup
(pre-gate), MCP contract, docs `00-MASTER_PLAN` / `10-A`…`10-G`
evidence that is already committed.

## PR 2 — SCIP + caret + G1

Opened: https://github.com/adenjessee/gridseak-graphengine/pull/26 (base: `plan-01-truth-recovery`)

OWNS (review this slice only):

- `graphengine-scip-adapter/**`
- `graphengine-parsing/src/infrastructure/semantic/**`
- `graphengine-parsing/src/infrastructure/runtime_fuse.rs`
- `graphengine-parsing/src/domain/authority.rs`
- `graphengine-parsing/src/domain/evidence_tier.rs`
- `graphengine-parsing/src/domain/fusion.rs`
- `graphengine-parsing/src/application/ports/semantic_index.rs`
- `graphengine-parsing/src/application/ports/runtime_evidence.rs`
- `graphengine-parsing/src/application/use_cases/parse_repo/pipeline/semantic_delta.rs`
- `graphengine-parsing/configs/go.yaml`
- `graphengine-parsing/tests/fixtures/{go,py,rust,ts}_scip/**`
- `graphengine-parsing/tests/disclosure_matrix.rs`
- `graphengine-parsing/tests/fusion_corroboration_tests.rs`
- `graphengine-parsing/tests/index_backed_*.rs`
- `graphengine-parsing/tests/rust_unification_crosscheck.rs`
- `graphengine-runtime-witness/**`
- `scripts/regen-scip-fixtures.sh`
- `.github/workflows/scip-fixture-drift.yml`
- `docs/workstreams/master-plan/evidence/10-B-results.md`
- `docs/workstreams/master-plan/02-COMPILER_TIER-TASKS.md`

G1 pins (do not re-scan for this PR):

| corpus | scan_id prefix | High Call / all Call |
|---|---|---|
| chi | `73abec07…` | 0.777 |
| zod | `4cb6e82a…` | 0.911 |

## PR 3 — agent tools + A/B + gate install

Opened: https://github.com/adenjessee/gridseak-graphengine/pull/27 (base: `p0-scip-g1`)

OWNS:

- `gridseak-cli/src/graph_queries/agent_tools.rs`
- `gridseak-cli/src/gate/**`
- `gridseak-cli/src/setup/hooks.rs`
- `gridseak-cli/src/setup/verify.rs`
- `gridseak-truth-benchmark/src/agent_ab/**`
- `gridseak-truth-benchmark/src/bin/agent_ab.rs`
- `gridseak-truth-benchmark/src/bin/false_claim_suite.rs`
- `tests/agent-ab/**`
- `tests/adversarial/**`
- `Makefile`
- `.github/workflows/release.yml` (`gridseak` required in the tarball)
- `scripts/install.sh`
- `plugin/**`
- `action/action.yml`
- `docs/workstreams/master-plan/evidence/11-AGENT_AB-*`
- `docs/workstreams/master-plan/evidence/12-*`

`make agent-ab` must still exit 0 on a machine with the G1 sqlite
artifacts. Gate fixtures (`cargo test -p gridseak-cli gate`) do not
need those artifacts — they ship a synthetic graph.

## Hold

- Observatory / 50-repo weekly scan
- Flagship video
- Homebrew / npm / marketplace submit (P3 checklist only)
- Apex LSP release gate blocking a `gridseak` tag (isolated in
  `release.yml`; Fly deploy is no longer a hard dependency of the
  binary release job)
- Previous `10-H-results.md` claim that G1 was unmet (rewritten)
