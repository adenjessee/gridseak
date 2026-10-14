#!/usr/bin/env python3
"""Check `gridseak audit --plan-only` answer keys against the MIR oracle.

Usage: check_audit_keys.py TRUTH.json COVERED_FILES.json TASKS.json

Each task asks for every transitive caller of a target through direct
calls (no trait dispatch, no function values; closures credited to their
enclosing function). The oracle's direct-call edges give the same closure;
a key is exact when both sets match. Callers in files the oracle did not
compile are reported separately, not as errors.
"""
import collections
import json
import sys

truth, covered, tasks = (json.load(open(p)) for p in sys.argv[1:4])
covered = set(covered)
callers = collections.defaultdict(set)
names = {}
for r in truth:
    if r["is_call"] and not r["via_iface"]:
        a = (r["caller_file"], r["caller_line"])
        b = (r["callee_file"], r["callee_line"])
        if a != b:
            callers[b].add(a)
            names[a] = r["caller_name"]

summary = []
for t in tasks:
    target = (t["file"], t["line"])
    seen, frontier = set(), [target]
    while frontier:
        nxt = []
        for f in frontier:
            for c in callers.get(f, ()):
                if c not in seen and c != target:
                    seen.add(c)
                    nxt.append(c)
        frontier = nxt
    key = {(a["file"], a["line"]) for a in t["answer"]}
    uncovered = {k for k in key if k[0] not in covered}
    missing = sorted(seen - key)
    extra = sorted((key - seen) - uncovered)
    exact = not missing and not extra
    summary.append(exact)
    print(f"{t['id']} {t['band']:6} {t['target']}: key={len(key)} oracle={len(seen)} "
          f"{'EXACT' if exact else 'DIFF'} missing={len(missing)} extra={len(extra)} "
          f"uncovered_by_oracle={len(uncovered)}")
    for k in missing[:8]:
        print(f"    missing {k[0]}:{k[1]} {names.get(k, '')}")
    for k in extra[:8]:
        print(f"    extra   {k[0]}:{k[1]}")
print(f"exact keys: {sum(summary)}/{len(summary)}")
