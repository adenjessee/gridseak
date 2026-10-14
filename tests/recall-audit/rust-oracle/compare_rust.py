#!/usr/bin/env python3
"""Compare a GridSeak Rust graph with the MIR oracle (mir_oracle.py).

Usage: compare_rust.py TRUTH.json COVERED_FILES.json GRAPH.sqlite WORKSPACE_ROOT

Edges are keyed by declaration lines: ((caller_file, fn line), (callee_file,
fn line)). Only callers in files the oracle compiled are scored, so targets
the oracle skipped (or failed to build) do not count as GridSeak errors.
"""
import collections
import json
import os
import sqlite3
import sys

truth_path, covered_path, db, root = sys.argv[1:5]
root = os.path.realpath(root)
truth = json.load(open(truth_path))
covered = set(json.load(open(covered_path)))

con = sqlite3.connect(db)
nodes = {}
for nid, loc in con.execute("select id, location from nodes where kind in ('Function','Method')"):
    loc = json.loads(loc)
    f = os.path.relpath(os.path.realpath(loc["file"]), root)
    nodes[nid] = (f, loc["start_line"])
node_locs = set(nodes.values())

gs = {}
for a, b, prov in con.execute("select from_id, to_id, provenance from edges where kind like '%Call%'"):
    if a in nodes and b in nodes and nodes[a][0] in covered:
        p = json.loads(prov)
        gs[(nodes[a], nodes[b])] = (p.get("source"), p.get("via"))


def key(r):
    return ((r["caller_file"], r["caller_line"]), (r["callee_file"], r["callee_line"]))


truth = [r for r in truth if r["caller_file"] in covered and key(r)[0] != key(r)[1]]
cats = collections.OrderedDict(
    direct_calls={key(r) for r in truth if r["is_call"] and not r["via_iface"]},
    dispatch_calls={key(r) for r in truth
                    if r["is_call"] and r["via_iface"] and not r.get("std_trait")},
    # Out of GridSeak's scope by design: generic calls of standard-library
    # trait methods (`T: Clone` -> every workspace `impl Clone`).
    dispatch_std_traits={key(r) for r in truth
                         if r["is_call"] and r["via_iface"] and r.get("std_trait")},
    fn_value_refs={key(r) for r in truth if not r["is_call"]},
)
truth_all = set().union(*cats.values())
res = collections.OrderedDict()
res["oracle"] = dict(references=len(truth_all), covered_files=len(covered),
                     callers_without_gs_node=len({k[0] for k in truth_all} - node_locs),
                     callees_without_gs_node=len({k[1] for k in truth_all} - node_locs))
for name, keys in cats.items():
    mappable = {k for k in keys if k[0] in node_locs and k[1] in node_locs}
    hit = sum(1 for k in mappable if k in gs)
    hit_c = sum(1 for k in mappable if gs.get(k, (None,))[0] == "Compiler")
    res[name] = dict(edges=len(keys), with_both_nodes=len(mappable),
                     recall_any=round(hit / max(1, len(mappable)), 4),
                     recall_compiler=round(hit_c / max(1, len(mappable)), 4))

by_via = collections.defaultdict(list)
for k, (src, via) in gs.items():
    by_via[(src, via)].append(k)
prec = {}
for (src, via), ks in sorted(by_via.items(), key=lambda kv: str(kv[0])):
    fp = [k for k in ks if k not in truth_all]
    prec[f"{src}/{via}"] = dict(edges=len(ks), not_in_oracle=len(fp),
                                precision=round(1 - len(fp) / max(1, len(ks)), 4),
                                samples=[f"{a[0]}:{a[1]} -> {b[0]}:{b[1]}" for a, b in fp[:10]])
res["precision"] = prec

# Caller sets per callee: does GridSeak list every caller the compiler sees?
by_callee = collections.defaultdict(set)
for a, b in truth_all:
    by_callee[b].add(a)
gs_in = collections.defaultdict(set)
for (a, b) in gs:
    gs_in[b].add(a)
used = [c for c in by_callee if c in node_locs]
res["callers_per_symbol"] = dict(
    symbols_with_callers=len(used),
    has_some_caller=round(sum(1 for c in used if gs_in[c]) / max(1, len(used)), 4),
    all_callers_listed=round(sum(1 for c in used if by_callee[c] & node_locs <= gs_in[c])
                             / max(1, len(used)), 4))
missed = sorted(k for k in cats["direct_calls"] if k not in gs and k[0] in node_locs and k[1] in node_locs)
res["missed_direct_samples"] = [f"{a[0]}:{a[1]} -> {b[0]}:{b[1]}" for a, b in missed[:15]]
print(json.dumps(res, indent=1))
