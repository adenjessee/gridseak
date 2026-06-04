# WS-GROUND-TRUTH — Labelled ground-truth datasets per language (Seven-axis, Axis 5)

> **Kind.** Design stub, not an implementation plan.
> **Parent strategy.** [`TRUST_AND_ACCURACY_MEMO.md §5 Axis 5`](../../00-strategy/TRUST_AND_ACCURACY_MEMO.md)
> **Status.** Unstaffed. Partial precedent exists in
> [`HAND_AUDIT_LOG.md`](../proof-foundation-gap/HAND_AUDIT_LOG.md)
> rounds A and 5b — those are labels, produced ad-hoc, not yet
> structured as a dataset.
> **Why this axis matters.** This is the one axis where a small
> investment produces the first ever single-number accuracy claim
> we can ship honestly. Every axis above this produces better
> evidence per edge; this axis produces the *measurement plane* that
> turns "the engine is pretty accurate" into *"precision 0.87 ± 0.03
> on visibility_private_unused on Rust, n=60 labels, as of 2026-Q3"*.

---

## 1. What this workstream owns

- A curated set of hand-labelled symbols per supported language,
  at `experiments/ground-truth/<lang>/<sample-name>/labels.json`.
- A CLI surface `gridseak benchmark <lang>` that runs the engine
  against each labelled corpus, matches verdicts to labels, and
  emits a precision / recall scorecard per `dead_code_reason`.
- A nightly CI job that publishes the scorecard and alarms on
  regression.

It does **not** own:

- the coverage collection of Axis 4 (those run independently);
- the heuristic-resolver source changes implied by a regression
  (those go to the relevant language workstream).

---

## 2. Label schema (v0 proposal)

```jsonc
{
  "corpus":        "apex-recipes@sha256:…",
  "corpus_pinned": "2026-05-01",
  "language":      "apex",
  "labels": [
    {
      "fqn":       "…::Foo::bar(Id)",
      "verdict":   "dead | live | unreachable",
      "reasoning": "free text, 1–3 sentences",
      "auditor":   "name",
      "audit_date":"2026-05-02",
      "evidence": {
        "runtime_coverage": null,       // hook for Axis 4 enrichment
        "caller_fqns":      [],
        "framework_invoker":"Iterator"
      }
    }
  ]
}
```

Labels are **append-only**; a mistake is corrected by adding a
superseding label with a later `audit_date`, never by editing the
old row. This preserves audit trail and enables "the precision
number on 2026-04 was 0.82; on 2026-06 it's 0.87 — what changed?"
diagnostics.

---

## 3. First-target label budget

| Language | Labels | Corpus | Shapes to cover |
| :--- | ---: | :--- | :--- |
| Apex | 50 | dreamhouse-lwc + apex-recipes + NPSP | fflib library, TDTM handler, VF controller, private helper, `@AuraEnabled`, test-only |
| Rust | 60 | gridseak-graphengine-self (we wrote it) | proc-macro-generated, `pub use` re-export, trait impl, private helper, `#[test]` |
| TypeScript | 50 | next.js example app + node library | default-exported, re-exported through barrel, decorator-invoked, JSX event handler |
| Python | 50 | Django tutorial app + pytest plugin | decorator-registered, `__getattr__` proxy, private helper, test fixture |
| Java | 40 | Spring PetClinic + library | `@Autowired` dependency, `@EventListener`, annotation-processor-generated, private helper |

Total v0 budget: **250 labels across five languages**. Assumes
~1 h per label for a senior engineer auditing their own familiar
codebase; ~2 h for unfamiliar codebases. Budget ≈ 1 engineer-month
of actual labelling plus infrastructure.

---

## 4. Scorecard output (v0)

```
Language: apex     Corpus: apex-recipes@… (50 labels)

  Reason                           | Precision | Recall  | n
  ---------------------------------+-----------+---------+----
  no_callers                       |   0.45    |  0.82   |  20
  visibility_private_unused        |   0.95    |  1.00   |  10
  framework_annotation_unresolved  |   0.65    |  N/A    |  10
  test_only_reference              |   1.00    |  0.90   |  10

  Overall (weighted by label count) | 0.70     |  N/A    |  50
```

`N/A` recall means we do not have enough labels of that shape to
compute it — distinct from "recall was measured and is low".

---

## 5. Relationship to the hand-audit log

The `HAND_AUDIT_LOG.md` rounds A / 5b are the *prototype* of this
workstream. They produced verdicts in markdown prose. This axis
normalises that output into a machine-readable dataset the engine
can self-measure against. The first deliverable of this workstream
is *"port the existing hand-audit verdicts into the new schema."*

---

## 6. Success criteria

- `gridseak benchmark apex` runs and emits the scorecard above.
- Every `dead_code_reason` emitted by the classifier has at least
  n ≥ 10 labels on at least one corpus per shipped language.
- The scorecard is published to the public marketing site on each
  stable release. **Measured precision becomes the customer-facing
  honesty claim.**
- Regression gate: a release that drops precision on any shape by
  >5 percentage points on any language fails CI.

---

## 7. Out of scope (v0)

- Labelling every `dead_code_reason` on every language. v0 covers
  the shapes we emit most often.
- Cross-language aggregate precision numbers. They are misleading
  because the easy languages dominate.
- Automated labelling via LLM / ML. v0 is human-labelled. If a
  future v2 proposes auto-labelling, it must ship its own precision
  verification on top.

---

## 8. Overlooked risks flagged per user rules

- **Labelling bias.** The engineer writing the label is often the
  same engineer tuning the classifier. This risks "teaching to the
  test". Mitigate by rotating labellers and by having a third-party
  auditor (possibly a contractor) adjudicate a blind sample every
  quarter.
- **Shifting corpus.** Labels pinned to a specific corpus SHA go
  stale when the upstream repo moves. Every label carries a
  `corpus_pinned` SHA, and the benchmark runner rejects a label if
  its corpus has shifted.
- **The dataset is the product.** This stub touches the point in
  `TRUST_AND_ACCURACY_MEMO.md §9` — in adjacent domains the
  benchmark dataset is often worth more than the algorithm. A
  future strategic decision may require gating dataset access
  behind licensing; design the artefact now to make that feasible
  later without breaking CI.
