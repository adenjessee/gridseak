# WS-RUNTIME-ORACLE — Runtime-trace integration (Seven-axis, Axis 4)

> **Kind.** Design stub, not an implementation plan.
> **Parent strategy.** [`TRUST_AND_ACCURACY_MEMO.md §5 Axis 4`](../../00-strategy/TRUST_AND_ACCURACY_MEMO.md)
> **Status.** Unstaffed. No engine work started. File exists so the
> axis is tracked out of the 48 h demo push and not re-invented.
> **Why this axis matters.** Static analysis has a ceiling. Runtime
> coverage has the truth. An `_actually_executed_in_prod_ flag on a
> call edge is orders of magnitude more evidence than any heuristic
> or Layer-2 inference. On the dead-code metric specifically, a
> single runtime-coverage signal flips a `no_callers High` verdict
> to `Confidence::RuntimeVerified` (live) — definitive.

---

## 1. What this workstream owns

A single CLI surface: `gridseak ingest coverage --<format> <path>`
that reads a production-standard coverage artefact, matches its
methods to existing graph nodes by FQN + signature, and stamps new
`EdgeKind::RuntimeVerified` edges (or upgrades existing
`Confidence` on matched edges).

It does **not** own:

- the coverage-collection runtime itself (that is the customer's
  existing test / prod infrastructure);
- any new language extractor (those stay with Layer 2 adapters
  and the per-language coverage-gap audit, Axis 2);
- cost / storage of coverage payloads at scale (that is operations,
  tracked separately).

---

## 2. Target coverage formats (priority order)

| Format | Language ecosystem | Why first | Estimated effort |
| :--- | :--- | :--- | :--- |
| JaCoCo XML | Java + Kotlin + Scala | Largest JVM install base; one well-documented format; per-method hit counts. | 2–3 weeks |
| Salesforce Apex Test Run JSON | Apex | Pipes into the highest-priority customer conversation today. Per-method coverage. | 1–2 weeks |
| Istanbul / nyc JSON | Node / TypeScript / JavaScript | Near-universal in TS/JS testing. Per-line → per-function aggregation. | 2 weeks |
| `coverage.py` JSON | Python | Near-universal in Python. Per-line → per-function aggregation. | 1–2 weeks |
| Go `cover` profile | Go | Official tool; simple text format. | 1 week |
| .NET Coverlet / VSTest `.coverage` | C# | Larger format surface; likely last. | 3–4 weeks |

Each integrator is a leaf crate under `graphengine-runtime-oracle/`
implementing a common `CoverageSource` trait. The output is a
normalised `CoverageReport { per_function: HashMap<FQN, HitRecord> }`
that the analysis layer consumes at graph-enrichment time.

---

## 3. Confidence semantics

- Coverage reports upgrade a verdict only when the FQN match is
  **byte-exact** on method signature (no partial / fuzzy match).
  Fuzzy matches emit `UF-FU-RUNTIME-FUZZY` warnings and do not
  enrich.
- A method with `hits == 0` in a sufficiently-broad coverage window
  (e.g. a month of prod traffic, passed via `--window`) is
  **runtime-confirmed dead**, a stronger verdict than any static
  analysis can produce. Emit `DeadCodeConfidence::RuntimeConfirmed`.
- A method with `hits > 0` unconditionally removes it from dead-code
  candidacy regardless of the static no_callers finding. The finding
  is still surfaced in the audit report as *"heuristic resolver
  missed this edge; fix upstream"* — feeds Layer-2 / framework
  resolver backlog.

---

## 4. Integration with existing honesty machinery

- The dual-metric bar (WS-DESKTOP-G.1) gains a third column
  **"runtime-verified"** once this ships.
- The `ResolutionDegraded` banner gains a mitigation:
  *"Critical degradation but runtime coverage from <window> covers
  X % of your calls — those findings are confirmed live."*
- `fidelity` block emits `runtime_verified_edges_count` alongside
  `all_edges_count` and `high_only_edges_count`. The trio becomes
  the customer-facing trust number: static → Layer-2 → runtime.

---

## 5. Out of scope

- **APM / tracing integration** (Datadog, Honeycomb, New Relic).
  Different data shape (traces not coverage), different volume,
  different privacy posture. A separate axis post this one.
- **Auto-instrumentation** of the customer's runtime. We do not
  deploy agents; we read artefacts the customer already produces.
- **Streaming / continuous ingestion.** Batch only in v1.

---

## 6. Dependencies + prerequisites

- Graph nodes must be keyed by FQN + signature consistently across
  extractor and analyser. Today this is true for all shipped
  languages; adding a new language via Axis 1 must preserve the
  invariant.
- Axis 5 (ground-truth datasets) benefits enormously from this
  axis: once a single language has runtime-verified edges, the
  labelled dataset for that language gets a high-quality label
  source for free.

---

## 7. Success criteria

- One format shipped end-to-end (proposed: Apex Test Run JSON, to
  unlock the Salesforce demo pipeline within the same customer
  workstream).
- A customer's dead-code candidate list shows both
  `dead_code_confidence` *and* `runtime_coverage_status` columns
  in the desktop UI.
- The fidelity strip on the dead-code metric emits a
  runtime-verified percentage.

---

## 8. Overlooked risks flagged per user rules

- **Coverage != call graph.** A covered line tells you the method
  executed at least once; it does not tell you what called it. We
  enrich edges we can match; we do not synthesise call edges from
  coverage alone (that would be a silent lie).
- **Coverage windows.** Short windows produce false negatives
  ("covered once in a year's worth of traffic" is live; "covered
  never in a day" is noise). Every runtime verdict is paired with
  the window from which it was derived.
- **Test coverage vs prod coverage.** Customer test coverage reports
  include test-only call edges; prod tracing does not. Both are
  valid for different questions; we must label which signal a given
  edge came from.
