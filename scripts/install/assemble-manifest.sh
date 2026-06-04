#!/usr/bin/env bash
# scripts/install/assemble-manifest.sh — merge per-target CLI tarballs
# into a single cli-manifest.json.
#
# build-cli-release.sh emits one tarball (+ .sha256 sidecar) per target it
# is asked to build, plus a manifest covering only that run. When releases
# are built across several runners (one OS per target), each runner produces
# a partial set. This script reconciles them: point it at a directory holding
# every `gridseak-<version>-<triple>.tar.gz` and it writes the combined
# manifest the installer (install.sh / install.ps1) consumes.
#
# It recomputes SHA256 + size from the tarballs themselves rather than
# trusting sidecars, so a corrupted upload cannot produce a manifest that
# "verifies". The .sha256 sidecars remain for humans / curl-based checks.
#
# Usage:
#   scripts/install/assemble-manifest.sh <dir> <version>
#
# Output:
#   <dir>/cli-manifest.json   (overwrites)
set -euo pipefail

DIR="${1:?usage: assemble-manifest.sh <dir> <version>}"
VERSION="${2:?usage: assemble-manifest.sh <dir> <version>}"
cd "$DIR"

shopt -s nullglob
ARCHIVES=(gridseak-"$VERSION"-*.tar.gz)
if [[ ${#ARCHIVES[@]} -eq 0 ]]; then
  echo "[assemble-manifest] no gridseak-$VERSION-*.tar.gz in $DIR" >&2
  exit 1
fi

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

MANIFEST="cli-manifest.json"
TMP="$MANIFEST.tmp"
{
  printf '{\n'
  printf '  "version": "%s",\n' "$VERSION"
  printf '  "artifacts": ['
} > "$TMP"

first=1
# Sort for a deterministic manifest regardless of glob/filesystem order.
IFS=$'\n' ARCHIVES=($(printf '%s\n' "${ARCHIVES[@]}" | sort)); unset IFS
for archive in "${ARCHIVES[@]}"; do
  # gridseak-<version>-<triple>.tar.gz  ->  <triple>
  triple="${archive#gridseak-$VERSION-}"
  triple="${triple%.tar.gz}"
  [[ -n "$triple" && "$triple" != "$archive" ]] || {
    echo "[assemble-manifest] could not parse triple from $archive" >&2
    exit 1
  }
  hash="$(sha256_of "$archive")"
  size="$(wc -c < "$archive" | tr -d ' ')"

  if [[ $first == 1 ]]; then first=0; else printf ',' >> "$TMP"; fi
  printf '\n    {"target": "%s", "url": "%s", "sha256": "%s", "size": %s}' \
    "$triple" "$archive" "$hash" "$size" >> "$TMP"
  echo "[assemble-manifest] $triple  $size bytes  $hash"
done

printf '\n  ]\n}\n' >> "$TMP"
mv "$TMP" "$MANIFEST"
echo "[assemble-manifest] wrote $DIR/$MANIFEST (${#ARCHIVES[@]} artifacts)"
