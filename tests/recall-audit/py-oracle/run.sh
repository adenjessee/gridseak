#!/usr/bin/env bash
# Check `gridseak audit` answer keys on a Python repo against the static
# oracle (mypy + Jedi) and the runtime witness (the repo's own tests).
#
# usage: run.sh <repo> <repo-venv-python> <oracle-venv-python> <graph.sqlite> [tasks.json]
#   repo-venv-python    the repo installed (`pip install -e .`) with its test deps
#   oracle-venv-python  a separate venv with pure-Python mypy and jedi:
#                       MYPY_USE_MYPYC=0 pip install --no-binary mypy mypy jedi
#   graph.sqlite        <data-dir>/project-graphs/<id>.sqlite of a `gridseak scan`
# Writes <repo-name>-mypy.json and <repo-name>-runtime.json to $OUT (default .).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$1" && pwd); py=$2; oracle_py=$3; db=$4; tasks=${5:-}
out=${OUT:-.}; name=$(basename "$repo")
"$oracle_py" "$here/mypy_oracle.py" "$repo" "$py" "$out/$name-mypy.json"
(cd "$repo" && PYTHONPATH="$here" RW_ROOT="$repo" RW_OUT="$out/$name-runtime.json" \
    "$py" -m pytest -p runtime_witness -q -p no:cacheprovider tests) || true
python3 "$here/../py_audit_keys.py" "$repo" "$db" "$out/$name-mypy.json" $tasks
python3 "$here/runtime_check.py" "$repo" "$db" "$out/$name-runtime.json" "$out/$name-mypy.json" $tasks
