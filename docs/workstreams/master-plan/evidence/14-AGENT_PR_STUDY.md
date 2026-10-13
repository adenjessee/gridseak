# 14 — Do merged AI-agent PRs break callers? (and can pr-check tell?)

Date: 2026-10-11. Scripts: `tests/agent-pr-study/` (see its README).
Sample: merged PRs authored by GitHub's Copilot coding agent
(`app/copilot-swe-agent`), Python repos.

## A. Dangling references in merged agent PRs

| Sample | PRs collected | PRs that delete/rename a `def` | Functions removed | Text search: name still mentioned at merge | Type checker (pyright): real dangling reference |
|---|---:|---:|---:|---:|---:|
| `microsoft` + `Azure` orgs | 1,226 | 61 | 1,308 | 31 PRs | **0** |
| Any repo, Sep 2026, ≤2 PRs/repo, random 1,500 | 1,500 (1,212 repos) | 82 | 585 | 33 PRs | **0** (1 hit, hand-checked: unrelated library method) |
| **Total** | 2,726 | **143** | **1,893** | **64 PRs (45%)** | **0** |

Positive control: renaming a used function in one of these checkouts made
pyright report both call sites (`"create_filtered_tool_list" is unknown import
symbol`). The pipeline detects breaks; the merged PRs simply had none.

**Finding:** in-repo "agent deleted something still in use" did not reach
`main` in this sample. CI, review and the agents' own loops catch it first.
A public index built on "agents ship broken callers" would be measuring
nothing. Text search (what grep-based reviewers and agents use) would have
raised a false alarm on 45% of these PRs.

Not measured: breakage caught *before* merge (inside the agent loop or CI),
breaks in downstream repos that import a removed public API, and languages
other than Python.

## B. `gridseak pr-check` on the same real PRs

`pr-check` compares a base scan and a head scan and flags a removed function
only when a caller the compiler proved at base still names it at head and
the head compiler index resolves that use to nothing (or to the removed
symbol). See `gridseak-cli/src/pr_check.rs`.

| Set | PRs | Functions removed | Removed with compiler-proven callers | Callers rewritten in the same PR | safe | review | unverified | broken |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Microsoft/Azure deletion PRs (≤ 2,500 .py files) | 50 | 2,944 | 205 | 375 | 45 | 2 | 3 | **0** |
| Any-repo Copilot-agent PRs (matched sample) | 30 | 33 | 5 | 4 | 29 | 1 | 0 | **0** |
| Human PRs, same repos and period | 33 | 232 | 8 | 14 | 32 | 1 | 0 | **0** |
| **Total** | **113** | **3,209** | **218** | **393** | 106 | 4 | 3 | **0** |

- All four `review` verdicts are methods whose name still exists on another
  class (`OnnxDAG.update`, `OBOCredentialProvider.close`, …) reached through
  an attribute call on an untyped Python receiver — advisory, and correct to
  not call `broken`.
- `unverified`: `microsoft/apm` (3 PRs), where scip-python produced no index.
- 11 PRs in repos with > 2,500 Python files (azure-sdk-for-python,
  autorest.python, pyright) were not scanned.
- The per-edit `gridseak gate` would have denied every removal that had
  compiler-proven callers, even though those callers were rewritten in the
  same PR; pr-check recognizes them as updated.

## C. Seeded breaks (recall)

Delete one function that has compiler-proven callers, keep the callers,
rescan, run pr-check.

| Corpus | Seeded | Flagged `broken` | Not flagged |
|---|---:|---:|---|
| chi (Go) | 30 | 29 | 1: method removal that breaks interface satisfaction |
| zod (TypeScript) | 10 | 8 | 2: removed abstract-method override; removed overload implementation (signatures remain) |
| 10 real Python repos from §B | 10 | 9 (8 `broken`, 1 `review`) | 1: removed overload implementation (`@overload` signatures remain) |

Every seeded break whose callers lost their target was flagged: **46/46**.
The four misses are inheritance/overload contracts — the compiler reports
them at the declaration, not at any caller — and need a separate check.

## C2. Does agent work change architecture more than human work?

Module-level shape delta (new compiler-proven call dependencies between two
directories that both existed at base), same measurement on both sides:

| Set | PRs | PRs adding a new cross-module dependency | PRs with 0 human reviews |
|---|---:|---:|---:|
| Microsoft/Azure agent deletion PRs | 50 | 9 (18%) | 7 |
| Any-repo agent PRs (paired) | 30 | 1 (3%) | 19 |
| Human PRs, same repos (paired) | 30 | 2 (7%) | 27 |

No evidence that agent PRs reshape architecture more than human PRs (small,
mostly personal repos; 29/30 and 30/33 scans had a compiler index).

## C3. Breakage inside the agent loop (before merge)

150 random Microsoft/Azure Copilot-agent PRs, check-runs per commit:
144 had CI; **41 (28%) had a failing check on a non-final commit** and were
then merged; 13 (9%) merged while the last commit still had a failing check
(required-ness and flakiness not checked). Failing checks are mixed: lint,
unit tests, build (`dotnet-build-and-test-check`), changelog, E2E.

## D. Defects found and fixed while running this study

1. pr-check identity: node FQNs omit the class/receiver, so `URLParam` and
   `Context.URLParam`, or every `_parse` override in zod, shared one name.
   Fixed with the enclosing class range plus the declaration line for names
   that are still duplicated.
2. Module top-level callers (`__file_scope__`) were ignored.
3. Docstrings, strings and template literals (`` `${fn()}` ``) were treated
   as code (or the reverse).
4. A signature edit was treated as a removal, then the still-defined symbol
   as dangling.
5. Python indexing silently fell back to heuristics when the repo had no
   root `pyproject.toml` / `setup.py` / `requirements.txt` (12 of 50 PRs).
   `provision_python` now indexes any tree that contains `.py` files, and
   pr-check reports `unverified` instead of `safe` when no compiler tier ran.
