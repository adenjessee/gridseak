# GridSeak CLI install

## Primary: curl | sh (recommended)

Installs `gridseak`, `graphengine-parsing`, `ge-analyze`, and `configs/`
into `~/.gridseak/bin`. The installer reads the manifest attached to the
latest GitHub release and SHA256-verifies every download — no website or
CDN in the path.

```bash
# macOS (Apple Silicon + Intel) and Linux x86_64:
curl -fsSL https://raw.githubusercontent.com/adenjessee/gridseak/main/scripts/install/install.sh | bash
```

Windows (PowerShell):

```powershell
iwr https://raw.githubusercontent.com/adenjessee/gridseak/main/scripts/install/install.ps1 -useb | iex
```

Once `gridseak.com` mirrors the release assets, the shorter
`curl -fsSL https://gridseak.com/install.sh | sh` becomes an equivalent
alias. Override the source anytime with `GRIDSEAK_MANIFEST_URL`.

### Supported platforms (v0.1.0)

| Platform | Command | Status |
|----------|---------|--------|
| macOS Apple Silicon | `install.sh` | shipped |
| macOS Intel | `install.sh` | shipped |
| Linux x86_64 | `install.sh` | shipped |
| Windows x86_64 | `install.ps1` | shipped |
| Linux arm64 | `install.sh` | **not yet** — explicit failure until tarballs ship |

---

## From this repository (local proof)

```bash
./scripts/install/build-cli-release.sh
./scripts/install/install.sh
```

Windows:

```powershell
./scripts/install/install.ps1
```

---

## From source (full workspace)

```bash
git clone https://github.com/adenjessee/gridseak
cd gridseak
cargo build --release -p gridseak-cli
# Sidecars: target/release/graphengine-parsing, ge-analyze; configs/ from repo root
```

### cargo install (CLI binary only)

```bash
cargo install --path gridseak-cli --locked
```

**`--locked` is required, not optional.** Without it, cargo re-resolves
dependencies from scratch and pulls a second, newer `tree-sitter` for the
`tree-sitter-rust` grammar (which declares an unbounded `tree-sitter = ">= 0.20"`),
producing two incompatible `tree_sitter::Language` types and a hard build
failure (E0308). The committed `Cargo.lock` pins a single `tree-sitter 0.20.10`
across the workspace, and `--locked` honors it. (A future grammar-version bump
will remove this constraint; tracked in the post-γ backlog.)

This installs **only** the `gridseak` binary — not the sidecar analyzers or
`configs/`. A working `gridseak scan` needs the full bundle from `curl | sh`
or a workspace release build.

**MCP from a dev build:** Cursor’s MCP server must see the same binary as your shell.
After `cargo install --path gridseak-cli`, run `gridseak setup` and restart the IDE.

---

## After install

```bash
gridseak setup          # MCP + Cursor rules
gridseak setup --verify # Confirm MCP + routing rule
gridseak scan .
```

See also: [CURSOR MCP migration](CURSOR_MCP_MIGRATION.md), [LIMITATIONS.md](../../LIMITATIONS.md).
