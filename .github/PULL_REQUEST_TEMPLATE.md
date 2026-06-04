<!--
Thank you for the PR. Please fill out the sections below. PRs with no
description are hard to review on a solo-maintained project and may sit
open until there is enough context to merge safely.
-->

## What this PR does

<!-- One sentence. The full PR title goes in the title field. -->

## Why

<!-- The problem this solves or the feature this adds. Link the
     tracking issue with `Fixes #N` or `Refs #N`. -->

## How it works

<!-- A short architectural description if the change is non-trivial.
     Skip if the diff speaks for itself. -->

## Tests

<!-- Which tests cover this change? Did you add new ones? If the
     change is a refactor (no behavior change), say so. -->

- [ ] Existing tests still pass (`cargo test --workspace`)
- [ ] New tests added for the new behavior / bug fix
- [ ] Not applicable (docs / chore / refactor with no behavior change)

## Checklist

- [ ] `cargo fmt --all` is clean
- [ ] `cargo clippy --workspace -- -D warnings` is clean
- [ ] `cargo test --workspace` is green locally
- [ ] If this changes Engine output, the
      `determinism_integration` test still passes (or has been
      updated in this same PR)
- [ ] If this changes the MCP tool surface, `.cursor/rules/gridseak.mdc`
      and `README.md` (tool table + count) have been updated
- [ ] If this affects a documented limitation, `LIMITATIONS.md` has
      been updated
- [ ] If this is a breaking change, it's called out in the PR title
      with `BREAKING:` and added to `CHANGELOG.md` under
      `## [Unreleased]`

## What could break

<!-- An honest "what could go wrong" list. Even one line is fine. -->
