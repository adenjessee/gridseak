# 05 — OBSERVATORY: public structural stats for the open-source world

Status: DRAFT v1 (2026-07-01)
Depends on: Gate G1 (accuracy) + plan 03 (distribution) MUST pass first.
Volume scales only behind Gate G4. Feeds plans 06 (gallery) and 07 (videos).
Executor profile: agent with shell/CI + basic static-site tooling.

## Context and strategic intent

Nobody publishes deterministic, longitudinal architecture-health data for
popular open-source repos (cycles, coupling, hotspots, dead code, edge
fidelity). A public observatory is: (a) a citable dataset — researchers,
bloggers, and LLMs ingest datasets and carry the tool's name with them;
(b) SEO/LLM-training surface area with zero founder audience required;
(c) GridSeak's own regression corpus (every engine release re-scans the
corpus and diffs); (d) the raw material for plan 07's video engine. The
owner's requirement: "a page that publicly shows any stat that a human or
agent would want."

HARD RULE: the observatory publishes nothing while the engine's measured
fidelity is below Gate G1 thresholds, and every published stat carries
its per-repo fidelity disclosure (from plan 01). Scale-only-if-accurate
is the constitution of this plan.

## Deliverable

1. **Scan fleet**: a batch pipeline (start: one machine + cron/CI; the
   engine is local-first so scale is horizontal and cheap):
   - Corpus manifest v1: 50 repos (top OSS by usage across TS, Python,
     Rust — bias toward languages with compiler-tier support; expand to
     500+ only after G4).
   - For each repo: clone at pinned weekly SHA → provision (npm install /
     venv where the SCIP indexer needs it, bounded time+disk budgets,
     sandboxed) → `gridseak scan` → extract report JSON + graph stats →
     append to the dataset store (one SQLite/parquet per release cycle).
   - Failures are first-class data: a repo whose indexer fails is
     published as "not scannable at tier X, reason enum", never silently
     dropped (same disclosure contract as plan 01).
2. **Public site** (static, generated, no backend to start):
   - Per-repo page: health score, cycles, coupling, hotspots, dead-code
     (with confidence caveats verbatim), edge counts by tier/confidence,
     trend charts over time, scan provenance (engine version, SHA,
     duration), and the raw JSON download link.
   - Index pages: leaderboards/distributions (most tangled, best layered,
     biggest hotspots), per-language fidelity averages — every number
     links to its raw artifact.
   - **Agent surface**: every page has a machine-readable twin
     (`/api/repo/<name>.json`, statically generated) + one `dataset.json`
     manifest; document it so MCP-less agents can consume it with plain
     HTTP. This is "any stat a human or agent would want" made literal.
3. **Methodology page**: how scans run, what tiers mean, known
   limitations, and a standing invitation for maintainers to correct or
   opt out (a friendly, non-shaming tone rule: findings are framed as
   structure, not ridicule — maintainers should feel measured, not
   attacked, or the observatory makes enemies instead of users).

## Steps

1. Build the batch runner as a small orchestrator script in-repo
   (`tools/observatory/`): manifest → jobs → artifacts. Reuse the
   existing benchmark harness patterns (`tools/benchmark/`).
2. Define the published-stats schema (versioned JSON) — review it once
   against plan 06/07 needs so report cards and videos read the same
   artifact rather than re-deriving numbers.
3. Static site generator: any minimal SSG or hand-rolled generator from
   the JSON; deploy on GitHub Pages first (no infra cost, no accounts
   beyond GitHub).
4. Dry-run on 10 repos; hand-review every page for wrong or embarrassing
   claims; fix engine or disclosure until a skeptical maintainer of each
   repo would say "fair".
5. Weekly automation via CI schedule; diff alerts (plan 09 consumes).
6. Expand corpus in steps (50 → 150 → 500) only while error rate and
   fidelity hold; each expansion is a release note.

## Acceptance criteria (binding)

- [ ] 50-repo corpus scanned end-to-end automatically; failure repos
      published with reason enums.
- [ ] Public site live with per-repo pages + JSON twins + dataset
      manifest + methodology page.
- [ ] Every published stat traceable to a raw artifact (scan id, engine
      version, repo SHA).
- [ ] No repo published below fidelity disclosure; spot-check 10 pages
      against local re-scans reproduces numbers exactly.
- [ ] Weekly refresh runs unattended for 3 consecutive weeks.

## Risks

- Provisioning arbitrary repos runs their build tooling → sandbox the
  fleet (container/VM, no credentials, network-restricted after clone).
- Maintainer backlash → tone rule above + opt-out honored within 48h.
- Cost creep → static site + local compute only until G4; no paid infra.
