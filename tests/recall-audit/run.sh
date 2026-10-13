#!/usr/bin/env bash
# Caller-recall audit: GridSeak's compiler-tier call graph vs. an independent
# oracle built directly on each language's own type checker (go/types, the
# TypeScript checker). Neither oracle reads SCIP or GridSeak output.
#
# Usage: tests/recall-audit/run.sh            (from the repo root)
# Needs: go, node, a global `typescript` npm package, sqlite3, python3.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HERE="$ROOT/tests/recall-audit"
CORPORA="$ROOT/.cache/g1"
OUT="$ROOT/target/recall-audit"
TS_PATH="${TS_PATH:-$(npm root -g)/typescript}"
mkdir -p "$CORPORA" "$OUT"

clone_pinned() { # name url sha
  [[ -d "$CORPORA/$1/.git" ]] || git clone -q "$2" "$CORPORA/$1"
  git -C "$CORPORA/$1" checkout -q "$3"
}
clone_pinned chi https://github.com/go-chi/chi.git 71307f9b7e4e9527638bc951c42b782cd1560331
clone_pinned zod https://github.com/colinhacks/zod.git e30870369d5b8f31ff4d0130d4439fd997deb523

cargo build -q -p graphengine-parsing -p gridseak-cli
export PATH="$ROOT/target/debug:$PATH" GRIDSEAK_AUTO_PROVISION=0

scan() { # corpus lang -> prints graph sqlite path
  local data="$OUT/data-$1"
  rm -rf "$data" && mkdir -p "$data"
  (cd "$CORPORA/$1" && gridseak doctor --provision "$2" --progress off >/dev/null)
  (cd "$CORPORA" && gridseak --data-dir "$data" --progress off scan "$1" --lang "$2" >"$data/scan.out" 2>"$data/scan.err")
  ls "$data"/project-graphs/*.sqlite
}

echo "== chi (Go) =="
(cd "$HERE/go-oracle" && go build -o "$OUT/go-oracle" .)
"$OUT/go-oracle" "$CORPORA/chi" >"$OUT/chi.truth.json"
CHI_DB="$(scan chi go)"
python3 "$HERE/compare_decl.py" "$OUT/chi.truth.json" "$CHI_DB" "$CORPORA/chi" | tee "$OUT/chi.result.json"
python3 "$HERE/classify_extra_go.py" "$OUT/chi.truth.json" "$CHI_DB" "$CORPORA/chi"

echo "== zod (TypeScript) =="
TS_PATH="$TS_PATH" node "$HERE/ts-oracle/oracle.js" "$CORPORA/zod" src >"$OUT/zod.truth.json"
ZOD_DB="$(scan zod typescript)"
python3 "$HERE/compare_pos.py" "$OUT/zod.truth.json" "$ZOD_DB" "$CORPORA/zod" | tee "$OUT/zod.result.txt"
