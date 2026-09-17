# 06 — ARTIFACTS AND BADGES: every scan is an advertisement

Status: DRAFT v1 (2026-07-01)
Depends on: Gate G1 (never beautify numbers we can't defend). Parallel
with 05. Executor profile: Rust agent + light design sense; templates
make design skill optional.

## Context and strategic intent

Codecov spread through READMEs via badges; every adopter becomes
distribution with zero marginal founder effort. GridSeak's scan already
produces a rich report JSON; what's missing is (a) a beautiful,
shareable, honest rendering and (b) a badge/report-card loop that
travels. Everything here renders EXISTING measured data — this plan adds
zero new analysis and must never invent numbers.

## Deliverables

1. **Terminal report card** (`gridseak scan` final output):
   - One screen: health score, top 3 risks (from existing
     recommendations), edge fidelity by tier (from plan 01 disclosure),
     cycles count, scan provenance line (engine version, duration,
     scan id). Honest by construction: the fidelity line always shows,
     including "python: heuristic only (ServerMissing)".
   - Implementation: a renderer module in `gridseak-cli/src/render/`
     (siblings exist there already); snapshot-test the output.
2. **Markdown/HTML report card** (`gridseak report --format md|html`):
   - Same content, styled for pasting into PRs/issues/READMEs; the HTML
     is a single self-contained file (inline CSS, no JS required) so it
     attaches anywhere.
3. **README badge**:
   - Static SVG generator: `gridseak badge` writes
     `.gridseak/badge.svg` (score + tier honesty mark) that repos commit,
     plus a shields.io-compatible JSON endpoint emitted into the
     observatory site for repos it tracks (no server needed — static).
   - Badge links to the repo's observatory page when it exists, else to
     the project README.
4. **Public gallery** (part of the observatory site, plan 05): a
   browsable, screenshot-friendly gallery of famous-repo report cards —
   the "wow" surface for links, articles, and plan 07 videos.

## Steps

1. Design the report-card information hierarchy ONCE in a doc page with
   two mock outputs (healthy repo / messy repo); review against the
   tone rule (structure, not shame) and the honesty rule (fidelity
   always visible).
2. Implement terminal renderer + snapshot tests.
3. Implement md/html renderer sharing the same data-selection layer
   (one module decides WHAT to show; renderers decide HOW — keep these
   separate so a future web UI reuses the selection).
4. Implement badge generator (SVG template + score→color mapping;
   colorblind-safe palette).
5. Add `gridseak report`/`gridseak badge` to README and `gridseak setup`
   epilogue ("add the badge to your README").
6. Gallery pages generated from observatory artifacts (plan 05 schema).

## Acceptance criteria (binding)

- [ ] `gridseak scan` ends with the report card; snapshot tests pin it.
- [ ] `gridseak report --format html` produces a single-file report that
      renders correctly in a browser with no network.
- [ ] `gridseak badge` produces a valid SVG; badge renders on GitHub.
- [ ] Fidelity/tier disclosure appears on EVERY artifact (terminal, md,
      html, badge tooltip/link target) — verified by tests.
- [ ] Gallery live for the observatory's first 10 repos.

## Risks

- Prettiness pressure vs honesty: the fidelity line is non-negotiable
  even when it's unflattering; that IS the brand.
- Badge staleness: badge embeds scan date; observatory-tracked repos get
  weekly refresh; local badges say "as of <date>".
