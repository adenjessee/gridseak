# Score a Graphify graph.json against a declaration-level oracle:
# truth rows = (caller_file, caller_line) -> (callee_file, callee_line).
import json, sys, collections
truth=json.load(open(sys.argv[1])); rels=set(sys.argv[3].split(',')); scope=sys.argv[4] if len(sys.argv)>4 else ''
pos={}; edges=collections.defaultdict(set)
if sys.argv[2].endswith('.sqlite'):   # GridSeak graph: same scoring, node start line = declaration line
    import sqlite3, os
    con=sqlite3.connect(sys.argv[2]); root=os.path.realpath(sys.argv[5])
    for nid,loc,fqn in con.execute("select id,location,fqn from nodes where kind='Function'"):
        l=json.loads(loc)
        if fqn.endswith('__file_scope__'): continue
        pos[nid]=(os.path.relpath(os.path.realpath(l['file']),root), l['start_line'])
    for a,b,p in con.execute("select from_id,to_id,provenance from edges where json_extract(kind,'$.kind')='Call'"):
        if a in pos and b in pos: edges[(pos[a],pos[b])].add('EXTRACTED' if json.loads(p)['source']=='Compiler' else 'INFERRED')
else:
    g=json.load(open(sys.argv[2]))
    for n in g['nodes']:
        loc=n.get('source_location') or ''
        if n.get('file_type')=='code' and loc.startswith('L') and loc[1:].isdigit():
            pos[n['id']]=(n['source_file'], int(loc[1:]))
    for l in g['links']:
        if l.get('relation') in rels and l['source'] in pos and l['target'] in pos:
            edges[(pos[l['source']],pos[l['target']])].add(l.get('confidence'))
if scope:
    edges={k:v for k,v in edges.items() if k[0][0].startswith(scope) and k[1][0].startswith(scope)}
def k(r): return ((r['caller_file'],r['caller_line']),(r['callee_file'],r['callee_line']))
decls=set(pos.values())
out={}
for name,rs in [('direct_calls',[r for r in truth if r['is_call'] and not r.get('via_iface')]),('value_refs',[r for r in truth if not r['is_call']])]:
    ks={k(r) for r in rs if k(r)[0]!=k(r)[1]}
    out[name]=dict(edges=len(ks),recall=round(sum(1 for x in ks if x in edges)/max(1,len(ks)),4),
                   recall_extracted_only=round(sum(1 for x in ks if 'EXTRACTED' in edges.get(x,()))/max(1,len(ks)),4))
by=collections.defaultdict(set)
for r in truth:
    if k(r)[0]!=k(r)[1]: by[k(r)[1]].add(k(r)[0])
gin=collections.defaultdict(set)
for (a,b) in edges: gin[b].add(a)
have=[c for c in by if c in decls]
out['gate_symbols']=dict(symbols_with_users=len(by),with_node=len(have),flagged=round(sum(1 for c in have if gin[c])/max(1,len(have)),4),
                         complete=round(sum(1 for c in have if by[c]<=gin[c])/max(1,len(have)),4))
tall={k(r) for r in truth}
fp=[x for x in edges if x not in tall]
out['precision']=dict(edges=len(edges),not_in_truth=len(fp),precision=round(1-len(fp)/max(1,len(edges)),4),
    by_conf={c:sum(1 for x in fp if c in edges[x]) for c in ('EXTRACTED','INFERRED')},samples=[f"{a[0]}:{a[1]} -> {b[0]}:{b[1]}" for a,b in fp[:6]])
print(json.dumps(out,indent=1))
# Classify extra edges: does the caller use a same-named symbol per truth (dispatch / overload / nesting)?
import re
def norm(s): return re.sub(r'[^A-Za-z0-9_$]','',s.split('::')[-1].split('.')[-1] if '::' in s else s.replace('()','').split('.')[-1])
names=collections.defaultdict(set)
for r in truth: names[(r['caller_file'],r['caller_line'])].add(r['callee_name'])
label={}
if sys.argv[2].endswith('.sqlite'):
    for nid,fqn in con.execute("select id,fqn from nodes where kind='Function'"): label[pos.get(nid)]=norm(fqn)
else:
    for n in g['nodes']:
        if n['id'] in pos: label[pos[n['id']]]=norm(n['label'])
cls=collections.Counter()
for a,b in fp if not scope else [x for x in edges if x not in tall]:
    cls['same_name_dispatch' if label.get(b) in names.get(a,()) else 'unexplained']+=1
print("extra edges:",dict(cls))
