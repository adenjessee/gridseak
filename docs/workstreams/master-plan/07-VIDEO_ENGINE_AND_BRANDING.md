# 07 — VIDEO ENGINE AND BRANDING: evidence that performs on camera

Status: DRAFT v1 (2026-07-01)
Depends on: Gate G1 + plan 05 (observatory data) + plan 06 (visual
artifacts). The flagship video also depends on plan 04 (benchmark
numbers). Executor profile: agent drafts everything; OWNER records the
flagship video; the per-repo pipeline is fully automated.

## Context and strategic intent

The owner can produce one high-quality YouTube video and wants an
AI-assisted pipeline that generates video reviews for scanned repos at
scale. Rules set by the owner: quality must be undeniable ("we can't
afford bad quality or nobody cares"), evidence must be hard, and the
video must serve both the personal brand and GridSeak. Strategy: ONE
flagship human video (the credibility anchor), then an automated
per-repo "structural review" video series fed by observatory data —
volume without founder hours, each episode carrying hard numbers.

## Part 1 — The flagship video (human, once, excellent)

Working title: "I measured how much AI coding agents actually know about
your codebase" (test 3–5 titles later; the content is the benchmark).

1. **Script skeleton** (agent drafts, owner rewrites in own voice):
   - Cold open: a live agent confidently giving a WRONG structural answer
     on a famous repo (reproduce a grep-baseline failure from plan 04).
   - The problem: agents guess structure; tokens are the tax; small
     architecture mistakes compound (30–60s, plain language).
   - The measurement: the truth benchmark — method in 60 seconds,
     on-screen tables from RESULTS.md, INCLUDING where GridSeak loses
     (the honesty beat is the branding).
   - The reveal: calibration — "the only tool that tells you when it's
     guessing" — with the ECE chart.
   - The observatory: 30-second flythrough of famous repos' report cards.
   - Close: one-command install, invitation to break the benchmark.
2. **Production notes**: screen captures rehearsed from a shot list;
   terminal food (report cards from plan 06) is the visual backbone; no
   claims that aren't in a published artifact; end-screen links to
   benchmark repo + observatory.
3. **Publication kit** (agent produces): description with artifact links,
   pinned comment with reproduction commands, timestamped chapters,
   thumbnail A/B variants, companion blog post (same content, for search
   and LLM ingestion), Show HN + dev.to + relevant subreddit drafts
   (each rewritten to that venue's norms, no cross-paste).

## Part 2 — The automated per-repo video pipeline

Deliverable: `tools/video-engine/` producing a 60–120s structural review
per observatory repo, fully generated, uploaded on a schedule.

1. **Script generation**: LLM prompt template that takes the repo's
   observatory JSON (plan 05 schema) and emits a tight narration script
   with a fixed structure: what the repo is (1 line), headline structure
   facts WITH tier citations, the one most interesting finding
   (hotspot/cycle/dead-code with confidence caveat verbatim), the trend
   if history exists, outro. Hard rule embedded in the prompt: every
   number must come from the JSON; the generator refuses findings whose
   confidence caveat is low unless the caveat is spoken aloud.
2. **Visuals**: deterministic renderer — animated report card + graph
   stats (template video composition, e.g. Remotion or a headless
   renderer over the plan-06 HTML) — NOT generative video for data
   (generative b-roll allowed for intros only). Charts are rendered from
   the same JSON.
3. **Voice**: TTS with one consistent voice (licensed for YouTube);
   disclose AI narration in the description (trust brand ≙ disclosure).
4. **QA gate**: every generated video gets an automated fact-check pass
   (script numbers re-matched against JSON) + human (owner) approval for
   the first 20 episodes; after error rate proves < 1 material error /
   20 episodes, sample-audit 1 in 5.
5. **Channel strategy**: separate playlist/series branding under the
   owner's channel (feeds the personal brand) with GridSeak watermark;
   per-video descriptions link the repo's observatory page (closing the
   loop: video → page → install).

## Steps

1. Wait for gates; do NOT produce public video from pre-G1 data.
2. Build Part 2's script generator + renderer against 3 sample repos;
   iterate privately until owner judges quality "undeniable".
3. Flagship video production (owner) once plan 04 publishes.
4. Launch order: benchmark post → flagship video → weekly automated
   episodes (consistency beats bursts for channel growth).
5. Wire analytics into plan 09 (views → observatory visits → installs).

## Acceptance criteria (binding)

- [ ] Flagship video published with every on-screen number traceable to
      a public artifact; companion post live.
- [ ] Pipeline produces an episode from observatory JSON with zero manual
      editing; fact-check pass green; 3 episodes approved by owner.
- [ ] Each episode's description links its observatory page and the
      install command.
- [ ] AI narration disclosed on every generated episode.

## Risks

- Quality bar: if generated episodes aren't good, publish NONE and keep
  the flagship + occasional human videos; a bad series damages the brand
  asymmetrically.
- Platform copyright/monetization quirks with TTS: use licensed voices;
  keep episodes factual commentary (fair-use-friendly, but repos' code is
  shown from public sources with attribution).
