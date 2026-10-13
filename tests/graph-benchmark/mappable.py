# Recall restricted to truth edges whose caller and callee lines both carry a node in the tool's graph.
import json, sys, sqlite3, os, glob, collections
truth=json.load(open(sys.argv[1])); src=sys.argv[2]; scope=sys.argv[3]; root=os.path.realpath(sys.argv[4]) if len(sys.argv)>4 else ''
lines=set(); E=set()
if src.endswith('.sqlite'):
    con=sqlite3.connect(src); pos={}
    for nid,loc in con.execute("select id,location from nodes where kind='Function'"):
        l=json.loads(loc); pos[nid]=(os.path.relpath(os.path.realpath(l['file']),root),l['start_line'])
    lines=set(pos.values())
    E={(pos[a],pos[b]) for a,b in con.execute("select from_id,to_id from edges where json_extract(kind,'$.kind')='Call'") if a in pos and b in pos}
else:
    g=json.load(open(src)); pos={}
    for n in g['nodes']:
        loc=n.get('source_location') or ''
        if n.get('file_type')=='code' and loc[1:].isdigit(): pos[n['id']]=(n['source_file'],int(loc[1:]))
    lines=set(pos.values())
    E={(pos[l['source']],pos[l['target']]) for l in g['links'] if l.get('relation')=='calls' and l['source'] in pos and l['target'] in pos}
ks={((r['caller_file'],r['caller_line']),(r['callee_file'],r['callee_line'])) for r in truth if r['is_call'] and not r.get('via_iface')}
ks={k for k in ks if k[0]!=k[1] and k[0][0].startswith(scope)}
m=[k for k in ks if k[0] in lines and k[1] in lines]
print(f"mappable {len(m)}/{len(ks)} truth call edges; recall on mappable {sum(1 for k in m if k in E)/max(1,len(m)):.1%}")
