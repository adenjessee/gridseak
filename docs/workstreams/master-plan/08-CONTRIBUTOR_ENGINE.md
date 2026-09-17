# 08 — CONTRIBUTOR ENGINE: turn strangers with expertise into owners

Status: DRAFT v1 (2026-07-01)
Depends on: 02 (SemanticIndex port proves the adapter shape), 03 (public
repo + CI), 04 (benchmark gives contributors a score to move).
Executor profile: agent for scaffolding/docs; OWNER for review culture.

## Context and strategic intent

The owner has no network; the people who will care most are engineers who
discover the project and find a crisp, well-bounded way to contribute.
The single best contribution shape this codebase can offer is
**"adopt a language"**: implement a `SemanticIndex` adapter (SCIP-based,
per plan 02 Phase C) for a language you care about, and watch your
language's benchmark scores rise on a public leaderboard. Secondary
shapes: benchmark adapters for competitor tools, observatory corpus
suggestions, and finding-quality reports.

## Deliverables

1. **The adapter contract, documented for outsiders**:
   `docs/contributing/ADD_A_LANGUAGE.md` — a complete walkthrough that a
   competent stranger finishes in a weekend:
   - The `SemanticIndex` trait, `IndexTarget`, disclosure/SkipReason
     contract, provenance stamping rules (Compiler tier), where the
     factory registers adapters.
   - A worked example: the TypeScript SCIP adapter, file by file.
   - The definition of done: fixture repo + prebuilt index committed,
     integration test green, benchmark harness run showing the
     language's before/after, LIMITATIONS.md row updated honestly.
   - Candidate menu with pointers: scip-java (Java/Scala/Kotlin),
     scip-dotnet (C#), scip-ruby, scip-clang (C/C++), scip-dart, scip-php
     — each with its indexer's install command and known quirks.
2. **Issue garden**: 10–15 genuinely good first issues, each
   self-contained with context links, acceptance criteria, and a
   "mentorship contract" (owner responds within 48h). Labels:
   `adopt-a-language`, `benchmark-adapter`, `good-first-issue`.
3. **Review SLA + templates**: PR template (checklist mirrors the
   definition of done), a CONTRIBUTING.md that states response times and
   the honesty norms (no claims without artifacts — contributors absorb
   the culture from the docs).
4. **Launch posts** (agent-drafted, owner-approved): Show HN, dev.to,
   r/rust + r/LocalLLaMA + language-specific communities when that
   language's adapter ships ("Rust tool X now has compiler-grade Ruby
   support, here's the measured before/after" is a per-language news
   moment — each adapter shipping is a distribution event).
5. **Recognition loop**: adapter authors listed on the observatory's
   methodology page + README credits + benchmark result pages ("Ruby
   adapter by @…"). Public credit is the compensation; make it generous
   and automatic.

## Steps

1. Extract and freeze the adapter-facing API surface after plan 02 lands
   (one release where the trait is stable; semver discipline from then on).
2. Write ADD_A_LANGUAGE.md against the real TS implementation; have an
   agent follow it cold to build a second adapter (Python) and fix every
   place the doc was insufficient — the doc is done when the agent
   succeeds without asking questions.
3. Plant the issue garden; open the repo to issues/discussions.
4. Ship launch posts in sequence AFTER benchmark + observatory exist
   (strangers need something to see within two clicks).
5. Owner practice: 30 min/day triage window; agents draft first-pass PR
   reviews and issue responses, owner approves/edits — written,
   asynchronous, no charisma required.

## Acceptance criteria (binding)

- [ ] A cold agent (or stranger) builds a working Python adapter using
      only ADD_A_LANGUAGE.md.
- [ ] >= 10 curated issues live with the labels and templates.
- [ ] First external PR merged (any size) with the review SLA met.
- [ ] Each shipped adapter gets its measured before/after published and
      its language-community launch post.

## Risks

- Contributor quality variance → the definition-of-done checklist and
  benchmark gate are the filter; no adapter merges below measured
  thresholds (document the thresholds in the doc).
- Maintainer burnout → the SLA is 48h responses, not 48h merges; agents
  do first-pass reviews; the issue garden is capped at what one person
  can shepherd.
