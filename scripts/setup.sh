#!/usr/bin/env bash
# scripts/setup.sh — single entry point for fresh-clone setup and on-demand
# fetching of optional external assets.
#
# WHY THIS EXISTS
# ---------------
# This repo is intentionally minimal: source, docs, configs, small fixtures.
# Anything large (regression fixtures, historical evidence archives,
# third-party canary repos) lives outside git and is fetched on demand.
# Every fetch is sha-pinned via experiments/artifacts.lock so a clean clone
# plus this script is a deterministic recipe for "everything a developer
# might want locally".
#
# SUBCOMMANDS
# -----------
#   dev                  Configure git hooks + install desktop UI deps.
#                        Run once after a fresh clone.
#   fixtures             Fetch sha-pinned regression fixtures (rev6.1).
#   historical-baselines Fetch sha-pinned historical experiment outputs.
#   canary-repos         Clone pinned third-party canary repos to an
#                        ignored local path (default .gridseak/canary-repos).
#   check                Verify required tooling + artifact registry.
#   help                 Print this help.
#
# COMMON FLAGS (passed through to fetchers)
# -----------------------------------------
#   --force        Overwrite existing extracted files / cloned dirs.
#   --check        Verify-only (download + sha256, do not extract).
#
# EXAMPLES
# --------
#   scripts/setup.sh dev
#   scripts/setup.sh fixtures
#   scripts/setup.sh fixtures --check
#   scripts/setup.sh historical-baselines --force
#   scripts/setup.sh canary-repos
#   CANARY_ROOTS_DIR=$HOME/canaries scripts/setup.sh canary-repos

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIB_DIR="${REPO_ROOT}/scripts/lib"
ARTIFACTS_LOCK="${REPO_ROOT}/experiments/artifacts.lock"
CANARY_LOCK="${REPO_ROOT}/experiments/canary_repos.lock"

print_help() { sed -n '2,37p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

require_tool() {
  local tool="$1"
  local hint="${2:-}"
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "missing tool: ${tool}${hint:+ (${hint})}" >&2
    return 1
  fi
}

cmd_dev() {
  echo "[dev] verifying core tools ..."
  local missing=0
  require_tool git              || missing=1
  require_tool cargo            "rustup.rs"            || missing=1
  require_tool python3                                   || missing=1
  require_tool jq               "brew install jq"       || missing=1
  require_tool zstd             "brew install zstd"     || missing=1
  if [ "${missing}" -ne 0 ]; then
    echo "[dev] install the tools above, then re-run scripts/setup.sh dev" >&2
    return 1
  fi

  echo "[dev] configuring git hooks (.githooks) ..."
  ( cd "${REPO_ROOT}" && git config core.hooksPath .githooks )

  if command -v pnpm >/dev/null 2>&1; then
    echo "[dev] installing desktop UI dependencies (pnpm) ..."
    pnpm -C "${REPO_ROOT}/desktop/ui" install --frozen-lockfile
  else
    echo "[dev] WARN: pnpm not on PATH; skipping desktop UI deps."
    echo "[dev]       install pnpm (https://pnpm.io/installation) before running"
    echo "[dev]       'pnpm -C desktop/ui build' or the pre-push hook UI typecheck."
  fi

  echo "[dev] done."
  echo
  echo "Optional next steps:"
  echo "  scripts/setup.sh fixtures              # rev6.1 regression-gate fixture"
  echo "  scripts/setup.sh historical-baselines  # rev3..rev11 evidence archive"
  echo "  scripts/setup.sh canary-repos          # third-party canary repos"
}

cmd_fixtures() {
  bash "${LIB_DIR}/fetch_artifact.sh" rev6_1_regression_fixture "$@"
}

cmd_historical_baselines() {
  bash "${LIB_DIR}/fetch_artifact.sh" historical_baselines "$@"
}

cmd_canary_repos() {
  if [ ! -f "${CANARY_LOCK}" ]; then
    echo "error: ${CANARY_LOCK} not found" >&2
    return 1
  fi
  require_tool git || return 1
  require_tool python3 || return 1

  local force=0
  for arg in "$@"; do
    case "$arg" in
      --force) force=1 ;;
      --check) echo "[canary-repos] --check: validating lockfile only";
               python3 -c "import json; json.load(open('${CANARY_LOCK}'))" \
                 && echo "[canary-repos] lockfile parses OK"; return 0 ;;
      *) echo "unknown flag for canary-repos: $arg" >&2; return 2 ;;
    esac
  done

  local default_root
  default_root="$(GS_LOCK="${CANARY_LOCK}" python3 -c \
    "import json,os; print(json.load(open(os.environ['GS_LOCK']))['default_root'])")"
  local roots_dir="${CANARY_ROOTS_DIR:-${REPO_ROOT}/${default_root}}"
  mkdir -p "${roots_dir}"

  echo "[canary-repos] checkout root: ${roots_dir}"

  GS_LOCK="${CANARY_LOCK}" GS_ROOTS="${roots_dir}" GS_FORCE="${force}" python3 - <<'PY'
import json, os, subprocess, sys
lock = os.environ['GS_LOCK']
roots_dir = os.environ['GS_ROOTS']
force = os.environ['GS_FORCE'] == '1'
with open(lock) as fh:
    data = json.load(fh)
for repo in data['repos']:
    name = repo['name']
    target = os.path.join(roots_dir, name)
    if os.path.isdir(os.path.join(target, '.git')):
        if force:
            print(f"[canary-repos] {name}: --force, refetching ref {repo['ref']}")
        else:
            print(f"[canary-repos] {name}: present, ensuring ref {repo['ref']}")
        subprocess.run(['git', '-C', target, 'fetch', '--tags', '--quiet', 'origin'], check=True)
        subprocess.run(['git', '-C', target, 'checkout', '--quiet', repo['ref']], check=True)
        continue
    print(f"[canary-repos] {name}: cloning {repo['url']} @ {repo['ref']}")
    subprocess.run(['git', 'clone', '--quiet', repo['url'], target], check=True)
    subprocess.run(['git', '-C', target, 'checkout', '--quiet', repo['ref']], check=True)
print("[canary-repos] done.")
PY
}

cmd_check() {
  echo "[check] tools ..."
  require_tool git    || true
  require_tool cargo  || true
  require_tool python3 || true
  require_tool jq     || true
  require_tool zstd   || true
  command -v pnpm >/dev/null 2>&1 \
    && echo "  pnpm: $(pnpm --version)" \
    || echo "  pnpm: missing (UI typecheck/build will skip)"
  echo "[check] artifact registry: ${ARTIFACTS_LOCK}"
  if [ -f "${ARTIFACTS_LOCK}" ]; then
    GS_LOCK="${ARTIFACTS_LOCK}" python3 -c \
      "import json,os; data=json.load(open(os.environ['GS_LOCK'])); print('  artifacts:', ', '.join(sorted(data['artifacts'].keys())))"
  else
    echo "  MISSING"
  fi
  echo "[check] canary lock: ${CANARY_LOCK}"
  if [ -f "${CANARY_LOCK}" ]; then
    GS_LOCK="${CANARY_LOCK}" python3 -c \
      "import json,os; data=json.load(open(os.environ['GS_LOCK'])); print('  repos:', ', '.join(r['name'] for r in data['repos']))"
  else
    echo "  MISSING"
  fi
  echo "[check] git hooks path: $(git -C "${REPO_ROOT}" config --get core.hooksPath || echo "(unset)")"
}

main() {
  local sub="${1:-help}"
  shift || true
  case "${sub}" in
    dev) cmd_dev "$@" ;;
    fixtures) cmd_fixtures "$@" ;;
    historical-baselines) cmd_historical_baselines "$@" ;;
    canary-repos) cmd_canary_repos "$@" ;;
    check) cmd_check "$@" ;;
    -h|--help|help) print_help ;;
    *) echo "unknown subcommand: ${sub}" >&2; print_help; exit 2 ;;
  esac
}

main "$@"
