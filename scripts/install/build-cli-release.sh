#!/usr/bin/env bash
# scripts/install/build-cli-release.sh — pack the standalone GridSeak CLI.
#
# Pilot scope (Stage 10 "local proof"): produce target/cli-release/<version>/
# with one tar.gz per supported host triple, each containing:
#
#   gridseak              # the CLI itself
#   graphengine-parsing   # parser engine
#   ge-analyze            # analyzer engine
#   configs/              # parser language configs (same set Tauri bundles)
#   LICENSE-MIT
#   LICENSE-APACHE
#   README.md             # short, install-script-friendly
#
# Also emits:
#   <archive>.sha256       # SHA256 sidecar
#   cli-manifest.json      # version + per-target {url, sha256, size}
#
# Build only for the host triple by default. Pass triples explicitly to
# cross-compile (Stage 10 explicitly defers cross-target publish to the
# website plan).
#
# Usage:
#   scripts/install/build-cli-release.sh                       # host triple only
#   scripts/install/build-cli-release.sh aarch64-apple-darwin x86_64-apple-darwin
#
# Outputs:
#   target/cli-release/<version>/gridseak-<version>-<triple>.tar.gz
#   target/cli-release/<version>/gridseak-<version>-<triple>.tar.gz.sha256
#   target/cli-release/<version>/cli-manifest.json
set -euo pipefail

WORKSPACE="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$WORKSPACE"

# Version comes from the CLI's Cargo.toml so manifest + binary agree.
# `version.workspace = true` redirects to the workspace package version.
read_workspace_version() {
  # Read lines between `[workspace.package]` and the next `[` heading, then
  # pull the first `version = "x.y.z"`. Bash-only; no toml parser required.
  awk '
    /^\[workspace\.package\]/ { in_block=1; next }
    in_block && /^\[/         { exit }
    in_block && /^version[[:space:]]*=/ {
      match($0, /"[^"]+"/)
      v=substr($0, RSTART+1, RLENGTH-2)
      print v
      exit
    }
  ' Cargo.toml
}

if grep -q '^version\.workspace' gridseak-cli/Cargo.toml; then
  VERSION="$(read_workspace_version)"
else
  VERSION="$(awk -F\" '/^version[[:space:]]*=/ {print $2; exit}' gridseak-cli/Cargo.toml)"
fi
[[ -n "$VERSION" ]] || { echo "[build-cli-release] could not determine version" >&2; exit 1; }

# Host triple resolution mirrors rustup default-target.
if [[ $# -ge 1 ]]; then
  TARGETS=("$@")
else
  HOST_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"
  [[ -n "$HOST_TRIPLE" ]] || { echo "[build-cli-release] could not detect host triple" >&2; exit 1; }
  TARGETS=("$HOST_TRIPLE")
fi

OUT_DIR="$WORKSPACE/target/cli-release/$VERSION"
mkdir -p "$OUT_DIR"

CONFIGS_DIR="$WORKSPACE/graphengine-parsing/configs"
[[ -d "$CONFIGS_DIR" ]] || { echo "[build-cli-release] expected $CONFIGS_DIR to exist" >&2; exit 1; }

MANIFEST_TMP="$OUT_DIR/cli-manifest.json.tmp"
{
  printf '{\n'
  printf '  "version": "%s",\n' "$VERSION"
  printf '  "artifacts": ['
} > "$MANIFEST_TMP"
FIRST_ENTRY=1

# Per-target build + archive loop. Each iteration is independent so the
# script can be re-run with a single triple to refresh just that artifact.
for TARGET in "${TARGETS[@]}"; do
  echo "[build-cli-release] target=$TARGET"

  ext=""
  case "$TARGET" in
    *windows*) ext=".exe" ;;
  esac

  case "$TARGET" in
    aarch64-apple-darwin|x86_64-apple-darwin|aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu)
      cargo build --release --target "$TARGET" \
        -p graphengine-parsing -p graphengine-analysis -p gridseak-cli \
        --bin graphengine-parsing --bin ge-analyze --bin gridseak
      ;;
    x86_64-pc-windows-msvc|aarch64-pc-windows-msvc)
      # On a native Windows host (CI runs each target on a matching-OS
      # runner) plain `cargo build` is correct and needs no extra tooling.
      # `cargo xwin` is only for cross-building Windows from macOS/Linux
      # (local-dev convenience; the desktop release vetted this path).
      HOST="$(rustc -vV | awk '/^host:/ {print $2}')"
      if [[ "$HOST" == *windows* ]]; then
        cargo build --release --target "$TARGET" \
          -p graphengine-parsing -p graphengine-analysis -p gridseak-cli \
          --bin graphengine-parsing --bin ge-analyze --bin gridseak
      else
        cargo xwin build --release --target "$TARGET" \
          -p graphengine-parsing -p graphengine-analysis -p gridseak-cli \
          --bin graphengine-parsing --bin ge-analyze --bin gridseak
      fi
      ;;
    *)
      echo "[build-cli-release] unsupported target: $TARGET" >&2
      exit 1
      ;;
  esac

  STAGE_DIR="$(mktemp -d)"
  trap 'rm -rf "$STAGE_DIR"' EXIT INT TERM

  for bin in gridseak graphengine-parsing ge-analyze; do
    src="$WORKSPACE/target/$TARGET/release/${bin}${ext}"
    [[ -f "$src" ]] || { echo "[build-cli-release] missing $src" >&2; exit 1; }
    # `strip` is target-aware; on macOS it operates on Mach-O and is a no-op for
    # Windows binaries which arrive already stripped via cargo-xwin's runner.
    if [[ "$ext" != ".exe" ]]; then
      strip "$src" 2>/dev/null || true
    fi
    cp "$src" "$STAGE_DIR/${bin}${ext}"
  done

  # Bundle the configs directory unchanged so the CLI can resolve them via
  # the same `GRAPHENGINE_CONFIGS_DIR` envvar (or the ancestor probe) as
  # the workspace dev path.
  cp -R "$CONFIGS_DIR" "$STAGE_DIR/configs"

  # Cursor wiring ships via `gridseak setup` (writes mcp.json + the
  # gridseak Cursor rule that teaches the agent when to call each tool),
  # so the tarball does not bundle a separate agent skill.

  for f in LICENSE-MIT LICENSE-APACHE; do
    if [[ -f "$WORKSPACE/$f" ]]; then
      cp "$WORKSPACE/$f" "$STAGE_DIR/$f"
    fi
  done

  # User-facing README inside the archive. Leads with copy-pasteable
  # commands instead of a directory listing — the directory layout
  # itself is documented at the bottom for the curious. The shell
  # examples assume the install script's symlink layout
  # (`~/.gridseak/bin/gridseak`) so they work without modification on
  # any host that ran install.sh / install.ps1.
  cat > "$STAGE_DIR/README.md" <<EOF
# GridSeak CLI ${VERSION} — ${TARGET}

Local, deterministic structural-health analysis for any codebase.
Source code never leaves your machine. No signup required.

## Quick start

\`\`\`bash
gridseak doctor              # verify the install
gridseak scan .              # first scan of the current folder
gridseak setup               # wire MCP + Cursor rule into your IDE(s)
\`\`\`

The first \`gridseak scan .\` prints a hero structural-health report
(score, top risks, metric table, next commands) directly to your
terminal. No upload, no account.

## Hook the scan into your AI agent (Cursor + MCP)

\`gridseak setup\` writes \`~/.cursor/mcp.json\` (so the IDE launches the
MCP server on demand) and installs the GridSeak Cursor rule that teaches
the agent when to call each of the deterministic tools. Run it once, then
open your IDE and ask:

> "What's risky to refactor in this repo?"

The agent will invoke the MCP tools, get back the same numbers
\`gridseak scan .\` shows, and synthesise an honest answer that cites the
deterministic evidence and flags confidence caveats.

## Useful drilldowns

\`\`\`bash
gridseak recommendations --limit 10
gridseak metrics --format markdown
gridseak graph top-fan-in --limit 10
gridseak graph blast-radius "<symbol>" --depth 2
gridseak graph cycles --limit 5
gridseak context --for-llm                 # compact agent-ready bundle
gridseak compare --previous                # delta vs your last scan
\`\`\`

Run \`gridseak --help\` for the full surface.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| macOS: "developer cannot be verified" on first launch | \`xattr -dr com.apple.quarantine ~/.gridseak/bin ~/.gridseak/share\` (one-time) |
| Windows: SmartScreen blocks first launch | Click "More info" -> "Run anyway" |
| Scan fails fast with \`BinaryVersionMismatch\` | Sidecar is stale. Reinstall via \`curl -fsSL https://raw.githubusercontent.com/adenjessee/gridseak/main/scripts/install/install.sh \| bash\`. |
| \`gridseak\` not found in a new shell | Add \`~/.gridseak/bin\` to \`PATH\`; \`gridseak doctor\` reports if it's missing. |
| MCP server not picked up by Cursor | \`gridseak setup\` then restart Cursor. \`gridseak doctor\` reports MCP status. |

## Contents

| Path | What it is |
| --- | --- |
| \`gridseak${ext}\` | CLI entry point. Runs the scan, hosts the MCP server, owns local store. |
| \`graphengine-parsing${ext}\` | Parser engine sidecar. Executed by \`gridseak scan\`. |
| \`ge-analyze${ext}\` | Analyzer engine sidecar. Computes the health report from the parsed graph. |
| \`configs/\` | Tree-sitter language descriptors. Loaded by the parser; do not hand-edit. |
| \`LICENSE-MIT\`, \`LICENSE-APACHE\` | Dual MIT/Apache-2.0 licence. |

## Local-only feedback

The CLI carries a \`gridseak feedback "<text>"\` command that writes to
a local SQLite table — nothing leaves your machine unless you explicitly
export it. Tell us what would unlock you and we'll see it the next
time you choose to share.

Full walkthrough: <https://gridseak.com/cli>
EOF

  ARCHIVE_NAME="gridseak-${VERSION}-${TARGET}.tar.gz"
  ARCHIVE="$OUT_DIR/$ARCHIVE_NAME"
  echo "[build-cli-release] packing $ARCHIVE"
  tar -C "$STAGE_DIR" -czf "$ARCHIVE" .
  rm -rf "$STAGE_DIR"
  trap - EXIT INT TERM

  if command -v shasum >/dev/null 2>&1; then
    HASH="$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"
  else
    HASH="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
  fi
  echo "$HASH  $ARCHIVE_NAME" > "$ARCHIVE.sha256"
  SIZE="$(wc -c < "$ARCHIVE" | tr -d ' ')"

  if [[ "$FIRST_ENTRY" == 1 ]]; then FIRST_ENTRY=0; else printf ',' >> "$MANIFEST_TMP"; fi
  printf '\n    {"target": "%s", "url": "%s", "sha256": "%s", "size": %s}' \
    "$TARGET" "$ARCHIVE_NAME" "$HASH" "$SIZE" >> "$MANIFEST_TMP"
done

printf '\n  ]\n}\n' >> "$MANIFEST_TMP"
mv "$MANIFEST_TMP" "$OUT_DIR/cli-manifest.json"
echo "[build-cli-release] wrote $OUT_DIR/cli-manifest.json"
ls "$OUT_DIR"
