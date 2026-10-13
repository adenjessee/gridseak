import json, sqlite3, sys, os, collections
truth_path, db, root = sys.argv[1], sys.argv[2], os.path.realpath(sys.argv[3])
truth = json.load(open(truth_path))
con = sqlite3.connect(db)
nodes = {}
for nid, kind, loc in con.execute("select id,kind,location from nodes where kind in ('Function','Method')"):
    l = json.loads(loc); f = os.path.relpath(os.path.realpath(l['file']), root)
    nodes[nid] = (f, l['start_line'])
gs = {}
for a, b, prov in con.execute("select from_id,to_id,provenance from edges where kind like '%Call%'"):
    if a in nodes and b in nodes:
        p = json.loads(prov); key = (nodes[a], nodes[b])
        tier = p.get('source')
        if key not in gs or tier == 'Compiler': gs[key] = tier
node_locs = set(nodes.values())
def k(r): return ((r['caller_file'], r['caller_line']), (r['callee_file'], r['callee_line']))
cats = collections.OrderedDict(
    direct_calls=[r for r in truth if r['is_call'] and not r['via_iface']],
    iface_calls=[r for r in truth if r['is_call'] and r['via_iface']],
    func_value_refs=[r for r in truth if not r['is_call']],
)
res = {}
for name, rs in cats.items():
    keys = {k(r) for r in rs if k(r)[0] != k(r)[1]}
    hit_any = sum(1 for x in keys if x in gs); hit_c = sum(1 for x in keys if gs.get(x) == 'Compiler')
    callee_missing = sum(1 for x in keys if x[1] not in node_locs)
    res[name] = dict(edges=len(keys), recall_any=round(hit_any/max(1,len(keys)),4), recall_compiler=round(hit_c/max(1,len(keys)),4), callee_node_missing=callee_missing)
# gate-relevant: symbol has >=1 caller (any reference kind) per truth -> does GS show >=1 caller?
by_callee = collections.defaultdict(set)
for r in truth:
    if k(r)[0] != k(r)[1]: by_callee[k(r)[1]].add(k(r)[0])
gs_in = collections.defaultdict(set)
for (a, b), t in gs.items(): gs_in[b].add(a)
used = [c for c in by_callee if c in node_locs]
detected = sum(1 for c in used if gs_in[c])
all_callers = sum(1 for c in used if by_callee[c] <= gs_in[c])
res['gate_symbols'] = dict(in_repo_symbols_with_callers=len(by_callee), with_gs_node=len(used),
    flagged_has_callers=detected, recall=round(detected/max(1,len(used)),4),
    all_callers_listed=all_callers, complete_recall=round(all_callers/max(1,len(used)),4),
    no_gs_node=[f"{c[0]}:{c[1]}" for c in by_callee if c not in node_locs][:15])
truth_all = {k(r) for r in truth}
comp = [x for x, t in gs.items() if t == 'Compiler']
fp = [x for x in comp if x not in truth_all]
res['precision_compiler'] = dict(edges=len(comp), not_in_truth=len(fp), precision=round(1-len(fp)/max(1,len(comp)),4), samples=[f"{a[0]}:{a[1]} -> {b[0]}:{b[1]}" for a,b in fp[:8]])
ts = [x for x, t in gs.items() if t != 'Compiler']
res['precision_heuristic'] = dict(edges=len(ts), not_in_truth=sum(1 for x in ts if x not in truth_all))
missed = [x for x in {k(r) for r in cats['direct_calls']} if x not in gs and x[0]!=x[1]]
res['missed_direct_samples'] = [f"{a[0]}:{a[1]} -> {b[0]}:{b[1]}" for a,b in missed[:12]]
print(json.dumps(res, indent=1))
