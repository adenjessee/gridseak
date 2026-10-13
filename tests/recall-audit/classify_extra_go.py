import json, sqlite3, sys, os
truth=json.load(open(sys.argv[1])); con=sqlite3.connect(sys.argv[2]); root=os.path.realpath(sys.argv[3])
nodes={}
for nid,loc,fqn in con.execute("select id,location,fqn from nodes where kind='Function'"):
    l=json.loads(loc); nodes[nid]=(os.path.relpath(os.path.realpath(l['file']),root),l['start_line'],fqn.split('::')[-1])
tk={((r['caller_file'],r['caller_line']),(r['callee_file'],r['callee_line'])) for r in truth}
iface={(r['caller_file'],r['caller_line'],r['callee_name']) for r in truth if r['via_iface']}
allnames={(r['caller_file'],r['caller_line'],r['callee_name']) for r in truth}
fp=[]
for a,b,p in con.execute("select from_id,to_id,provenance from edges where kind like '%Call%'"):
    if a in nodes and b in nodes:
        ca,cb=nodes[a],nodes[b]
        if ((ca[0],ca[1]),(cb[0],cb[1])) not in tk:
            cls='iface-dispatch' if (ca[0],ca[1],cb[2]) in iface else ('same-name-ref' if (ca[0],ca[1],cb[2]) in allnames else 'UNEXPLAINED')
            fp.append((cls,f"{ca[0]}:{ca[1]} -> {cb[0]}:{cb[1]} {cb[2]}",json.loads(p)['confidence']))
from collections import Counter
print(Counter(c for c,_,_ in fp)); [print(x) for x in fp if x[0]!='iface-dispatch']
