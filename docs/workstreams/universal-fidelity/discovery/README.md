# Universal-fidelity — post-sprint discovery stubs

> **Purpose.** Home for focused investigation briefs that surfaced
> from the sprint but do **not** fit the `tasks/*.md` shape (those
> are sized design docs for scheduled implementation work).
> Discovery stubs are smaller, scoped to "measure or prove X before
> we know whether it is worth ticketing".
>
> Each stub in this directory traces back to a row in
> [`../FOLLOWUPS.md`](../FOLLOWUPS.md). The stub carries the
> investigation plan; the FOLLOWUPS row carries the recommended
> next step. Graduation criterion is explicit: when a stub produces
> evidence that warrants a scheduled PR, promote it to
> `../tasks/<id>.md` and update the FOLLOWUPS row's
> `Recommended next step` to point at the new design doc.

---

## Index

| Stub | Source follow-up | Question the stub answers |
| :--- | :--- | :--- |
| [`rust-layer2-symbol-index-loss.md`](rust-layer2-symbol-index-loss.md) | [UF-FU-012 (a)](../FOLLOWUPS.md) | Why does the `SymbolIndex` caller/callee lookup drop 36 % of successful adapter resolutions on `gridseak-self`? What per-shape (impl-block / macro / nested-mod) miss histogram does a one-shot instrumentation PR produce? |
| [`rust-layer2-proc-macro-miss-control.md`](rust-layer2-proc-macro-miss-control.md) | [UF-FU-012 (b)](../FOLLOWUPS.md) + [UF-FU-003](../FOLLOWUPS.md) | Is the 88.9 % adapter no-target-miss rate on `gridseak-self` dominated by proc-macro expansion (UF-FU-003's known miss shape) or by a second latent cause? A proc-macro-light control run on `ra_ap_stdx` (or equivalent) is the test. |
| [`rust-layer2-adapter-error-histogram.md`](rust-layer2-adapter-error-histogram.md) | [UF-FU-012 (c)](../FOLLOWUPS.md) | What per-variant `SemanticResolverError` distribution does the 0.9 % adapter-error rate decompose into? Specifically — is it dominated by `FileNotInProjectModel`, by LineIndex / VFS failures, or by something else? Today the error rate is only aggregate-counted. |

---

## Adding a new stub

1. File the FOLLOWUPS row first. The stub exists to execute the
   FOLLOWUPS row's plan, not to replace it.
2. Use the template at [`_STUB_TEMPLATE.md`](_STUB_TEMPLATE.md)
   (to be added on next use — keep the template colocated).
3. Cross-link: the stub cites the `UF-FU-NNN` ID; the FOLLOWUPS
   row's `Location` column cites the stub path.
4. Keep stubs short. If a stub exceeds one screen, it is probably
   ready to graduate to `../tasks/`.

---

## Non-goals of this directory

- Not a dumping ground for speculative ideas — every stub must
  have a FOLLOWUPS row justifying why the question is worth
  the next engineer's cycles.
- Not a substitute for `DISCOVERY_REPORT.md` — that document
  contains **settled** discovery findings (D1–D6) that informed
  the sprint plan. Stubs here are **open** questions surfaced
  *by* the sprint, to be answered *after* it.
- Not a design-doc surface. When a stub has enough evidence to
  author an implementation plan, graduate it to `../tasks/`.
