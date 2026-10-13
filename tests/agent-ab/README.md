# Agent A/B task pack (TS + Go)

Twenty frozen tasks. Gold is **human-read source** (and `tsserver` where noted), not our SCIP index.

Corpora (gitignored clones):

| Corpus | Pin | Default graph scan_id |
|---|---|---|
| `.cache/g1/zod` | `e30870369d5b8f31ff4d0130d4439fd997deb523` | `4cb6e82a-e4d7-458f-aecb-09c3f8d87452` |
| `.cache/g1/chi` | `71307f9b7e4e9527638bc951c42b782cd1560331` | `73abec07-12fd-47e9-a2e8-fb3c69fdf971` |

```
make agent-ab
# → target/agent-ab.json
```

Control is a deterministic grep policy (no GridSeak). Treatment must query the scan artifact (CLI if present, else the same SQL as `verify_claim`). Do not gold files SCIP never indexed (`chi/_examples`, zod `deno/` as the *correct* callee).

This is an internal, rerunnable protocol — not a live Cursor/Claude bakeoff. If treatment loses false-delete or wrong-twin, `agent-ab` exits 1 and the clip is not allowed.

Evidence: `docs/workstreams/master-plan/evidence/11-AGENT_AB-results.md`.
