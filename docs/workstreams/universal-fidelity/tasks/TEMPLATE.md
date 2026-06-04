# Task design template (universal-fidelity sprint and beyond)

> **How to use this template.** Copy this file to `docs/workstreams/universal-fidelity/tasks/<task-id>.md` and fill in every section. Do not skip sections — if a section does not apply, write "N/A — <one-line rationale>" rather than deleting it. The skeleton exists to force authors to answer the diagnostic questions **before** writing code, not after.
>
> This template is the institutional-memory outcome of the T1 review. Five of six T1 design "decisions" turned out to be false trade-offs hiding a missing predicate, channel, or boundary type. Every T1-class failure is a failure to pause and ask whether the A-vs-B framing is itself the bug. The §3 diagnostic questions exist so that pause happens on every task, not just when a senior reviewer is in the loop.

---

## 1. Problem statement

Describe what fails today. Be concrete:

- What code path, user-visible behaviour, test output, or measurement is broken or absent?
- Cite the file(s) and line range(s) where the symptom lives.
- If the problem is "we cannot measure X", say so and link the measurement gap to the reason it matters (e.g., which downstream decision is blocked).

Avoid vague framings like "improve fidelity" or "clean up extraction". Name the symptom.

## 2. Non-goals

List what this task explicitly will NOT fix. Two purposes:

1. Prevent scope bleed during implementation.
2. Point the reader at the companion task that owns each excluded concern.

Format as bullets, one per non-goal, each naming the companion ticket or design doc that owns it (or "deferred, no ticket yet" if truly out-of-scope for this sprint).

## 3. Five diagnostic questions

Answer every question **before** writing code. For each, mark `Yes` or `No` and give a one-sentence rationale. If any answer is `Yes`, §4 must include the dissolving element for that axis — not a chosen loser.

1. **Are we forcing one type to do two jobs?**
   Rationale:
2. **Is the trade-off between "hardcoded list" and "brittle update"?**
   Rationale:
3. **Is the trade-off between "earlier stage knows X" and "later stage knows Y"?**
   Rationale:
4. **Is the trade-off about serialization format?**
   Rationale:
5. **Is the trade-off between two modes of failure?**
   Rationale:

> **Reading guide.** If every answer is `No`, you likely have a real design choice and should explain the chosen trade-off in §4. If any answer is `Yes`, the standard dissolving moves are: (1) split the type into domain vs wire vs metadata, (2) replace variant lists with predicates, (3) introduce a typed channel (enum) between stages, (4) use `serde` with explicit tags, (5) introduce an explicit error type or `Result` boundary. Reference `docs/workstreams/universal-fidelity/NEW_ENGINEER_PRIMER.md` §5.5 "the two-jobs rule" and §8.0 "diagnostic questions".

## 4. Chosen shape

Describe the design the diagnostic questions land on. If any §3 question answered `Yes`, state the dissolving element and cite which answer it dissolves.

Cover:

- **Types introduced or changed.** Signatures, key fields, invariants.
- **Data flow.** Which stage produces what, which stage consumes it. A mermaid diagram is welcome if the flow is non-obvious.
- **Compile-time guarantees.** What the type system will enforce that the current shape does not.
- **Predicate contracts.** If this task touches `EdgeKind` or any taxonomy enum, list the predicates it adds or updates. Metric consumers should read predicates, not variant lists.

## 5. Acceptance criteria

Name the behavioural invariants a reviewer can grep for. Each criterion must be:

- **Behavioural** (asserts invariant, not implementation shape).
- **Grep-able** (names the test file or the log string a reader would look for).
- **Falsifiable** (someone could construct a failing input and see the test fail).

Bad: "the code is cleaner". Good: "`cargo test --test t4_canary_tier_classification` passes six named fixtures including the 40%/80% boundary quartet".

## 6. Test plan

Every code-producing task ships tests in three tiers:

1. **Unit tests** for the predicate, enum, or conversion layer. Every new predicate gets a named unit test asserting its variant membership. Every new enum variant gets a round-trip test.
2. **Integration tests** against synthesized fixtures (in-memory `AnalysisGraph`, synthesized `parse.db`, etc.) with named behavioural assertions.
3. **Regression fixture** (canary-style) with a pinned expected-output snapshot. Snapshots are updated only when an intentional behavioural change ships, and the PR description must call out the snapshot delta.

List every test file name and the specific assertion it owns. Do not hand-wave "we will add tests" — name them.

## 7. Rollback criterion

State the single specific test failure or measurement threshold that forces a revert. Not "if anything breaks" — a named test or a named metric threshold.

This is deliberately narrow. A rollback criterion exists so the on-call engineer at 2 AM knows when to pull the plug without re-reading the design doc. If the criterion is broad, rewrite it until it is a single named signal.

## 8. Out-of-scope follow-ups

Work that surfaces during implementation but does not block this task. Each entry:

- Names the concern.
- Links or proposes the companion ticket / design doc that will own it.
- States why it is not required for this task's acceptance.

This section is where discipline lives: anything the implementer is tempted to fix "while I'm in there" goes here as a named follow-up, not silently merged into the current PR.

---

## Optional: "shipped, see rework ticket" retrospective entry

For tasks that are being re-planned after code already shipped (e.g., T1 rework), include a top-of-doc retrospective section naming:

- What shipped and when.
- What the post-ship review found missing or wrong.
- The rework ticket that owns the correction.

This preserves the audit trail without pretending the earlier work did not exist.
