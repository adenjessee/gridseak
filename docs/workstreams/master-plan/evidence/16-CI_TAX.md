# 16 — P1: why agent PRs go red before merge

Date: 2026-10-11. Data: the 41 Copilot-agent PRs (Microsoft/Azure orgs) from
`14-AGENT_PR_STUDY.md` §C3 that had a failing check on a non-final commit.
Scripts: `tests/agent-pr-study/p1_fetch.py`, `p1_classify.py`.

## What failed

244 failing check runs on non-final commits (220 GitHub Actions, 24 Azure
Pipelines).

| Check category (by name) | Runs | PRs touched (of 41) |
|---|---:|---:|
| Build + test jobs | 149 | 28 |
| Lint / format / pre-commit | 34 | 12 |
| Merge-gate aggregator (mirrors others) | 27 | 10 |
| Docs / changelog | 14 | 3 |
| Build only | 8 | 2 |
| Deploy / E2E | 6 | 3 |
| Security scan | 3 | 1 |
| Other | 3 | 3 |

Logs survive for 42 runs from 12 PRs (GitHub deletes Actions logs after 90
days). Read by hand:

| Root cause in the log | Examples |
|---|---|
| Behavioral test failures | 17 failed torch-op tests (onnxscript), E2E assertion (AzureTRE), hook timing tests (ShadowFrog) |
| Type / lint | pyright `str` vs `int` argument, mypy union-attr, Super-linter "file not executable" |
| Policy | "Code update without version change" |
| Infrastructure | runner shutdown signal, container permission error |
| **Structural (missing import, undefined name, missing symbol, broken caller)** | **none (0 of 12 PRs)** |

Time cost: median **35 minutes** from a red commit to the agent's next
commit (p75 418 min, n = 117 red commits); median 1 red non-final commit per
PR, max 13.

## What this means

- A pre-CI caller/deletion check would not have prevented any of these red
  rounds. The gate and `pr-check` do not address the CI tax.
- The largest share is **behavioral test failures discovered only in CI**.
  The same call graph can answer "which tests exercise the functions this
  change touched?" (test impact analysis) so an agent runs those tests
  locally before pushing. Untested hypothesis; it is folded into P2.

## Limits

12 PRs with logs; one org family; logs for older PRs are gone. Check names
were classified by keyword.
