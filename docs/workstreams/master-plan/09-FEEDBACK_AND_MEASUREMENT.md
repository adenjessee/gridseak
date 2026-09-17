# 09 — FEEDBACK AND MEASUREMENT: know if it's working, forever

Status: DRAFT v1 (2026-07-01)
Depends on: starts with plan 03 (first public surfaces) and never ends.
Executor profile: agent-automatable almost entirely; owner reviews a
weekly digest.

## Context and strategic intent

The owner's requirement: "a way to know if it's working, have AI measure
it and run on its own over time, and improve from any feedback the moment
it comes in from credible sources." This plan is the nervous system: it
defines the metrics, the collection (privacy-respecting), the credibility
weighting, and the automated loop that turns signals into triaged work.

## Part 1 — What we measure (the funnel, instrumented honestly)

Stage → metric → source:
1. **Findability**: registry listing status + directory scores (Glama
   score number), search ranking for 5 fixed queries — collected weekly
   by an agent, no accounts needed.
2. **Acquisition**: GitHub stars/clones/traffic (repo API), release
   download counts per artifact/platform (proves the ARM fix matters),
   npm/Homebrew install counts when live.
3. **Activation**: OPT-IN, anonymous telemetry ping on scan completion
   (engine version, OS/arch, language mix, fidelity tier reached,
   duration bucket — NO paths, NO code, NO identifiers). Default off;
   one-line prompt at first run; document the exact payload in
   PRIVACY.md. If opt-in rates make data useless, fall back to release
   download counts + registry stats and say so — never dark-pattern it.
4. **Engagement**: observatory page visits (static-site analytics,
   privacy-lite e.g. GoatCounter), benchmark repo stars/forks, MCP
   directory install clicks where exposed.
5. **Advocacy**: unsolicited mentions — weekly agent search sweep (HN,
   Reddit, dev.to, X, YouTube, newsletters) for "gridseak" and category
   comparison articles; log every mention with sentiment + reach.
6. **Contribution**: external issues/PRs opened+merged (plan 08's SLA
   adherence measured here too).

Anti-metrics (tracked to keep us honest): any public claim without a
reproducible artifact (target: zero), fidelity regressions on the
observatory corpus between releases (target: zero unexplained).

## Part 2 — Credibility-weighted feedback triage

Every inbound item (issue, comment, thread reply, email) gets scored by
an agent on evidence, not status:
- **High credibility**: includes a reproduction, a scan id, a benchmark
  run, or a concrete file/line claim; OR author demonstrably maintains
  relevant software (their profile shows it). → triaged within 24h into
  a labeled issue with the evidence attached; owner sees it in the daily
  window.
- **Medium**: specific but unreproduced claims. → agent attempts
  reproduction first; promotes or archives with the attempt logged.
- **Low**: vibes, drive-bys, praise/insults without content. → logged
  for sentiment trend only.
The rule the owner asked for, made mechanical: credibility = evidence
attached, reproducibility, and demonstrated domain history — never
follower counts.

## Part 3 — The autonomous loop

A scheduled agent job (weekly, plus a light daily sweep) that:
1. Collects Part-1 metrics into `metrics/weekly/<date>.json` in a
   dedicated repo (append-only, diffable history).
2. Runs the mention sweep + triage queue (Part 2).
3. Re-runs the benchmark CI slice and observatory diff; flags fidelity
   regressions as P0 issues automatically.
4. Produces a one-page weekly digest for the owner: funnel deltas, top 3
   credible feedback items with drafted responses, one recommended
   action for the week (drafted, not auto-executed).
5. Escalation rules: fidelity regression, security report, or a
   high-reach negative mention → immediate notification, not weekly.

Implementation: a `tools/pulse/` script suite runnable by any scheduled
agent (Cursor cloud agents / GitHub Actions cron); all state in git; no
databases, no dashboards to maintain — markdown + JSON digests.

## Part 4 — Decision cadence

- Weekly: owner reads digest, picks/edits the recommended action (15 min).
- Monthly: gate review against 00-MASTER_PLAN gates; kill/scale calls
  (e.g. observatory corpus expansion, video series continuation) are
  made HERE with the funnel data, not on gut feel.
- Quarterly: re-run the competitive sweep (plans 04's landscape section)
  and re-validate that the category hasn't shifted under us (frontier
  agents absorbing capabilities is the standing existential risk —
  watch for first-party graph features in Cursor/Claude/Copilot
  releases specifically).

## Acceptance criteria (binding)

- [ ] PRIVACY.md + opt-in telemetry shipped (or documented fallback).
- [ ] `tools/pulse/` produces the weekly JSON + digest unattended for 4
      consecutive weeks.
- [ ] Triage rubric applied to 100% of inbound items in those 4 weeks;
      every high-credibility item answered within 24h.
- [ ] First monthly gate review held with the digest as the input.

## Risks

- Telemetry backlash → opt-in only, payload public, easy off-switch;
  the trust brand extends to how we measure.
- Metric theater → the anti-metrics section exists so we notice when
  numbers move without reality moving; the monthly review must ask
  "which of these would we bet the roadmap on?"
