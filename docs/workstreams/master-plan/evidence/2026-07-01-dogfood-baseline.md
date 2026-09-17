# Evidence baseline — MCP dogfood of GridSeak on itself (2026-07-01)

Recorded so plan executors verify against the same starting truth.
Method: GridSeak MCP tools called from a Cursor agent session against
this workspace. No numbers below are estimated; all come from tool
responses or the report JSON on disk.

## Fresh self-scan (the F1 evidence)

- Tool: `gridseak_scan` with `path: /Users/me/gridseak-graphengine`.
- Scan id: `7d3f8035-e63e-438b-b440-0314570b22d8`
- Completed: 2026-07-02T03:02:26Z, duration 85,633 ms, status ready.
- Branch `lsp-reliability-foundation`, commit `ce724323`, git_dirty true.
- Primary language: rust; scan_languages: rust, apex, typescript,
  python, javascript.
- Headline metrics: health 61.0, nodes 9,281, edges 37,427, functions
  5,677, modules 82, cycles 412, dead_code 87, findings 119.
- Report path:
  `~/Library/Application Support/com.gridseak.desktop/project-reports/7d3f8035-e63e-438b-b440-0314570b22d8.report.json`

### resolution_quality (verbatim)

```json
{
  "import_edges_total": 1192,
  "resolution_tier": "full",
  "measured_fidelity": {
    "tier": "syntactic_only",
    "high_ratio_on_calls": 0.14337299023147973,
    "call_edges_by_confidence": { "high": 3772, "medium": 97, "low": 22440, "unknown": 0 },
    "all_edges_by_confidence": { "high": 13515, "medium": 1457, "low": 22455, "unknown": 0 }
  }
}
```

Interpretation: on the flagship Tier-3 language (Rust), the standard MCP
scan path produced `syntactic_only` fidelity with 14.3% high-confidence
call edges. The rust-analyzer Layer-2 adapter either did not run or
barely contributed.

### Missing telemetry

The report JSON top level contains NO keys matching lsp/telemetry/session
(checked programmatically). The semantic-tier skip is therefore silent —
no fallback-reason histogram, no session metrics, no disclosure. This is
the primary defect plan 01 exists to fix.

## Wrong-project default routing (the F2 evidence)

- Tool: `gridseak_context_for_llm` with `project: "."` (schema default),
  called from a Cursor session whose workspace is
  `/Users/me/gridseak-graphengine`.
- Response summary block: `project: "pg-meta"`, scan id
  `73dc9c36-84b9-4cf0-9f69-172ad14814db`, languages `[typescript]`, and
  `incremental_plan.changed_paths_parse` listing ~130 files under
  `/Users/me/Desktop/supabase/packages/pg-meta/` — an unrelated
  repository. `workspace_delta.since_scan_seconds` = 1,627,394 (~19 days).
- No error or warning indicated the mismatch. An agent trusting this
  response would answer structural questions about the wrong codebase.

## Working notes

- The `gridseak_scan` response itself was well-formed: correct project,
  scan provenance envelope (`scan_age_seconds: 0`), tier legend present.
  The engine's local store and MCP surface work; the defects are
  (1) semantic-tier absence + silence, (2) default-project resolution.
- tier_legend currently names tier_0 / tier_1 / tier_3 only; when the
  Compiler tier ships (plan 02), the legend vocabulary needs a
  coordinated update (scheduled in plan 04 step 6).
