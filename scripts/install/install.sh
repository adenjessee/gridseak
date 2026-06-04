#!/usr/bin/env bash
# scripts/install/install.sh — GridSeak local-proof installer.
#
# Reads a manifest at $GRIDSEAK_MANIFEST_URL (default: localhost staging),
# picks the artifact for this host's triple, verifies SHA256, and
# extracts it into $GRIDSEAK_HOME/bin (default: $HOME/.gridseak/bin).
#
# Design constraints from the spec:
#   - readable, < 200 lines
#   - no blanket `set -e`; every step has an explicit failure path
#   - prints every action it intends to take before doing it
#   - never requests admin
#   - emits PATH instructions for bash / zsh / fish
#
# Local proof flow:
#   1. scripts/install/build-cli-release.sh
#   2. (cd target/cli-release/<version> && python3 -m http.server 8765)
#   3. GRIDSEAK_MANIFEST_URL=http://localhost:8765/cli-manifest.json \
#        bash scripts/install/install.sh
set -u

PROG="gridseak-install"
log()   { printf '[%s] %s\n' "$PROG" "$*"; }
warn()  { printf '[%s] WARN: %s\n' "$PROG" "$*" >&2; }
fail()  { printf '[%s] ERROR: %s\n' "$PROG" "$*" >&2; exit 1; }
plan()  { printf '[%s] plan: %s\n' "$PROG" "$*"; }

# Production default: the manifest attached to the latest GitHub CLI
# release. GitHub serves release assets at a stable redirect
# (`releases/latest/download/<asset>`), and the manifest's per-target
# `url` fields are relative, so they resolve against this same
# directory — making the whole install path GitHub-native with no
# website or CDN dependency. The vanity host (gridseak.com) is an
# optional override once the site mirrors these assets. For local-proof
# flows, override with
# `GRIDSEAK_MANIFEST_URL=http://localhost:8765/cli-manifest.json`.
MANIFEST_URL="${GRIDSEAK_MANIFEST_URL:-https://github.com/adenjessee/gridseak/releases/latest/download/cli-manifest.json}"
HOME_DIR="${GRIDSEAK_HOME:-$HOME/.gridseak}"
BIN_DIR="$HOME_DIR/bin"
SHARE_DIR="$HOME_DIR/share/$(date -u +%Y%m%dT%H%M%SZ)"

# Note: this installer is the standalone CLI flow. The repo root also
# has a `.gridseak/` folder used by `scripts/setup.sh canary-repos`
# (Stage 11 fixture clones) which is unrelated and lives under the
# repo path, not your $HOME. They never overlap on disk.

if ! command -v curl >/dev/null 2>&1; then fail "missing curl"; fi
if ! command -v tar  >/dev/null 2>&1; then fail "missing tar"; fi

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)   TRIPLE="aarch64-apple-darwin" ;;
  Darwin-x86_64)  TRIPLE="x86_64-apple-darwin"  ;;
  Linux-x86_64)   TRIPLE="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64)  TRIPLE="aarch64-unknown-linux-gnu" ;;
  *) fail "unsupported host: $(uname -s)-$(uname -m). Run install.ps1 on Windows." ;;
esac

log "manifest:   $MANIFEST_URL"
log "host:       $TRIPLE"
log "install to: $BIN_DIR"
log "share to:   $SHARE_DIR"

plan "1. download $MANIFEST_URL"
plan "2. locate artifact for $TRIPLE in the manifest"
plan "3. download and SHA256-verify the artifact"
plan "4. extract into $SHARE_DIR (versioned)"
plan "5. link into $BIN_DIR (gridseak, graphengine-parsing, ge-analyze)"
plan "6. print PATH instructions"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT INT TERM

log "[1/6] download manifest"
MANIFEST="$WORK_DIR/cli-manifest.json"
if ! curl -fsSL "$MANIFEST_URL" -o "$MANIFEST"; then
  fail "could not download manifest from $MANIFEST_URL"
fi

log "[2/6] locate artifact for $TRIPLE"
if command -v python3 >/dev/null 2>&1; then
  ARTIFACT_LINE="$(python3 - "$MANIFEST" "$TRIPLE" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
triple = sys.argv[2]
for a in data.get("artifacts", []):
    if a.get("target") == triple:
        print(a.get("url",""), a.get("sha256",""), a.get("size","-"), data.get("version","unknown"))
        sys.exit(0)
sys.exit(2)
PY
)" || fail "no artifact for $TRIPLE in manifest"
else
  ARTIFACT_LINE="$(grep -A4 "\"$TRIPLE\"" "$MANIFEST" | grep -E '"url"|"sha256"|"size"' | tr -d '",' | tr '\n' ' ')"
  [[ -n "$ARTIFACT_LINE" ]] || fail "no artifact for $TRIPLE in manifest (and python3 missing for json parse)"
fi
ART_URL_REL="$(echo "$ARTIFACT_LINE" | awk '{print $1}')"
ART_SHA="$(echo "$ARTIFACT_LINE" | awk '{print $2}')"
ART_VERSION="$(echo "$ARTIFACT_LINE" | awk '{print $4}')"
[[ -n "$ART_URL_REL" && -n "$ART_SHA" ]] || fail "manifest entry missing url/sha256 for $TRIPLE"

# Resolve relative URL against the manifest URL.
case "$ART_URL_REL" in
  http*://*) ART_URL="$ART_URL_REL" ;;
  *)         ART_URL="${MANIFEST_URL%/*}/$ART_URL_REL" ;;
esac
log "         version=$ART_VERSION url=$ART_URL"

log "[3/6] download artifact"
ART="$WORK_DIR/$(basename "$ART_URL_REL")"
if ! curl -fsSL "$ART_URL" -o "$ART"; then
  fail "could not download $ART_URL"
fi
if command -v shasum >/dev/null 2>&1; then
  GOT="$(shasum -a 256 "$ART" | awk '{print $1}')"
else
  GOT="$(sha256sum "$ART" | awk '{print $1}')"
fi
[[ "$GOT" == "$ART_SHA" ]] || fail "SHA256 mismatch (expected $ART_SHA got $GOT)"
log "         sha256 verified"

log "[4/6] extract into $SHARE_DIR"
mkdir -p "$SHARE_DIR" || fail "could not create $SHARE_DIR"
if ! tar -C "$SHARE_DIR" -xzf "$ART"; then
  fail "tar extract failed"
fi
[[ -x "$SHARE_DIR/gridseak" || -x "$SHARE_DIR/gridseak.exe" ]] || fail "extracted bundle missing gridseak binary"

log "[5/6] link into $BIN_DIR"
mkdir -p "$BIN_DIR" || fail "could not create $BIN_DIR"
for bin in gridseak graphengine-parsing ge-analyze; do
  src="$SHARE_DIR/$bin"
  [[ -f "$src" ]] || src="$SHARE_DIR/${bin}.exe"
  [[ -f "$src" ]] || { warn "skipping $bin (not in archive)"; continue; }
  dst="$BIN_DIR/${bin}"
  [[ "$src" == *.exe ]] && dst="${dst}.exe"
  rm -f "$dst"
  if ! ln -sf "$src" "$dst"; then
    cp "$src" "$dst" || fail "could not place $dst"
  fi
  chmod +x "$dst" || true
done
# Also expose the configs dir adjacent to bin for the CLI's ancestor probe.
if [[ -d "$SHARE_DIR/configs" ]]; then
  ln -sfn "$SHARE_DIR/configs" "$HOME_DIR/configs"
fi
log "         installed:"
ls -l "$BIN_DIR"

log "[6/6] PATH guidance"
case ":$PATH:" in
  *":$BIN_DIR:"*) log "         $BIN_DIR is already on PATH." ;;
  *)
    cat <<EOF

Add this to your shell startup file so \`gridseak\` works in new sessions:

  bash / zsh:   echo 'export PATH="$BIN_DIR:\$PATH"' >> ~/.zshrc      # or ~/.bashrc
  fish:         fish_add_path "$BIN_DIR"

Smoke test now (uses absolute path so PATH change isn't required):
  $BIN_DIR/gridseak --version
  $BIN_DIR/gridseak scan .

EOF
    ;;
esac

log "done. version=$ART_VERSION root=$HOME_DIR"

# macOS Gatekeeper hint. The binaries inside the tarball are not yet Apple
# Developer ID-notarised (see docs/05-deployment/SIGNING_PROCUREMENT.md), so
# the first `gridseak` invocation may print "cannot be opened because the
# developer cannot be verified". We surface the one-line workaround here so
# users aren't stuck. Linux + Windows are unaffected; install.ps1 emits the
# equivalent SmartScreen note.
case "$(uname -s)" in
  Darwin)
    cat <<'EOF'

macOS note
----------
If your first `gridseak` command is blocked by Gatekeeper
("...cannot be opened because the developer cannot be verified"),
clear the quarantine attribute once and the binary will run from then on:

  xattr -dr com.apple.quarantine ~/.gridseak/bin ~/.gridseak/share

This is a one-time step until we publish a notarised build.
EOF
    ;;
esac

# Cursor / Claude Code / Codex / Windsurf MCP nudge. The CLI ships with
# `gridseak setup`, which auto-writes the Cursor + Windsurf mcp.json plus
# the Cursor rule that teaches the agent when to call each MCP tool, and
# prints copy-paste instructions for Claude Code + Codex.
cat <<EOF

Next steps
----------
  1. Verify the install:
       $BIN_DIR/gridseak doctor

  2. Wire GridSeak into your IDE(s) (writes mcp.json + Cursor rule):
       $BIN_DIR/gridseak setup

  3. Run your first scan:
       $BIN_DIR/gridseak scan .

  4. Open a fresh chat in your IDE and ask: "what's risky to refactor here?"
     The agent should call gridseak_get_recommendations within its first
     two tool calls. If it doesn't:
       $BIN_DIR/gridseak setup --verify

Full walkthrough: https://gridseak.com/cli
EOF
