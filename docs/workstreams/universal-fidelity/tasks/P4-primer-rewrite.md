# P4 — Primer surgical rewrite

> Authored against [TEMPLATE.md](TEMPLATE.md).

## 1. Problem statement

The current [NEW_ENGINEER_PRIMER.md](../NEW_ENGINEER_PRIMER.md) §8 frames T1 as a series of six design "decisions" presented as A-vs-B trade-offs:

- Decision 1: `EdgeKind: Copy` vs forward-compat `Unknown(String)`.
- Decision 2: `FrameworkKind` unit variants vs data-carrying.
- Decision 3: extract-time vs resolve-time emission (with `CallSite.edge_kind_hint`).
- Decision 4: `{:?}` vs hand-rolled `to_stable_str`.
- Decision 5: which metrics include `Framework` / `Declarative` variants.
- Decision 6: ship all planned variants vs only variants with emitters.

Senior review found five of six are false trade-offs hiding a missing type, predicate, or channel. The A/B framing teaches future engineers the wrong lesson: that every design question is a forced choice between imperfect alternatives. The correct shape for four of those six is not "pick a loser" but "name the third element that dissolves the trade-off".

Concrete symptom: engineers reading the current primer will re-litigate these trade-offs whenever they touch the code and may "pick the other option next time" on a false premise — exactly the regression mode the rework is trying to prevent. The primer is instructional; a wrongly-framed decision is reproduced forever.

## 2. Non-goals

- **Not rewriting T2, T5, T6, T7, T8 sections.** Only §8 is restructured. Other sections are factually correct; they get minor link additions to point at the diagnostic-questions section.
- **Not deleting the historical text.** The original framing remains, preserved as an "earlier framing" prelude inside each rewritten subsection. Commit archaeology pointing at "Decision 3" still lands on content about Decision 3.
- **Not proposing code changes.** Code changes are owned by [T1-rework.md](T1-rework.md) (P1). This doc only ships documentation edits.
- **Not authoring `EDGE_TAXONOMY.md` here.** The artifact itself lives under `docs/04-architecture/`; this task creates it, but its content (planned variants with emitter / tests / ships columns) is the artifact, not the task plan.

## 3. Five diagnostic questions

1. **Are we forcing one type to do two jobs?** Partial — the primer's §8 was forcing one prose section to do two jobs (decisions-to-relitigate AND architectural principles). Dissolving element: split §8 into §5.5 ("the two-jobs rule", the principle) and a restructured §8 (the applied consequences), with a new §8.0 for the diagnostic questions themselves.
2. **Is the trade-off between "hardcoded list" and "brittle update"?** No. The existing decision list is what it is; the task is framing, not enumeration.
3. **Is the trade-off between "earlier stage knows X" and "later stage knows Y"?** No — this task is documentation-only.
4. **Is the trade-off about serialization format?** No — the content, not its serialization.
5. **Is the trade-off between two modes of failure?** Yes — "preserve history" vs "stop teaching the wrong lesson". Dissolving element: the "earlier framing" preamble per subsection. Preserve the text verbatim under a clearly-marked heading, then follow with the reframed analysis. Both properties satisfied.

## 4. Chosen shape

Three additions, four rewrites, one companion artifact.

### 4.1 Additions

- **§5.5 "The two-jobs rule".** New subsection under §5 (Layered Fidelity Architecture). States: when a type / prose-section / metric / code path is fighting you, it is probably carrying two responsibilities. Split before you compromise. Cites four live examples from the sprint itself (listed in §4.3).
- **§8.0 "Diagnostic questions before accepting a trade-off".** New subsection opening §8. Lists the five reviewer questions, each with a one-sentence rationale. Framed as a discipline (a reflex run when seeing A/B framing), not a checklist to fill out in every design doc.
- **§8.7 "Further reading" pointer** at end of §8 pointing readers at [docs/workstreams/universal-fidelity/tasks/TEMPLATE.md](TEMPLATE.md) so every task using the diagnostic-questions discipline is one link away.

### 4.2 Rewrites

All four rewritten subsections follow the same structural template:

```
### Decision N — <original title>

#### Earlier framing (preserved)
<original A/B text, unedited>

#### Root cause
<one paragraph naming the missing element>

#### Dissolving shape
<code block or sketch of the corrected design>

#### Why this is not actually a choice
<one paragraph explaining the third option evaporates the trade-off>
```

Subsections affected:

- **Decision 1** — `EdgeKind: Copy` vs `Unknown(String)`. Root cause: one type doing two jobs (in-memory domain vs on-disk wire). Dissolving element: `PersistedEdgeKind { Known(EdgeKind), Unknown(String) }` at the SQLite boundary, `EdgeKind` stays `Copy`.
- **Decision 3** — extract-time vs resolve-time emission. Root cause: missing typed channel between the extractor (which knows the framework context) and the resolver (which knows the target node ID). Dissolving element: `UnresolvedReference { Call, FrameworkBinding, DeclarativeBinding }` enum. Resolver matches on variant; compiler enforces exhaustiveness.
- **Decision 4** — `{:?}` vs hand-rolled `to_stable_str`. Root cause: both options ignore the serialization crate every Rust project already uses. Dissolving element: `#[serde(tag = "kind", content = "sub")]` on `EdgeKind`. Free round-trip, free forward-compat via `PersistedEdgeKind::Unknown(String)` composing with `#[serde(untagged)]`.
- **Decision 5** — which metrics include `Framework` / `Declarative`. Root cause: hardcoded filter lists instead of declared predicates. Dissolving element: `is_structural()`, `is_call_like()`, `is_dependency()`, `is_inheritance()` on `EdgeKind`. Metrics consume predicates; variants declare membership. Adding a variant is a one-file edit.

### 4.3 Reframed subsections (kept but shortened)

- **Decision 2** — unit vs data-carrying. Reframed as one paragraph: "taxonomy and metadata are two axes, each with one correct answer. §5.5 dissolves the trade-off."
- **Decision 6** — ship all vs ship none. Reframed as one paragraph: "code holds what runs; markdown holds what's planned." Links to `docs/04-architecture/EDGE_TAXONOMY.md` (companion artifact).
- **Decision 7** — test list. Unchanged; it was already a task list, not a decision.

### 4.4 Companion artifact — `docs/04-architecture/EDGE_TAXONOMY.md`

New markdown under `docs/04-architecture/` (NOT under `docs/workstreams/`, because the taxonomy outlives any single sprint). Columns: `variant | family | emitter exists? | tests exist? | ships in code?`. Populated with:

- Shipped today: `Framework::VisualforcePage`, `Declarative::Flow` (placeholder, no emitter).
- Planned: `FrameworkKind::{LwcTemplate, AuraComponent, Trigger, InboundEmail}`, `DeclarativeKind::{ProcessBuilder, WorkflowRule}`, plus Spring XML / Django URLconf slots as generic Declarative analogues.

Primer §8 Decision 6 links to this file as "the home for planned-but-unemitted variants". The invariant is: a variant sits in this file until an emitter ships in the same PR as the variant's addition to `EdgeKind`.

## 5. Acceptance criteria

A new engineer reading the primer front-to-back can, without consulting the source:

1. Name "the two-jobs rule" and give one example from the sprint.
2. Recite the five diagnostic questions.
3. Explain why `CallSite.edge_kind_hint: Option<EdgeKind>` is worse than `UnresolvedReference { Call, FrameworkBinding, DeclarativeBinding }` in two sentences.
4. Find the home for a planned-but-unemitted variant (`docs/04-architecture/EDGE_TAXONOMY.md`) within 30 seconds of asking "where does `LwcTemplate` go before it has an emitter?".
5. Reproduce Decision 5's predicate list (`is_structural`, `is_call_like`, `is_dependency`, `is_inheritance`) and explain what metric filters on each.

No test automation here; acceptance is reviewer judgment against these five questions.

## 6. Test plan

- **Markdown lint.** `markdownlint docs/workstreams/universal-fidelity/NEW_ENGINEER_PRIMER.md` and the new `docs/04-architecture/EDGE_TAXONOMY.md` must parse without warnings.
- **Link check.** `lychee --verbose docs/workstreams/universal-fidelity/NEW_ENGINEER_PRIMER.md` — every relative link resolves.
- **Self-test via a team read-aloud.** Not automatable. One engineer unfamiliar with the §8 rewrite reads it cold and answers the five acceptance questions. If they cannot, the section is rewritten until they can. This is the only meaningful test for documentation; we list it explicitly so the reviewer knows the cost.

## 7. Rollback criterion

If the primer rewrite is judged harder to read than the current version by two independent engineers (one who read the old version, one who has not), revert and re-author. This is the only rollback signal that matters for a documentation change; no code is in flight.

## 8. Out-of-scope follow-ups

- **Mermaid diagram of the `EdgeKind` family hierarchy.** Would replace the textual taxonomy in §3 with a visual. Deferred; not blocking the diagnostic-questions discipline install.
- **Cross-link the taxonomy from `docs/deferred/aspirational-architecture/graph-system/*.md`.** The edge-related architecture docs there predate T1 and are deferred; updating them to point at `EDGE_TAXONOMY.md` is a doc-hygiene follow-up.
- **SPRINT_PLAN.md update to reference §5.5 and §8.0.** The plan explicitly says "does NOT edit SPRINT_PLAN.md"; this follow-up lives outside the plan's scope.
