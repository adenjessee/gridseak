# Contributing to GridSeak

Thank you for taking the time to read this. The bullets below are the
honest version of what to do — they exist because they speed your PR up,
not to gatekeep.

## TL;DR

1. **Open an issue first** if your change is larger than a bug fix or a
   tiny feature. We close drive-by PRs that touch lots of files without
   a tracking issue, politely, because reviewing them without prior
   alignment is unfair to everyone.
2. **Run `cargo fmt --all` and `cargo clippy --workspace -- -D warnings`
   before pushing.** CI runs both; failing CI on a formatting issue
   wastes everyone's afternoon.
3. **Add a test for the bug you're fixing or the behavior you're
   adding.** "I tested it locally" is not the same as a regression
   guard.
4. **Keep PRs small.** A 200-line PR gets reviewed in a day. A
   2000-line PR gets reviewed in a week and merged with regret. If you
   have a big change, split it.
5. **Be honest in commit messages and PR descriptions.** What you
   changed, why, and what could break. Avoid "fix bug" — name the bug.

## Quick start

```sh
git clone https://github.com/adenjessee/gridseak
cd gridseak-graphengine
scripts/setup.sh dev               # installs git hooks, verifies tools
cargo build --workspace            # compiles the full public surface
cargo test --workspace             # ~370 tests, ~50 seconds
```

If `cargo test` is green and `cargo clippy --workspace -- -D warnings`
is silent, you're set up correctly.

## What changes are welcome

In order of how likely we are to merge it:

1. **Bug fixes with a regression test.** Always welcome.
2. **Documentation improvements** — typo fixes, broken-link fixes,
   clarifications, new examples. Always welcome.
3. **New languages for Tier 0 (tree-sitter) edges** — the parsing
   pipeline lives in `graphengine-parsing/`; adding a language is
   typically a self-contained PR.
4. **Performance improvements with a benchmark** — show the
   before/after. Without numbers we cannot tell if it's faster or
   just different.
5. **New analysis passes** — `graphengine-analysis/src/health/`.
   These need to be deterministic, well-tested, and have a
   confidence caveat (see existing modules for the pattern).
6. **New MCP tools.** Open an issue first; the tool surface is
   intentionally small (fourteen tools in v0.1.0). The agent-facing
   surface only stays planner-friendly if we keep it small, so we
   want to understand the use case before growing it.

## What changes we are unlikely to merge

- **Telemetry of any kind** in the on-machine binaries.
  Non-negotiable; see the LIMITATIONS doc for why.
- **Auto-update logic** that phones home. Same reason.
- **License gating** of features in the open-source crates. The
  free tier is the whole tier on a single machine; paid features
  (if and when they ship) will live in separate hosted services,
  not in this repo.
- **"Cloud sync" features** added to the open-source crates. They
  belong in a SaaS layer, not here.

## How we review

- We aim for a first response on every PR within 72 hours, weekdays.
- A "Looks Good To Me" from a maintainer + green CI = merge.
- We squash by default. Your commit messages on the branch can be
  whatever; the merge commit message will be the PR title + body.
- We may ask you to split a PR. We are not asking because the work
  is bad; we are asking because two small reviews are better than
  one long one.

## Commit message conventions

We use [Conventional Commits](https://www.conventionalcommits.org/),
loosely. The prefix tells the reader what *kind* of change it is at a
glance:

- `feat:` new user-facing behavior
- `fix:` bug fix
- `perf:` measurable performance improvement
- `refactor:` no behavior change, code quality
- `docs:` documentation only
- `test:` tests only
- `ci:` CI / build pipeline changes
- `chore:` housekeeping (deps, formatting, file moves)
- `revert:` revert a prior commit

Scope is optional but helpful: `feat(view): add hierarchy perspective`.

## Code style

- **Formatting**: `cargo fmt --all`. We do not negotiate the formatter.
- **Lints**: `cargo clippy --workspace -- -D warnings`. If you disagree
  with a clippy lint, `#[allow(clippy::lint_name)]` it locally with a
  comment explaining why, rather than fighting it in PR review.
- **Naming**: Rust convention. `snake_case` for fns and modules,
  `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- **Comments**: explain *why*, not *what*. The code already says what
  it does. The comment exists for the constraint or trade-off that
  isn't obvious.
- **Public API**: prefer typed wrappers (`StableId(String)`) over raw
  primitives at module boundaries. The engine has many of these for
  a reason.

## Testing

- **Unit tests** live next to the code they test, in
  `#[cfg(test)] mod tests` blocks.
- **Integration tests** live in each crate's `tests/` directory.
- **Determinism tests** are special: if you change anything about the
  output of `ge-analyze`, run
  `cargo test -p graphengine-analysis --test determinism_integration`
  and ensure the assertion still holds. If you intentionally changed
  the output format, update the determinism fixtures in the same PR.
- **Snapshot tests** use `insta`. After a deliberate output change:
  `cargo insta review` to accept new snapshots, then commit them.

## Developer Certificate of Origin (DCO)

We use the [Developer Certificate of Origin](https://developercertificate.org/)
(DCO 1.1), not a Contributor License Agreement. A CLA is often read as
"the project plans to relicense later"; we committed to MIT OR
Apache-2.0 forever in [`LICENSE-COMMITMENT.md`](LICENSE-COMMITMENT.md).

Every commit must include a sign-off line:

```
Signed-off-by: Your Name <your.email@example.com>
```

Use your real name and an email you are comfortable being associated
with the change. `git commit -s` adds the line automatically.

By signing off, you certify the DCO terms (see the link above): you
wrote the patch or have the right to submit it under our license, and
you understand the contribution is licensed MIT OR Apache-2.0 like the
rest of the project.

## Code of Conduct

We have no published Code of Conduct file. We do have an expectation:
discuss the work, not the person. Maintainers reserve the right to
close issues and ask repeat-offender users to take a break. There is no
formal appeals process — this is a small project run by humans.

If the lack of a formal CoC is a blocker for your contribution, open
an issue saying so and we'll discuss adopting the
[Contributor Covenant](https://www.contributor-covenant.org/). We have
not adopted it preemptively because we want the choice to be a
deliberate one rather than a default.

## Where to ask questions

- **GitHub Issues** for bug reports, feature requests, design
  discussions. Use the issue templates.
- **GitHub Discussions** for "how do I…" questions, broader brainstorms,
  show-and-tell.
- We do not currently have a Discord / Slack. We may, eventually; we
  do not have one now because we want the discussion to be
  searchable and indexed by Google, not lost in a chat scrollback.

Thank you again.
