"""Check every target `gridseak audit` could plan (the eligibility rules of
gridseak-cli/src/audit.rs) against the TypeScript checker: the answer key (the
reverse closure over `via=call` edges) must equal the closure of direct calls
in ts-oracle/audit-oracle.js output.

usage: ts_audit_keys.py <repo-root> <graph.sqlite> <audit-oracle.json>"""
import json, sys, collections, os, sqlite3
root, dbp, truthp = os.path.realpath(sys.argv[1]) + "/", sys.argv[2], sys.argv[3]
repo = os.path.basename(root.rstrip("/"))
kinds = {"call", "new", "super", "tag"}
con = sqlite3.connect(dbp)
nodes = {}
for nid, kind, fqn, loc, syn in con.execute("select id, kind, fqn, location, coalesce(json_extract(properties,'$.synthetic'),0) from nodes"):
    l = json.loads(loc); f = (l.get("file") or "").replace(root, ""); name = fqn.rsplit("::", 1)[-1]
    nodes[nid] = dict(file=f, line=l.get("start_line", 0), name=name,
                      callable=kind in ("Function", "Method") and not syn and not name.startswith("__file_"))
callers = collections.defaultdict(list)
for a, b in con.execute("select from_id,to_id from edges where json_extract(kind,'$.kind')='Call' and coalesce(json_extract(provenance,'$.via'),'call')='call'"):
    if a != b: callers[b].append(a)
unindexed = set(json.loads((con.execute("select value from metadata where key='resolution_compiler_unindexed_files'").fetchone() or ["[]"])[0]))
partial = {fp.replace(root, "") for fp, pj in con.execute("select file_path,payload_json from file_extraction_coverage") if json.loads(pj).get("confidence") == "Low"}
def is_test(f):
    b = f.rsplit("/", 1)[-1]
    return b.endswith("_test.go") or ".test." in b or ".spec." in b or b.startswith("test_") or "/__tests__/" in f
def closure(seed):
    seen = {seed}; out = []; q = collections.deque([(seed, 0)]); md = 0
    while q:
        x, d = q.popleft()
        for c in callers.get(x, ()):
            n = nodes.get(c)
            if not n or not n["callable"] or c in seen: continue
            seen.add(c); out.append(c); md = max(md, d + 1); q.append((c, d + 1))
    return out, md
# oracle
truth = json.load(open(truthp))
ocallers = collections.defaultdict(set); onames = {}
for r in truth:
    onames[(r["callee_file"], r["callee_line"])] = r["callee_name"]
    if "caller_file" in r: onames.setdefault((r["caller_file"], r["caller_line"]), r.get("caller_name"))
    if r["kind"] in kinds and not r["via_iface"] and "caller_file" in r:
        a = (r["caller_file"], r["caller_line"]); b = (r["callee_file"], r["callee_line"])
        if a != b: ocallers[b].add(a)
byfile = collections.defaultdict(list)
for k in onames: byfile[k[0]].append(k)
def okey(nid):
    n = nodes[nid]; k = (n["file"], n["line"])
    if k in onames: return k
    c = sorted([x for x in byfile[n["file"]] if onames[x] == n["name"]], key=lambda x: abs(x[1] - n["line"]))
    return c[0] if c and abs(c[0][1] - n["line"]) <= 3 else k
def oclosure(seed):
    seen = {seed}; q = [seed]
    while q:
        x = q.pop()
        for c in ocallers.get(x, ()):
            if c not in seen: seen.add(c); q.append(c)
    seen.discard(seed); return seen
elig = 0; exact = 0; bad = []
for nid, n in nodes.items():
    if not n["callable"] or not n["file"] or is_test(n["file"]) or n["file"] in unindexed or n["file"] in partial: continue
    cl, d = closure(nid)
    if d < 2 or not (8 <= len(cl) <= 400): continue
    if partial and any(nodes.get(c, {}).get("file") in partial for x in [nid] + cl for c in callers.get(x, ())): continue
    elig += 1
    key = {okey(c) for c in cl}; orc = oclosure(okey(nid))
    if key == orc: exact += 1
    else: bad.append((n["file"], n["line"], n["name"], len(key), len(orc), sorted(orc - key)[:3], sorted(key - orc)[:3]))
print(f"{repo}: eligible targets={elig} exact={exact}")
for b in bad[:20]: print("  ", b)
