# Building GridSeak from source

## Prerequisites

- **Rust toolchain.** We track stable Rust. The minimum supported
  Rust version (MSRV) is whatever stable was 60 days ago; we update
  the `rust-toolchain.toml` lazily. `rustup` will install everything
  needed.
- **C compiler.** Required by some of our tree-sitter grammar
  vendored sources. macOS users get this from Xcode Command Line
  Tools; Linux users from `build-essential` or equivalent.
- **`pkg-config`** and **`libssl-dev` / `openssl-devel`** for the
  install script's curl-equivalent paths. The Rust binaries
  themselves use `rustls`; pkg-config is only needed for tests that
  exercise OpenSSL-using transitive deps.
- **`git`** (you're cloning this repo).

That's it. We deliberately avoid Node.js / pnpm / Python / Docker as
hard build dependencies. The previous Tauri desktop shell required
pnpm — that shell has been retired (see [`CHANGELOG.md`](CHANGELOG.md)).

## Fast path

```sh
git clone https://github.com/adenjessee/gridseak
cd gridseak-graphengine
scripts/setup.sh dev        # installs git hooks, verifies tools
cargo build --workspace     # everything in the public surface
cargo test --workspace      # ~370 tests, ~50 seconds
```

## What each step does

### `scripts/setup.sh dev`

- Configures `core.hooksPath = .githooks` so the pre-push gate runs
  locally.
- Verifies you have the required tools (`cargo`, `git`, `python3`
  for the optional fixture-fetch scripts, `jq` and `zstd` for the
  optional historical-baseline fetcher).
- Does **not** install anything globally. If you do not want the git
  hooks, skip this step.

### `cargo build --workspace`

Compiles every crate listed in `[workspace.members]`. The public
surface is the `gridseak` binary from `gridseak-cli` (CLI + MCP
server) plus its sidecar analyzers (`graphengine-parsing`,
`ge-analyze`).

### `cargo test --workspace`

Runs all workspace tests. Expected outcome: 0 failed.

If you see a test that mentions external fixtures (e.g., NPSP
baselines, canary repos), it should be skipped automatically when
the fixture is not present. If a test fails *because* a fixture is
missing, that's a bug — please file it.

## Reproducible builds

We do not currently produce byte-identical binaries across machines.
Reproducible builds are on the roadmap. When we get there, this
section will say how to verify.

In the meantime, **what we do guarantee**:

- `cargo build --release -p <crate>` produces deterministic output
  for the *contents* of the binary (apart from build timestamps and
  rustc-version strings). Two builds on the same machine, same
  toolchain, same Cargo.lock → identical binaries.
- `Cargo.lock` is checked in, so all dependency versions are pinned
  exactly.

## CI

Our CI matrix:

- `ubuntu-latest` — full test + clippy + fmt-check.
- `macos-latest` — full test + clippy + fmt-check.
- `windows-latest` — currently skipped pending egui-on-windows
  validation.

See `.github/workflows/` for the active workflows. The retired
`desktop-release.yml` workflow has been removed (see
[`CHANGELOG.md`](CHANGELOG.md)).

## Unsafe inventory

We minimize `unsafe` in this workspace. The blocks that exist:

| Location | Why it's there |
| --- | --- |
| `graphengine-parsing/src/syntax/treesitter.rs` | FFI to tree-sitter C bindings, required to call grammar functions |
| `graphengine-parsing/vendor/tree-sitter-sfapex/` | Vendored Apex grammar source (C), not our code |
| Various `rusqlite` adapter sites | rusqlite re-exposes some `unsafe` for prepared-statement reuse |

We track the inventory because OSS users reasonably want to know how
much `unsafe` is in code they run on their machine. To audit it
yourself:

```sh
rg -n 'unsafe ' --type rust --glob '!**/vendor/**'
```

(The exclusion of `vendor/**` skips the vendored Apex grammar source,
which is third-party C code, not Rust we maintain.)

## Build artifacts and what they ship

After `cargo build --release`, you get:

- `target/release/gridseak` — the CLI + MCP server.
- `target/release/ge-analyze` — the analyzer CLI (analysis only,
  scriptable subset of `gridseak`).
- `target/release/graphengine-parsing` — the parsing sidecar
  invoked by the engine pipeline.

These are statically linked Rust binaries. They depend on:

- libc (any modern glibc on Linux, or any macOS libc)
- libgcc (Linux)
- the dynamic linker

They do **not** depend on:

- A JavaScript runtime
- A Python runtime
- A network connection
- A license server

You can copy them between machines of the same OS+arch and they will
work. We do not strip them by default; if you want smaller binaries,
`strip target/release/gridseak` and similar.

## Optional artifacts

Anything bulkier than source lives outside git as a sha-pinned GitHub
release asset. The fetch scripts are documented in the README under
"Optional artifacts."

## Troubleshooting

### "linker `cc` not found"

Install your platform's C build tools.

- macOS: `xcode-select --install`
- Ubuntu / Debian: `sudo apt install build-essential`
- Fedora / RHEL: `sudo dnf install gcc make`

### "linker terminated with signal 7 [Bus error]" on Linux

Our `Cargo.toml` already sets `profile.test.debug = "line-tables-only"`
to avoid this on GitHub's ubuntu-latest runner. If you still hit it
on a memory-constrained machine, lower `CARGO_BUILD_JOBS`:

```sh
CARGO_BUILD_JOBS=1 cargo test --workspace
```

### `cargo test -p graphengine-analysis --test determinism_integration` fails

This is the byte-identical-output gate. It is failing because your
change altered scan output. If that was intentional, update the
determinism fixtures in the same PR. If unintentional, you have
found a bug — please file it.
