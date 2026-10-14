"""Compare GridSeak's static Python call edges with the runtime witness.

usage: runtime_check.py <repo-root> <graph.sqlite> <runtime.json> <mypy-oracle.json> [tasks.json] [-v]

Runtime pairs (runtime_witness.py) are mapped to named functions: a lambda,
comprehension or generator frame is credited to the `def` around it, and a
decorated function's code (which starts at its first decorator) to its `def`
line. A pair is *countable* under the audit's static rules when the caller's
body (lambdas included, nested defs excluded) contains a call expression that
names the callee: `f(...)` / `x.f(...)`, or for `__init__`/`__new__`, the
instantiated class `C(...)` or `super().__init__(...)`. Implicit calls
(`with`, iteration, operators, properties, `__call__` on instances, callbacks
passed as values, bare `@decorator` application) are not countable.

Every countable runtime edge GridSeak lacks is classified:
  dispatch      runtime ran an override; GridSeak links the caller to a
                same-named method the static type resolves to (definitional)
  untyped       mypy cannot type the call either (receiver typed Any, e.g.
                pytest fixtures or unannotated parameters); some only Jedi resolves
  dispatch (static type outside the repo)
                mypy binds the call to a declaration outside the repo
                (typing.IO.write) while the run reached a repo override
  gridseak      neither: a GridSeak miss to explain
and the planned keys are checked for callers that only the runtime graph adds.
"""
import ast
import collections
import json
import os
import sqlite3
import sys

args = [a for a in sys.argv[1:] if a != "-v"]
verbose = "-v" in sys.argv
root = os.path.realpath(args[0]) + "/"
dbp, rtp, orp = args[1], args[2], args[3]
tasksp = args[4] if len(args) > 4 else None
repo = os.path.basename(root.rstrip("/"))


# --- source model -------------------------------------------------------------
class FileModel:
    def __init__(self, rel):
        self.rel = rel
        try:
            self.tree = ast.parse(open(root + rel, encoding="utf-8").read())
        except (OSError, SyntaxError, ValueError):
            self.tree = None
        self.defs = []  # (first_line, last_line, def_line, node)
        self.start_to_def = {}
        if self.tree is not None:
            for n in ast.walk(self.tree):
                if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    first = min([n.lineno] + [d.lineno for d in n.decorator_list])
                    self.defs.append((first, n.end_lineno, n.lineno, n))
                    self.start_to_def[first] = n.lineno
                    self.start_to_def[n.lineno] = n.lineno
        self.called = {}

    def named_def(self, line, name):
        """Def line of the named function owning code that starts at `line`."""
        if not name.startswith("<"):
            return self.start_to_def.get(line, line)
        inner = [d for d in self.defs if d[0] <= line <= d[1] and d[0] != line]
        if not inner:
            return None
        return max(inner, key=lambda d: d[0])[2]

    def calls_in(self, def_line):
        """Names called directly in the body of the def on `def_line`."""
        if def_line in self.called:
            return self.called[def_line]
        out = set()
        node = next((d[3] for d in self.defs if d[2] == def_line), None)
        if node is not None:
            stack = list(node.body)
            while stack:
                n = stack.pop()
                if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    # decorators and defaults run here; the body does not
                    stack.extend(n.decorator_list)
                    stack.extend(n.args.defaults + [d for d in n.args.kw_defaults if d is not None])
                    continue
                if isinstance(n, ast.ClassDef):
                    stack.extend(n.decorator_list + n.bases)
                    stack.extend(s for s in n.body if not isinstance(s, (ast.FunctionDef, ast.AsyncFunctionDef)))
                    continue
                if isinstance(n, ast.Call):
                    f = n.func
                    if isinstance(f, ast.Name):
                        out.add(f.id)
                    elif isinstance(f, ast.Attribute):
                        out.add(f.attr)
                stack.extend(ast.iter_child_nodes(n))
        self.called[def_line] = out
        return out


models = {}


def model(rel):
    if rel not in models:
        models[rel] = FileModel(rel)
    return models[rel]


# --- GridSeak graph ---------------------------------------------------------------
con = sqlite3.connect(dbp)
nodes = {}
for nid, kind, fqn, loc in con.execute("select id, kind, fqn, location from nodes where kind in ('Function','Method')"):
    l = json.loads(loc)
    nodes[nid] = ((l.get("file") or "").replace(root, ""), l.get("start_line", 0), fqn.rsplit("::", 1)[-1])
gs = {}
gcallers = collections.defaultdict(set)
for a, b, via in con.execute("select from_id, to_id, coalesce(json_extract(provenance,'$.via'),'call') from edges where json_extract(kind,'$.kind')='Call'"):
    if a in nodes and b in nodes and a != b:
        k = (nodes[a][:2], nodes[b][:2])
        if gs.get(k) != "call":
            gs[k] = via
        if via == "call":
            gcallers[nodes[b][:2]].add(nodes[a][:2])
gname = {v[:2]: v[2] for v in nodes.values()}
gby_name = collections.defaultdict(set)
for v in nodes.values():
    gby_name[v[2]].add(v[:2])

oracle = json.load(open(orp))
unres = {(u["caller_file"], u["caller_line"], u["name"]) for u in oracle["unresolved"]}
# (caller, called name) pairs mypy bound to an in-repo function, and those
# only Jedi resolved (the call is untyped for mypy and Pyright alike)
oracle_resolved = {((r["caller_file"], r["caller_line"]), r["callee_name"]) for r in oracle["rows"]
                   if r.get("resolver", "mypy") == "mypy"}
jedi_resolved = {((r["caller_file"], r["caller_line"]), r["callee_name"]) for r in oracle["rows"]
                 if r.get("resolver") == "jedi"}
oracle_edges = {((r["caller_file"], r["caller_line"]), (r["callee_file"], r["callee_line"])) for r in oracle["rows"]}

# --- runtime pairs -------------------------------------------------------------
rt = json.load(open(rtp))
countable = {}
skipped = collections.Counter()
for r in rt:
    a, b = r["caller"], r["callee"]
    if b["name"].startswith("<"):
        skipped["callee is a lambda/comprehension"] += 1
        continue
    ma, mb = model(a["file"]), model(b["file"])
    ca = ma.named_def(a["line"], a["name"])
    cb = mb.named_def(b["line"], b["name"])
    if ca is None:
        skipped["caller is module or class-body code"] += 1
        continue
    if (a["file"], ca) == (b["file"], cb):
        continue
    called = ma.calls_in(ca)
    name = b["name"]
    ok = name in called
    if name in ("__init__", "__new__") and not ok:
        ok = r.get("cls") in called
    if not ok:
        skipped["implicit or value call (not countable)"] += 1
        continue
    countable[((a["file"], ca), (b["file"], cb))] = name

hits = 0
miss = collections.defaultdict(list)
for (ca, cb), name in sorted(countable.items()):
    via = gs.get((ca, cb))
    if via == "call":
        hits += 1
        continue
    if via is not None:
        miss[f"gridseak has it as {via}"].append((ca, cb))
        continue
    # static call to a same-named method elsewhere (base declaration)?
    if any(gs.get((ca, other)) == "call" for other in gby_name[name] if other != cb):
        miss["dispatch"].append((ca, cb))
    elif (ca[0], ca[1], name) in unres or (ca, name) in jedi_resolved:
        miss["untyped"].append((ca, cb))
    elif (ca, name) not in oracle_resolved:
        # mypy bound the call to a declaration outside the repo (typing.IO.write,
        # a stdlib base class) while the run reached a repo override.
        miss["dispatch (static type outside the repo)"].append((ca, cb))
    else:
        miss["gridseak"].append((ca, cb))

total = len(countable)
print(f"{repo}: runtime pairs={len(rt)} countable={total} gridseak-call-edges-hit={hits} "
      f"recall={hits / total if total else 1:.3f}")
print("  not countable:", dict(skipped))
for k in sorted(miss):
    print(f"  missed [{k}]: {len(miss[k])}")
    if verbose or k == "gridseak":
        for ca, cb in miss[k][:40]:
            print(f"     {ca[0]}:{ca[1]} {gname.get(ca, '?')} -> {cb[0]}:{cb[1]} {gname.get(cb, '?')}")

# --- planned keys vs. the runtime graph -----------------------------------------
if tasksp:
    tasks = json.load(open(tasksp))
    tasks = tasks if isinstance(tasks, list) else tasks.get("tasks", tasks)
    # Runtime-only edges the static rules count: not overrides reached by
    # dispatch (the question follows the declared type), but untyped calls and
    # GridSeak misses.
    rcallers = collections.defaultdict(set)
    for k in ("untyped", "gridseak"):
        for ca, cb in miss.get(k, []):
            rcallers[cb].add(ca)
    for t in tasks:
        key = {(c["file"], c["line"]) for c in t["answer"]}
        seed = (t["file"], t["line"])
        seen = {seed}
        q = [seed]
        while q:
            x = q.pop()
            for c in gcallers.get(x, set()) | rcallers.get(x, set()):
                if c not in seen:
                    seen.add(c)
                    q.append(c)
        seen.discard(seed)
        extra = sorted(seen - key)
        print(f"  task {t['id']} {t['target']}: key={len(key)} runtime-witnessed callers missing from key={len(extra)}")
        for x in extra[:8]:
            print("     ", x, gname.get(x, "?"))
