#!/usr/bin/env bash
set -euo pipefail

# ge-analyze calibration runner
# Clones target repos, parses them, runs ge-analyze, and collects results
# for threshold calibration across multiple real-world projects.
#
# Prerequisites:
#   cargo build --release -p graphengine-parsing
#   cargo build --release -p graphengine-analysis --bin ge-analyze
#
# Usage:
#   ./calibrate.sh                  # Run all targets
#   ./calibrate.sh hono zod         # Run specific targets
#   SKIP_PARSE=1 ./calibrate.sh     # Re-analyze without re-parsing

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CALIBRATION_DIR="$SCRIPT_DIR/calibration"
REPOS_DIR="$CALIBRATION_DIR/repos"
FIXTURES_DIR="$SCRIPT_DIR/test-fixtures"
RESULTS_DIR="$CALIBRATION_DIR/results"

PARSER_BIN="$WORKSPACE_ROOT/target/release/graphengine-parsing"
ANALYZER_BIN="$WORKSPACE_ROOT/target/release/ge-analyze"
PARSER_CWD="$WORKSPACE_ROOT/graphengine-parsing"

mkdir -p "$REPOS_DIR" "$FIXTURES_DIR" "$RESULTS_DIR"

# --- Target repositories ---
# Each entry: name|git_url|language|notes
TARGETS=(
  "hono|https://github.com/honojs/hono.git|typescript|Well-maintained TS framework (baseline)"
  "zod|https://github.com/colinhacks/zod.git|typescript|Small focused TS library"
  "trpc|https://github.com/trpc/trpc.git|typescript|TS monorepo with complex structure"
  "express|https://github.com/expressjs/express.git|javascript|Mature JS framework (JS path validation)"
  "fastapi|https://github.com/fastapi/fastapi.git|python|Python framework (Python path validation)"
  "chi|https://github.com/go-chi/chi.git|go|Go HTTP router (Go path validation)"
  "graphengine|LOCAL|rust|Our own Rust codebase (self-analysis)"
)

# Allow filtering targets from CLI args
REQUESTED=("$@")

should_run() {
  local name="$1"
  if [ ${#REQUESTED[@]} -eq 0 ]; then
    return 0
  fi
  for req in "${REQUESTED[@]}"; do
    if [ "$req" = "$name" ]; then
      return 0
    fi
  done
  return 1
}

# --- Build binaries ---
echo "=== Building binaries ==="
if [ ! -f "$PARSER_BIN" ] || [ ! -f "$ANALYZER_BIN" ]; then
  echo "Building release binaries..."
  (cd "$WORKSPACE_ROOT" && cargo build --release -p graphengine-parsing -p graphengine-analysis --bin ge-analyze 2>&1)
  echo "Build complete."
else
  echo "Binaries already exist (use 'cargo build --release' to rebuild)."
fi

# --- Process each target ---
for entry in "${TARGETS[@]}"; do
  IFS='|' read -r name url lang notes <<< "$entry"

  if ! should_run "$name"; then
    continue
  fi

  echo ""
  echo "=========================================="
  echo "Target: $name ($lang)"
  echo "Notes:  $notes"
  echo "=========================================="

  REPO_DIR="$REPOS_DIR/$name"
  DB_PATH="$FIXTURES_DIR/calibration_${name}.sqlite"
  REPORT_PATH="$RESULTS_DIR/${name}_health.json"

  # Clone or update repo
  if [ "$url" = "LOCAL" ]; then
    REPO_DIR="$WORKSPACE_ROOT"
    echo "Using local workspace as repo."
  elif [ -d "$REPO_DIR" ]; then
    echo "Repo already cloned at $REPO_DIR (pull to update)."
    (cd "$REPO_DIR" && git pull --ff-only 2>/dev/null || true)
  else
    echo "Cloning $url..."
    git clone --depth 1 "$url" "$REPO_DIR"
  fi

  # Parse (skip if SKIP_PARSE=1 and DB exists)
  if [ "${SKIP_PARSE:-0}" = "1" ] && [ -f "$DB_PATH" ]; then
    echo "Skipping parse (SKIP_PARSE=1, DB exists)."
  else
    echo "Parsing $name..."
    (cd "$PARSER_CWD" && "$PARSER_BIN" parse --root "$REPO_DIR" --lang "$lang" --db "$DB_PATH" --clear 2>&1) || {
      echo "WARNING: Parse failed for $name, skipping analysis."
      continue
    }
    echo "Parse complete. DB: $DB_PATH"
  fi

  # Analyze
  echo "Running ge-analyze..."
  "$ANALYZER_BIN" --db "$DB_PATH" --output "$REPORT_PATH" 2>&1

  # Extract summary
  echo ""
  echo "--- $name summary ---"
  if command -v python3 &>/dev/null; then
    python3 -c "
import json, sys
with open('$REPORT_PATH') as f:
    r = json.load(f)
s = r['summary']
c = r['health_score_components']
print(f\"  Health Score:       {r['health_score']}/100\")
print(f\"  Nodes:              {s['total_nodes']}\")
print(f\"  Edges:              {s['total_edges']}\")
print(f\"  Functions:          {s['total_functions']}\")
print(f\"  Modules:            {s['total_modules']}\")
print(f\"  Cycles:             {s['cycles_found']}\")
print(f\"  Avg Coupling:       {s['avg_module_coupling']:.3f}\")
print(f\"  Dead Functions:     {s['dead_functions']}\")
print(f\"  Max Call Depth:     {s['max_call_depth']}\")
print(f\"  Tangle Index:       {s['tangle_index']:.3f}\")
print(f\"  Findings:           {len(r['findings'])}\")
print()
print(f\"  Score Components:\")
for name, comp in c.items():
    print(f\"    {name:30s}  {comp['score']:3d}  (weight {comp['weight']:.2f})\")
" 2>/dev/null || echo "  (install python3 for formatted output)"
  else
    echo "  (install python3 for formatted output)"
  fi

  echo ""
done

# --- Cross-project comparison ---
echo ""
echo "=========================================="
echo "Cross-Project Calibration Summary"
echo "=========================================="

if command -v python3 &>/dev/null; then
  python3 -c "
import json, os, glob

results_dir = '$RESULTS_DIR'
files = sorted(glob.glob(os.path.join(results_dir, '*_health.json')))

if not files:
    print('No results found.')
else:
    header = f\"{'Project':15s} {'Score':>5s} {'Nodes':>7s} {'Funcs':>7s} {'Mods':>5s} {'Cycles':>6s} {'AvgCpl':>7s} {'Dead':>5s} {'Depth':>5s} {'Tangle':>7s} {'Finds':>6s}\"
    print(header)
    print('-' * len(header))

    for f in files:
        name = os.path.basename(f).replace('_health.json', '')
        with open(f) as fh:
            r = json.load(fh)
        s = r['summary']
        print(f\"{name:15s} {r['health_score']:5d} {s['total_nodes']:7d} {s['total_functions']:7d} {s['total_modules']:5d} {s['cycles_found']:6d} {s['avg_module_coupling']:7.3f} {s['dead_functions']:5d} {s['max_call_depth']:5d} {s['tangle_index']:7.3f} {len(r['findings']):6d}\")
" 2>/dev/null || echo "(install python3 for cross-project comparison)"
fi

echo ""
echo "Results saved to: $RESULTS_DIR/"
echo "SQLite fixtures saved to: $FIXTURES_DIR/"
echo ""
echo "Next steps:"
echo "  1. Review the cross-project comparison above"
echo "  2. Check if well-maintained projects (hono, zod, express) score 65+"
echo "  3. Check if findings count is 10-60 per project"
echo "  4. Adjust thresholds in health_score.rs if scores don't match intuition"
echo "  5. Verify all language paths produce non-zero nodes (TS, JS, Python, Go, Rust)"
echo "  6. Integration tests in tests/integration_test.rs use calibration fixtures"
