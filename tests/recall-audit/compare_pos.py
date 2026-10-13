# Position-based audit: truth = (use position, callee declaration); GridSeak must
# have an edge from the function enclosing the use to the function declared there.
import json, sqlite3, sys, os, collections
truth=json.load(open(sys.argv[1])); con=sqlite3.connect(sys.argv[2]); root=os.path.realpath(sys.argv[3])
fn=collections.defaultdict(list); nodes={}
for nid,loc,fqn in con.execute("select id,location,fqn from nodes where kind='Function'"):
    l=json.loads(loc); f=os.path.relpath(os.path.realpath(l['file']),root)
    n=(nid,f,l['start_line'],l.get('start_char',0),l['end_line'],l.get('end_char',10**6),fqn.split('::')[-1].split('.')[-1])
    fn[f].append(n); nodes[nid]=n
def span(n): return (n[4]-n[2], n[5]-n[3])
def enclosing(f,line,col):
    c=[n for n in fn[f] if (n[2],n[3])<=(line,col)<=(n[4],n[5])]
    return min(c,key=span) if c else None
def decl(f,line,name):
    c=[n for n in fn[f] if n[2]<=line<=n[4] and n[6]==name]
    return min(c,key=span) if c else None
edges={}
for a,b,p in con.execute("select from_id,to_id,provenance from edges where kind like '%Call%'"):
    edges[(a,b)]=json.loads(p)['source']
stats=collections.Counter(); per_callee=collections.defaultdict(lambda:[0,0]); missed=[]
tkeys=set()
for r in truth:
    cal=enclosing(r['ref_file'],r['ref_line'],r['ref_col']); cee=decl(r['callee_file'],r['callee_line'],r['callee_name'])
    if cal is None: stats['use_outside_any_function']+=1; continue
    if cee is None: stats['callee_has_no_gs_node']+=1; continue
    if cal[0]==cee[0]: stats['self_reference']+=1; continue
    kind='call' if r['is_call'] else 'value_ref'
    tkeys.add((cal[0],cee[0])); stats[kind+'_total']+=1
    hit=(cal[0],cee[0]) in edges; stats[kind+'_hit']+=hit
    per_callee[cee[0]][0]+=1; per_callee[cee[0]][1]+=hit
    if not hit and len(missed)<10: missed.append(f"{r['ref_file']}:{r['ref_line']} uses {r['callee_name']} ({r['callee_file']}:{r['callee_line']})")
res=dict(stats)
for k in ('call','value_ref'):
    if stats[k+'_total']: res[k+'_recall']=round(stats[k+'_hit']/stats[k+'_total'],4)
res['symbols_with_users']=len(per_callee)
res['symbols_flagged']=sum(1 for v in per_callee.values() if v[1]>0)
res['symbols_complete']=sum(1 for v in per_callee.values() if v[0]==v[1])
res['symbol_recall']=round(res['symbols_flagged']/max(1,len(per_callee)),4)
res['symbol_complete_recall']=round(res['symbols_complete']/max(1,len(per_callee)),4)
comp=[k for k,s in edges.items() if s=='Compiler']
res['compiler_edges']=len(comp); res['compiler_edges_not_in_truth']=sum(1 for k in comp if k not in tkeys)
res['heuristic_edges']=sum(1 for s in edges.values() if s!='Compiler'); res['heuristic_not_in_truth']=sum(1 for k,s in edges.items() if s!='Compiler' and k not in tkeys)
res['missed_samples']=missed
fps=[f"{nodes[a][1]}:{nodes[a][2]} {nodes[a][6]} -> {nodes[b][1]}:{nodes[b][2]} {nodes[b][6]}" for a,b in comp if (a,b) not in tkeys and a in nodes and b in nodes][:10]
res['extra_compiler_samples']=fps
print(json.dumps(res,indent=1))
# classify extra compiler edges: same caller uses a same-named symbol per truth => dispatch/overload
names=collections.defaultdict(set)
for r in truth:
    cal=enclosing(r['ref_file'],r['ref_line'],r['ref_col'])
    if cal: names[cal[0]].add(r['callee_name'])
cls=collections.Counter(); unexplained=[]
for a,b in comp:
    if (a,b) in tkeys or a not in nodes or b not in nodes: continue
    if nodes[b][6] in names[a]: cls['same_name_dispatch_or_overload']+=1
    else:
        cls['unexplained']+=1
        if len(unexplained)<12: unexplained.append(f"{nodes[a][1]}:{nodes[a][2]} {nodes[a][6]} -> {nodes[b][1]}:{nodes[b][2]} {nodes[b][6]}")
print(dict(cls)); print("\n".join(unexplained))
