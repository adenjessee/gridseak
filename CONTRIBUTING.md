# Contributing to GridSeak

Thanks for reading this. GridSeak is a **solo, spare-time** project — I
merge good work when I can, not on a corporate schedule.

## TL;DR

1. **Open an issue first** for anything bigger than a small bug fix or doc
   tweak. Large surprise PRs are hard to review alone and may sit open a while.
2. **`cargo fmt --all` and `cargo clippy --workspace -- -D warnings`**
   before you push. CI runs both.
3. **Add a test** for the bug or behavior you're changing.
4. **Keep PRs small** when you can. A focused diff gets merged faster.
5. **Say what could break** in the PR description. "Fix bug" is not enough.

## Quick start

```sh
git clone https://github.com/adenjessee/gridseak
cd gridseak-graphengine
scripts/setup.sh dev               # optional: git hooks
cargo build --workspace
cargo test --workspace             # ~370 tests, ~50 seconds
```

Green `cargo test --workspace` and silent clippy is the bar.

## What changes are welcome

Most likely to merge:

1. **Bug fixes with a regression test.**
2. **Documentation** — typos, clarifications, examples.
3. **New Tier 0 (tree-sitter) languages** in `graphengine-parsing/`.
4. **Performance improvements with a benchmark** — numbers, not vibes.
5. **New analysis passes** in `graphengine-analysis/src/health/` — must stay
   deterministic; follow existing confidence-caveat patterns.
6. **New MCP tools** — open an issue first; the surface is intentionally small.

Unlikely to merge:

- **Telemetry** in on-machine binaries.
- **Auto-update** that phones home.
- **License gating** in this open-source tree.
- **Cloud sync** in these crates (belongs in a separate hosted product, if ever).

## How review works

- I respond when I can — **no response-time promise**.
- Green CI + a clear description + a reasonable diff size → merge when it looks right.
- I squash by default. Branch commit messages can be messy; the squash message
  uses the PR title + body.
- I may ask you to split a PR. That's about review bandwidth, not quality.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/), loosely:
`feat:`, `fix:`, `perf:`, `refactor:`, `docs:`, `test:`, `ci:`, `chore:`,
`revert:`. Optional scope: `feat(cli): …`.

## Code style

- `cargo fmt --all`
- `cargo clippy --workspace -- -D warnings` — if you need an allow, comment why.
- Rust naming conventions. Comments explain *why*, not *what*.
- Prefer typed wrappers at module boundaries where the codebase already does.

## Testing

- Unit tests: `#[cfg(test)]` next to the code.
- Integration tests: each crate's `tests/`.
- Engine output changes: run
  `cargo test -p graphengine-analysis --test determinism_integration`.
- Snapshot changes: `cargo insta review`, then commit accepted snapshots.

## Developer Certificate of Origin (DCO)

[`LICENSE-COMMITMENT.md`](LICENSE-COMMITMENT.md) explains the MIT license
commitment. Contributions use the
[Developer Certificate of Origin](https://developercertificate.org/) (not a
CLA). Add a sign-off with `git commit -s`:

```
Signed-off-by: Your Name <your.email@example.com>
```

## Conduct

No formal Code of Conduct file. Discuss the work, not the person. I may close
threads that aren't constructive. For a formal CoC, open an issue — I haven't
adopted one by default.

## Questions

- [GitHub Issues](https://github.com/adenjessee/gridseak/issues) — bugs,
  features, design.
- [GitHub Discussions](https://github.com/adenjessee/gridseak/discussions) —
  how-to and brainstorming.

No Discord or Slack. No `hello@gridseak.com` inbox.

Thank you.
