# Discovery stub — Rust Layer 2 adapter no-target-miss proc-macro attribution

> **Source follow-ups.** [UF-FU-012 item (b)](../FOLLOWUPS.md) +
> [UF-FU-003](../FOLLOWUPS.md).
> **Status.** Stub — no control run executed yet.
> **Priority rationale.** Second-highest of UF-FU-012's three.
> Unlike (a) — which is entirely in our code — this investigation
> may prove the miss share is upstream (proc-macro expansion
> unsupported by our adapter), in which case no engineering
> response is warranted until we decide to pay the expansion API
> cost.

---

## 1. The measured shape

From the same 2026-04-21 Gate 1.2 dogfood run:

- `call_refs = 101 343` (tree-sitter-emitted references we asked
  the adapter to resolve)
- `no_target_misses = 90 136` (**88.9 %**)
- High + Medium = 10 311 (10.2 %)
- Errors = 896 (0.9 %, see sibling stub
  [`rust-layer2-adapter-error-histogram.md`](rust-layer2-adapter-error-histogram.md))

`no_target_miss` is the adapter's `Ok(None)` case — rust-analyzer
responded to the `goto_definition` query but returned no target.
That is the expected behaviour when the call site is inside a
proc-macro-expanded body the adapter does not descend into
(UF-FU-003).

## 2. Why this matters

The headline `high_ratio_on_calls = 10.17 %` is being damaged by
either:

- **Cause A.** Proc-macro expansion (UF-FU-003's known miss
  shape). If so, the 88.9 % number is a property of the
  workspace's macro-heaviness, not an adapter bug. No engine
  response — dogfood on less macro-heavy workspaces.
- **Cause B.** A latent second cause — orphan `use` items, VFS
  gaps, toolchain mismatch, something else. If so, there may be
  a second fix that can take a meaningful chunk out of the 88.9 %.

The control run distinguishes them.

## 3. Proposed investigation

**Proc-macro-light control run.** Identify a Rust workspace whose
dependency tree uses proc-macros minimally:

- Candidate A — `ra_ap_stdx` stand-alone (from the
  `rust-analyzer` repo subcrate). Small, self-contained, zero
  `#[derive]` macros in the stdx crate itself.
- Candidate B — any pure-logic crate without `serde`,
  `tokio::main`, `async_trait`, `tracing::instrument`, or
  `clap`'s derive macros. These are the heaviest macro producers
  in `gridseak-graphengine`'s dependency closure.
- Candidate C — a hand-crafted small fixture where the macro
  population is controlled explicitly. Most rigorous but most
  labour-intensive.

Run the dogfood harness against the candidate workspace exactly
as documented for UF-FU-012, record the `no_target_miss` share,
and compare.

## 4. Decision rules

- **Miss share drops to < 20 %.** UF-FU-003 is confirmed as the
  dominant cause. Nothing to fix today — the follow-up remains
  parked on the proc-macro expansion spike (also tracked in
  UF-FU-003's recommended next step, which requires > 15 %
  headroom to graduate to `DISCOVERY_REPORT.md`).
- **Miss share stays > 80 %.** UF-FU-003 is NOT the dominant
  cause. Graduate this stub to a `DISCOVERY_REPORT.md` entry,
  and spike a second cause. Candidate second causes to audit:
  - VFS root-set mismatch — workspace crate members not
    registered with `ra_ap_load_cargo::load_workspace_at`.
  - `rust-toolchain.toml` skew — running a dogfood binary built
    against toolchain X against a workspace requiring toolchain
    Y.
  - Tree-sitter call-site over-generation — we are asking the
    adapter to resolve references that are syntactically
    call-shaped but semantically not (macros-that-look-like-fns,
    e.g. `println!` without the `!`).
- **Miss share lands 20–80 %.** Mixed cause. The report should
  split by enclosing-node shape (reuse the sibling stub's shape
  classifier if landed) to estimate proc-macro share directly.

## 5. Success criteria for the stub

- One control workspace measured. Not a sweep — the decision
  rule above only needs one data point.
- The `no_target_miss` share is recorded alongside the
  `gridseak-self` datum in
  `experiments/results/gate1-2-t6-pr2-dogfood/`
  (gitignored; local artefact).
- One of the three decision branches is taken.

## 6. Non-goals

- Not a proc-macro expansion implementation. The graduation of
  UF-FU-003 to `DISCOVERY_REPORT.md` is gated on this stub, but
  the actual expansion work is a separate ticket that may or may
  not ship.
- Not a multi-canary sweep. One control is enough for the binary
  decision.
- Not a re-measurement of (a) on the control workspace. Different
  workload — would produce new numbers that are incomparable to
  the `gridseak-self` baseline.

## 7. Graduation path

- If Cause A wins: close this stub with a short
  "UF-FU-003 is dominant" note; do not graduate.
- If Cause B wins: graduate to `DISCOVERY_REPORT.md` with the
  measured control-workspace miss share, the top candidate second
  causes, and a recommended next spike. Update UF-FU-012 (b) to
  cite the DISCOVERY_REPORT entry.
