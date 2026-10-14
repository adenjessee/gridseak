"""Check every target `gridseak audit` could plan on a Python repo (the
eligibility rules of gridseak-cli/src/audit.rs) against the mypy oracle
(py-oracle/mypy_oracle.py): the answer key (reverse closure over `via=call`
edges) must equal the closure of direct calls in the oracle output. Also
diffs the direct edges themselves, which is where disagreements are explained.

usage: py_audit_keys.py <repo-root> <graph.sqlite> <mypy-oracle.json> [tasks.json] [-v]"""
import collections
import json
import os
import sqlite3
import sys

args = [a for a in sys.argv[1:] if a != "-v"]
verbose = "-v" in sys.argv
root, dbp, truthp = os.path.realpath(args[0]) + "/", args[1], args[2]
tasksp = args[3] if len(args) > 3 else None
repo = os.path.basename(root.rstrip("/"))
KINDS = {"call", "init", "super", "prop"}

con = sqlite3.connect(dbp)
nodes = {}
for nid, kind, fqn, loc, syn in con.execute(
        "select id, kind, fqn, location, coalesce(json_extract(properties,'$.synthetic'),0) from nodes"):
    l = json.loads(loc)
    f = (l.get("file") or "").replace(root, "")
    name = fqn.rsplit("::", 1)[-1]
    nodes[nid] = dict(file=f, line=l.get("start_line", 0), name=name,
                      callable=kind in ("Function", "Method") and not syn and not name.startswith("__file_"))
callers = collections.defaultdict(list)
gedges = set()
for a, b in con.execute(
        "select from_id,to_id from edges where json_extract(kind,'$.kind')='Call' "
        "and coalesce(json_extract(provenance,'$.via'),'call')='call'"):
    if a != b:
        callers[b].append(a)
        if nodes.get(a, {}).get("callable") and b in nodes:
            gedges.add(((nodes[a]["file"], nodes[a]["line"]), (nodes[b]["file"], nodes[b]["line"])))
unindexed = set(json.loads((con.execute(
    "select value from metadata where key='resolution_compiler_unindexed_files'").fetchone() or ["[]"])[0]))
partial = {fp.replace(root, "") for fp, pj in con.execute("select file_path,payload_json from file_extraction_coverage")
           if json.loads(pj).get("confidence") == "Low"}
gname = {(n["file"], n["line"]): n["name"] for n in nodes.values() if n["callable"]}
# Calls GridSeak's index could not resolve (audit.rs skips targets whose
# caller set shares a name with one).
unresolved_names = {name for _, _, name in json.loads((con.execute(
    "select value from metadata where key='resolution_compiler_unresolved_calls'").fetchone() or ["[]"])[0])}


def is_test(f):
    b = f.rsplit("/", 1)[-1]
    return b.endswith("_test.go") or ".test." in b or ".spec." in b or b.startswith("test_") or "/__tests__/" in f


def closure(seed):
    seen = {seed}; out = []; q = collections.deque([(seed, 0)]); md = 0
    while q:
        x, d = q.popleft()
        for c in callers.get(x, ()):
            n = nodes.get(c)
            if not n or not n["callable"] or c in seen:
                continue
            seen.add(c); out.append(c); md = max(md, d + 1); q.append((c, d + 1))
    return out, md


truth = json.load(open(truthp))
ocallers = collections.defaultdict(set)
oedges = {}
onames = {}
for r in truth["rows"]:
    a = (r["caller_file"], r["caller_line"]); b = (r["callee_file"], r["callee_line"])
    onames[a] = r["caller_name"]; onames[b] = r["callee_name"]
    if r["kind"] in KINDS and not r["via_iface"] and a != b:
        ocallers[b].add(a)
        oedges.setdefault((a, b), r)
iface_edges = {((r["caller_file"], r["caller_line"]), (r["callee_file"], r["callee_line"]))
               for r in truth["rows"] if r["via_iface"]}
unres_by_name = collections.defaultdict(list)
for u in truth["unresolved"]:
    unres_by_name[u["name"]].append(u)


def oclosure(seed):
    seen = {seed}; q = [seed]
    while q:
        x = q.pop()
        for c in ocallers.get(x, ()):
            if c not in seen:
                seen.add(c); q.append(c)
    seen.discard(seed)
    return seen


# --- direct edges ------------------------------------------------------------
missing = sorted(set(oedges) - gedges)
extra = sorted(gedges - set(oedges))
print(f"{repo}: direct call edges gridseak={len(gedges)} oracle={len(oedges)} "
      f"both={len(gedges & set(oedges))} oracle-only={len(missing)} gridseak-only={len(extra)}")
kind_of_missing = collections.Counter(oedges[k]["kind"] for k in missing)
print("  oracle-only by kind:", dict(kind_of_missing),
      "| by resolver:", dict(collections.Counter(oedges[k].get("resolver", "mypy") for k in missing)),
      "| through a union-typed receiver:", sum(1 for k in missing if oedges[k].get("union")))
extra_iface = [e for e in extra if e in iface_edges]
print(f"  gridseak-only that the oracle marks as abstract/Protocol dispatch: {len(extra_iface)}")
if verbose:
    for a, b in missing[:60]:
        r = oedges[(a, b)]
        print(f"   MISSING {a[0]}:{a[1]} {onames.get(a)} -> {b[0]}:{b[1]} {onames.get(b)} [{r['kind']}{', ' + r.get('resolver', 'mypy')}{', union' if r.get('union') else ''}] at {r['ref_file']}:{r['ref_line']}")
    for a, b in extra[:60]:
        print(f"   EXTRA   {a[0]}:{a[1]} {gname.get(a)} -> {b[0]}:{b[1]} {gname.get(b)}{' (iface)' if (a, b) in iface_edges else ''}")

# --- keys --------------------------------------------------------------------
def key_report(nid, cl):
    n = nodes[nid]
    key = {(nodes[c]["file"], nodes[c]["line"]) for c in cl}
    orc = oclosure((n["file"], n["line"]))
    # Untyped call sites whose member name matches the target or a key member:
    # the key may be missing callers hidden behind them.
    names = {n["name"]} | {nodes[c]["name"] for c in cl}
    hidden = [u for nm in names for u in unres_by_name.get(nm, [])]
    return key, orc, hidden


elig = exact = hidden_skips = 0
bad = []
for nid, n in nodes.items():
    if not n["callable"] or not n["file"] or is_test(n["file"]) or n["file"] in unindexed or n["file"] in partial:
        continue
    cl, d = closure(nid)
    if d < 2 or not (8 <= len(cl) <= 400):
        continue
    if partial and any(nodes.get(c, {}).get("file") in partial for x in [nid] + cl for c in callers.get(x, ())):
        continue
    if unresolved_names & ({n["name"]} | {nodes[c]["name"] for c in cl}):
        hidden_skips += 1
        continue
    elig += 1
    key, orc, hidden = key_report(nid, cl)
    if key == orc:
        exact += 1
    else:
        bad.append((n["file"], n["line"], n["name"], len(key), len(orc), sorted(orc - key)[:3], sorted(key - orc)[:3]))
print(f"{repo}: eligible targets={elig} exact={exact} "
      f"(skipped {hidden_skips} whose callers share a name with one of {len(unresolved_names)} unresolved call names)")
for b in bad[:25]:
    print("  ", b)

if tasksp:
    tasks = json.load(open(tasksp))
    tasks = tasks if isinstance(tasks, list) else tasks.get("tasks", tasks)
    byloc = {(n["file"], n["line"]): nid for nid, n in nodes.items() if n["callable"]}
    for t in tasks:
        k = {(c["file"], c["line"]) for c in t["answer"]}
        orc = oclosure((t["file"], t["line"]))
        nid = byloc.get((t["file"], t["line"]))
        names = {t["target"]} | {c["name"] for c in t["answer"]}
        hidden = sorted({(u["caller_file"], u["caller_line"], u["caller_name"], u["name"])
                         for nm in names for u in unres_by_name.get(nm, [])})
        print(f"  task {t['id']} {t['target']} key={len(k)} oracle={len(orc)} "
              f"{'EXACT' if k == orc else 'DIFF'} untyped-sites-with-matching-names={len(hidden)}")
        for x in sorted(orc - k)[:10]:
            print("     oracle-only", x, onames.get(x))
        for x in sorted(k - orc)[:10]:
            print("     key-only   ", x, gname.get(x))
        if verbose:
            for h in hidden[:15]:
                print("     untyped", h)
