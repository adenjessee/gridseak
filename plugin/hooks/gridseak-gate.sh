#!/usr/bin/env bash
# Bundled plugin hook — host JSON on stdin, verdict on stdout.
# Always exit 0 with JSON so a failClosed host cannot DoS the IDE.
set -u
MODE="${1:-claude}"
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)"
if [ -n "${GRIDSEAK_BIN:-}" ]; then
  BIN="$GRIDSEAK_BIN"
elif [ -x "$REPO_ROOT/target/release/gridseak" ]; then
  BIN="$REPO_ROOT/target/release/gridseak"
elif [ -x "$REPO_ROOT/target/debug/gridseak" ]; then
  BIN="$REPO_ROOT/target/debug/gridseak"
else
  BIN="gridseak"
fi

fail_json() {
  # A broken Stop hook must never trap the agent in its turn: fail open.
  if [ "$MODE" = "stop" ]; then
    printf '{}\n'
    exit 0
  fi
  printf '%s\n' "{\"permission\":\"ask\",\"user_message\":\"$1\",\"failClosed\":true}"
  exit 0
}

if [ ! -x "$BIN" ] && ! command -v "$BIN" >/dev/null 2>&1; then
  fail_json "gridseak binary not found — build target/release/gridseak or set GRIDSEAK_BIN"
fi

input=$(cat)
if [ -n "${GRIDSEAK_GATE_TEE:-}" ]; then
  printf '%s\n' "$input" >> "$GRIDSEAK_GATE_TEE" || true
fi

out_file="$(mktemp -t gridseak-gate.XXXXXX)"
err_file="$(mktemp -t gridseak-gate.XXXXXX)"
set +e
if [ "$MODE" = "card" ] || [ "$MODE" = "--card" ]; then
  printf '%s' "$input" | "$BIN" gate --card >"$out_file" 2>"$err_file"
elif [ "$MODE" = "stop" ]; then
  printf '%s' "$input" | "$BIN" gate --stop >"$out_file" 2>"$err_file"
else
  printf '%s' "$input" | "$BIN" gate --format "$MODE" >"$out_file" 2>"$err_file"
fi
status=$?
set +e
if [ "$status" -eq 0 ] && [ -s "$out_file" ]; then
  cat "$out_file"
  rm -f "$out_file" "$err_file"
  exit 0
fi
if [ "$MODE" = "stop" ]; then
  rm -f "$out_file" "$err_file"
  fail_json ""
fi
reason="$(tr '\n' ' ' <"$err_file" | tr -d '"\\')"
if [ -z "$reason" ]; then
  reason="gridseak gate exited $status without a verdict"
fi
printf '%s\n' "{\"permission\":\"ask\",\"user_message\":\"$reason\",\"failClosed\":true}"
rm -f "$out_file" "$err_file"
exit 0
