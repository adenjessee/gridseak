"""Runtime witness: record caller -> callee pairs while a repo's own test
suite runs, as a recall check on GridSeak's static Python call edges.

Load as a pytest plugin (Python >= 3.12, uses `sys.monitoring`):

    PYTHONPATH=<this dir> RW_ROOT=<repo> RW_OUT=<out.json> \
        <repo venv>/bin/python -m pytest -p runtime_witness -q tests

Every function start (PY_START) whose code lives under RW_ROOT is paired with
the frame that started it, when that frame's code is under RW_ROOT too. Code
outside the repo is disabled per code object, so the overhead stays small.

Output rows: caller/callee code identity (file, first line, qualname) plus,
for `__init__`/`__new__` callees, the class actually instantiated
(`type(self).__name__`), which `runtime_check.py` needs to tell `C()` from
`super().__init__()`. Runtime edges are a lower bound (executed paths only)
and include implicit and dynamic calls; `runtime_check.py` keeps the ones the
static rules count.
"""
import json
import os
import sys
import threading

ROOT = os.path.realpath(os.environ.get("RW_ROOT", os.getcwd())) + os.sep
OUT = os.environ.get("RW_OUT", "runtime-edges.json")
SKIP = tuple(p for p in (os.environ.get("RW_SKIP", "") or "").split(":") if p)
TOOL = 4  # sys.monitoring tool id (0-5; 4 is free in practice)

_in_repo: dict = {}
_edges: dict = {}
_lock = threading.Lock()


def _repo(code):
    r = _in_repo.get(code)
    if r is None:
        name = code.co_filename
        f = os.path.realpath(name)
        # `<frozen os>`, `<string>`: no file; realpath would place it in cwd.
        r = not name.startswith("<") and f.startswith(ROOT) and not any(s in f for s in SKIP)
        _in_repo[code] = r
    return r


def _on_start(code, offset):
    if not _repo(code):
        return sys.monitoring.DISABLE
    try:
        frame = sys._getframe(1)
    except ValueError:
        return None
    if frame.f_code is not code:
        return None
    caller = frame.f_back
    if caller is None or not _repo(caller.f_code):
        return None
    cls = None
    if code.co_name in ("__init__", "__new__") and code.co_argcount:
        first = frame.f_locals.get(code.co_varnames[0])
        if first is not None:
            cls = first.__name__ if code.co_name == "__new__" and isinstance(first, type) else type(first).__name__
    key = (caller.f_code, code, cls)
    with _lock:
        _edges[key] = _edges.get(key, 0) + 1
    return None


def _ident(code):
    return {
        "file": os.path.realpath(code.co_filename)[len(ROOT):],
        "line": code.co_firstlineno,
        "qualname": code.co_qualname,
        "name": code.co_name,
    }


def start():
    sys.monitoring.use_tool_id(TOOL, "runtime_witness")
    sys.monitoring.register_callback(TOOL, sys.monitoring.events.PY_START, _on_start)
    sys.monitoring.set_events(TOOL, sys.monitoring.events.PY_START)


def stop():
    sys.monitoring.set_events(TOOL, 0)
    sys.monitoring.register_callback(TOOL, sys.monitoring.events.PY_START, None)
    sys.monitoring.free_tool_id(TOOL)
    rows = []
    for (a, b, cls), n in _edges.items():
        rows.append({"caller": _ident(a), "callee": _ident(b), "cls": cls, "count": n})
    with open(OUT, "w") as fh:
        json.dump(rows, fh)
    print(f"\nruntime_witness: {len(rows)} caller->callee pairs -> {OUT}", file=sys.stderr)


def pytest_configure(config):
    start()


def pytest_unconfigure(config):
    stop()
