# WS-SCAN-STABILITY — Scan-stability fixtures (Seven-axis, Axis 6)

> **Kind.** Design stub, not an implementation plan.
> **Parent strategy.** [`TRUST_AND_ACCURACY_MEMO.md §5 Axis 6`](../../00-strategy/TRUST_AND_ACCURACY_MEMO.md)
> **Precursor shipped.** R35 determinism fix (byte-identical
> `ge-analyze` output on two runs at the same git SHA) is the
> *per-run* determinism guarantee. TR-A.0 byte-identical CI gate is
> specified but not yet armed with vendored artefacts.
> **This workstream is the *across-revision* stability layer on top
> of that.** R35 says "the engine is consistent with itself at a
> point in time". TR-A.0 + this workstream say "the engine's output
> only changes when the engine genuinely changes, never drifts
> silently".

---

## 1. What this workstream owns

- A curated **canary corpus set** stored at
  `experiments/scan-stability/<corpus-name>/`, each with a pinned
  git SHA, an expected `baseline.json`, and a distribution-drift
  tolerance per metric.
- A nightly / per-release CI job that re-scans every canary and
  compares the new output against the stored expected output.
- A distribution-drift alarm that fires when *any* metric
  deviates by more than its declared tolerance, forcing the
  engineer causing the drift to either update the expected
  output (if the change is intentional) or fix the engine (if
  the drift is unintended).

It does **not** own:

- the R35 per-run determinism gate (shipped separately).
- the adversarial fixtures of Axis 7 (those test *degradation
  shape*; these test *metric stability*).

---

## 2. Canary corpus set (v0 proposal)

| Corpus | Language | Why this corpus | Tolerance profile |
| :--- | :--- | :--- | :--- |
| `dreamhouse-lwc` pinned SHA | Apex | Smallest auditable Apex corpus; full hand-audit coverage in Round A. | Zero-drift on every metric. |
| `apex-recipes` pinned SHA | Apex | Mid-size Apex, Round A audit as ground-truth. | Zero-drift on `no_callers_high_confidence` and `resolution_degraded.severity`; ±2 on `fan_in` rankings. |
| `NPSP` pinned SHA | Apex | Large Apex canary, Round 5b audit established baseline. | Zero-drift on `no_callers_total`, `no_callers_high_confidence`; ±3 on `complexity.functions_above_threshold`. |
| `gridseak-self` pinned SHA | Rust (+ Layer 2) | First-party Rust with known ground-truth; also the only corpus where Layer-2 vs heuristic comparison exists. | Zero-drift on `high_only_edges_count`, `all_edges_count`, top-20 `fan_in`. |
| A TypeScript canary (Next.js example app, pinned) | TS/JS | Once Axis 1 ships TS adapter, this corpus becomes Layer-2-confirmable. | Initial: zero-drift on file / node counts; post-Layer-2: zero-drift on `high_only_edges_count` within ±5 %. |
| A Python canary (Django tutorial, pinned) | Python | Once Axis 1 ships Python adapter, analogous to TS. | Same. |

Each canary's pinned SHA is recorded in a `corpus.toml`:

```toml
[canary]
name = "NPSP"
upstream = "https://github.com/SalesforceFoundation/NPSP"
sha = "<pinned-sha>"
retrieved_at = "2026-04-17"
retrieval_script = "scripts/stability_fetch_npsp.sh"

[tolerance.no_callers_total]
drift = 0   # zero-drift

[tolerance.no_callers_high_confidence]
drift = 0

[tolerance.max_call_depth]
drift = 1   # ±1 on the depth metric

[tolerance."complexity.functions_above_threshold"]
drift_ratio = 0.02   # 2 % relative drift allowed
```

---

## 3. Drift classification

When a canary re-scan diverges from the stored baseline, the CI
job classifies the drift:

- **Intentional drift.** The PR contains an engine change (e.g.
  shipping R48 VF-binding extraction will *correctly* reduce
  `no_callers_total` on NPSP). The PR author updates the stored
  baseline as part of the PR, with a changelog entry explaining
  why the metric moved and by how much.
- **Unintentional drift.** The PR did not intend to change the
  canary output. CI fails. The author either:
  - reverts the offending change,
  - narrows the engine change so the canary holds,
  - or, if the drift is an actual *improvement* that was not
    declared, the PR description is updated and the baseline
    refreshed.

The *default assumption* is that every metric drift is a bug
until proven otherwise. This is the inversion of current
practice where metrics shift silently as the engine evolves.

---

## 4. Integration with existing honesty machinery

- A canary scan's `baseline.json` **is** the stored expected
  output. No post-processing; the byte-identical gate from R35
  does the comparison.
- The `HAND_AUDIT_LOG.md` Round rows point back to the pinned
  canary SHA so audit verdicts remain meaningful across engine
  revisions.
- The `TRUST_AND_ACCURACY_MEMO.md §3.6` section on determinism
  (post-ship) will absorb this workstream as its inter-release
  counterpart.

---

## 5. Relationship to existing tests

| Today | This workstream |
| :--- | :--- |
| Unit tests lock function-level behaviour. | Canaries lock whole-corpus output. |
| Integration tests (`*_e2e.rs`) lock specific extractor shapes. | Canaries lock *emergent aggregate metrics* across thousands of shapes. |
| Regression fixtures lock expected extraction for known shapes. | Canaries catch *distribution drift* on unknown shapes that nobody wrote a fixture for. |

A regression fixture failing is a direct pointer: "this shape
broke". A canary failing is an indirect signal: "something moved,
find out what." Both are necessary.

---

## 6. Success criteria

- Five canaries with pinned SHAs and stored baselines committed
  under `experiments/scan-stability/`.
- CI job runs all five canaries per PR touching the parsing /
  analysis layers, and nightly on `main`.
- A canary drift fails the PR with a clear diff against the
  stored baseline.
- Every release's changelog carries a "canary deltas" section
  enumerating intentional baseline updates.

---

## 7. Out of scope (v0)

- **Scaling the canary set beyond five.** Adding more canaries
  grows CI time linearly; v0 keeps the set small and lets it
  grow with per-language adapter roll-out.
- **Continuous benchmarking against upstream HEAD.** The whole
  point of pinning is that we test against a fixed input. A
  separate workstream can do "engine vs current HEAD" benchmarks
  if ever needed.
- **Performance-regression gating.** Scan time / memory-use
  canaries are a separate workstream under performance, not
  semantic correctness.

---

## 8. Dependencies + prerequisites

- R35 (determinism) — SHIPPED. Without R35, the whole
  byte-identical comparison is moot.
- Canary-fetch scripts that produce identical input across CI
  runs (fetch a specific SHA, not `main`; archive the
  `.tar.gz` if upstream ever rewrites history).
- A `stability compare` subcommand on `ge-analyze` that takes
  the stored baseline and the new output, produces a
  human-readable drift diff for CI log output.

---

## 9. Overlooked risks flagged per user rules

- **Pinned canary going dark.** If the upstream repo is deleted
  or rewritten, a pinned-SHA fetch may break. Mitigation:
  vendor the tarball once, under
  `experiments/scan-stability/<canary>/.tarball.tar.gz`.
  Disk cost is small; audit-trail resilience is high.
- **False-positive drift from clock / hostname / OS
  idiosyncrasy.** R35 fixed the largest of these; future
  drift-sensitive fields must be reviewed before shipping. Any
  field that includes a timestamp or absolute path is a red
  flag and must be normalised out of the comparison.
- **Intentional drift becoming a loophole.** If every engineer
  updates the canary baseline as "intentional", the gate
  becomes rubber-stamp. Mitigation: require a reviewer on the
  baseline update commit, separate from the engine-change
  commit; and surface canary-baseline updates in the release
  changelog so shipping silently is visibly hard.
- **Customer corpus is not a canary.** The customer's
  production code is never a canary — it moves, it's private,
  and we do not own it. The canaries are our own proxy for
  "what do customer scans probably look like"; real customer
  stability is a separate trust conversation (covered by the
  per-scan `scan_metadata.json` audit trail, not by this
  workstream).
