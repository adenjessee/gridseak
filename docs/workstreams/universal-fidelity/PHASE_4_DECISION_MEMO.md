# Phase 4 Decision Memo — Universal-Fidelity Sprint

> **Status.** Decision open. Authored 2026-04-21. Awaiting explicit
> go/no-go from the repo owner before any Phase-B implementation
> starts.
> **Companions.** [`INCEPTION_REPORT.md`](INCEPTION_REPORT.md)
> (measurement artefact), [`CUSTOMER_READINESS_GATE.md`](CUSTOMER_READINESS_GATE.md)
> (customer-bar itemisation).
> **Sprint plan anchor.** [`../../02-strategy/SPRINT_PLAN.md`](../../02-strategy/SPRINT_PLAN.md),
> WS-HONESTY row (Phase 4 = decision-open).

This memo is the artefact the WS-HONESTY Phase 4 row promised: a
scoped, evidence-backed framing of the three Phase-B candidate
shapes, a recommendation, and a decision log that will be stamped
when the owner replies. It does **not** commit engineering time
and does **not** pick a customer. It picks the next *engine-side*
shape so that downstream workstreams (Apex verification, desktop
shell, proof-foundation gap) know what to plan against.

---

## 1. Framing

Phase 3 is closed: T6, T7, T8 shipped; [`INCEPTION_REPORT.md`](INCEPTION_REPORT.md)
records the measured state; [`CUSTOMER_READINESS_GATE.md`](CUSTOMER_READINESS_GATE.md)
enumerates the Bar A/B/C blockers. The remaining engine-side
question is a scope one: what is the next authoritative-tier
signal the engine should gain?

This memo constrains itself to three options. They are not
exhaustive — they are the three that were discussed during
Phase 3 inception and each has a concrete design-doc anchor in
this repo. Any alternative (e.g. "build a TypeScript Layer-2
adapter" or "ship the non-Apex T8.b stubs first") is a write-in
that requires its own memo before it competes.

The memo writes *no* code. The owner replies with a choice; a
subsequent ticket (T9 / T10 / T11 depending on choice) opens
with its own design doc.

### 1.1 Constraints the memo inherits

- **One Authoritative-tier language today.** Rust, at
  measured `high_ratio_on_calls = 10.17 %` on the
  `gridseak-self` dogfood ([INCEPTION_REPORT §2](INCEPTION_REPORT.md)).
  Apex is heuristic-only until WS-APEX-A + -D land.
- **Dead-code confidence is split.** T8's dual metric
  (`no_callers_total` vs `no_callers_high_confidence`) means
  "dead" is no longer a single number; any Phase-B scope that
  does not move the high-confidence column is not moving
  customer-visible dead-code reliability.
- **Measured-fallback discipline applies.** Whatever ships must
  degrade honestly when its input is missing, same contract as
  T7 shallow-clone + T8 empty-gap records.
- **Non-engine blockers are real.** WS-APEX-A, WS-APEX-D,
  WS-DESKTOP-A/B/C/D, WS-PROOF-R3 block different customer
  bars independently of this decision; this memo must not
  assume any of them.

---

## 2. Evidence inputs (read before deciding)

Every section below cites at most two documents. These are the
authoritative inputs. If any of them has changed since
2026-04-21, re-evaluate the recommendation before replying.

| Question this memo has to answer | Primary evidence | Secondary evidence |
| :------------------------------- | :--------------- | :----------------- |
| How authoritative is Rust Layer 2 today? | [`INCEPTION_REPORT.md §3`](INCEPTION_REPORT.md) — 10.17 % high share | [`tasks/T6-rust-layer2.md §10`](tasks/T6-rust-layer2.md) — dogfood table |
| How much of Apex dead-code is already trustworthy? | [`INCEPTION_REPORT.md §5`](INCEPTION_REPORT.md) — NPSP 251/567 | [`tasks/T8-coverage-awareness.md §9`](tasks/T8-coverage-awareness.md) — NPSP canary retrospective |
| What Apex shapes are still not walked? | [`CUSTOMER_READINESS_GATE.md §3.4 + §6`](CUSTOMER_READINESS_GATE.md) | [`FOLLOWUPS.md` UF-FU-018](FOLLOWUPS.md), [`tasks/T8-coverage-awareness.md §4.1`](tasks/T8-coverage-awareness.md) |
| What would a full framework-resolver sprint entail? | [`../apex/FRAMEWORK_RESOLVER_PLAN.md`](../apex/FRAMEWORK_RESOLVER_PLAN.md) | [`../proof-foundation-gap/HAND_AUDIT_LOG.md`](../proof-foundation-gap/HAND_AUDIT_LOG.md) — rev-9 Round 5 |
| What is the Layer-2 seam actually capable of? | [`tasks/T6-rust-layer2.md §3`](tasks/T6-rust-layer2.md) — adapter trait | [`FOLLOWUPS.md` UF-FU-012](FOLLOWUPS.md) — adapter-error inventory |

Reviewer roster: the repo owner is the sole decider today. No
roster expansion is planned at this stage — Phase-B implementation
will be scoped again once a shape is picked.

---

## 3. The three options

Each option is presented identically: scope, effort band, what
it proves, what it explicitly does NOT prove, prerequisite
workstream rows, exit criteria. Effort bands are calendar time,
not engineer-hours; they already bake in the design-doc,
measurement, and FOLLOWUPS-hygiene overhead this sprint revealed.

### 3.1 Option 1 — Narrow declarative-wiring Phase B

**Scope.** Ship the WS-TRUTH-C scope first: LWC, Aura, VisualForce,
Flow, and Platform Event declarative resolvers. Targets R25 (LWC
wiring) and the first half of R28 (Flow / Process Builder
invocable-actions). Does not touch the Framework Resolver backlog
in [`../apex/FRAMEWORK_RESOLVER_PLAN.md`](../apex/FRAMEWORK_RESOLVER_PLAN.md).

**Effort.** 2–4 weeks.

**What it proves.** Every "unparsed caller" that today shows up
as `declarative_wiring_unparsed` is converted to a counted,
named frame. The Apex fidelity-gap `declarative_wiring_unparsed`
column closes cleanly. NPSP's 83 LWC + 12 Aura controllers gain
explicit caller edges. Customer-visible: "why is this method
dead?" gets a real answer on LWC-wired Apex classes.

**What it does NOT prove.** `framework_annotation_unresolved`
(WS-TRUTH-A's `@AuraEnabled` / `@RestResource` surface) and
`dynamic_dispatch_target` (the invocable-actions, batch classes,
Queueable chain) are untouched. The Apex "heuristic-only"
narrative remains for those frames.

**Prerequisites.** WS-TRUTH-B (status-quo framework annotation
registry) must be current, because declarative resolvers emit
into the same registry. Nothing else.

**Exit criteria.**

- `declarative_wiring_unparsed` count drops to zero on NPSP.
- All new resolvers have counters for `resolved` vs `unresolved`
  (honest partial success). Every `unresolved` case lands in a
  follow-up.
- Coverage test on LWC + Aura + VF + Flow + PE fixture set
  (one fixture per bindable surface).

### 3.2 Option 2 — Original Apex-depth Phase B (Framework Resolver backlog)

**Scope.** Execute the full plan in
[`../apex/FRAMEWORK_RESOLVER_PLAN.md`](../apex/FRAMEWORK_RESOLVER_PLAN.md):
annotation-driven caller inference (`@AuraEnabled`,
`@RestResource`, `@InvocableMethod`, `@HttpPost`/`GET`/...),
Queueable / Batch / Schedulable chains, REST namespace
resolution, and the dynamic-dispatch target registry.

**Effort.** 2–3 weeks (concentrated Apex-specific work).

**What it proves.** The Apex heuristic-only narrative ends.
`framework_annotation_unresolved` closes on the canary repos
we have hand-audited. Combined with a Bar-B Apex pilot, a
customer sees an Authoritative-tier story on the same Apex
code that currently only the heuristic reaches.

**What it does NOT prove.** Rust stays at ~10 %
`high_ratio_on_calls` — UF-FU-012 investigations do not
ship here. LWC / Aura wiring (R25) is deferred to a later
Phase-C unless bundled in (which inflates effort by ~1 week).

**Prerequisites.** WS-APEX-A (LSP verification loop) **must**
land first, or the framework resolver ships against a
heuristic-only annotation registry and can't pattern-match
canonical identifiers like `Schema.DescribeSObjectResult`
reliably. WS-APEX-D (LSP robustness) is nice-to-have but not
strictly blocking.

**Exit criteria.**

- `framework_annotation_unresolved` and
  `dynamic_dispatch_target` counts each drop by ≥ 80 % on NPSP
  vs pre-Phase-B baseline.
- New resolvers each ship with a corresponding
  `CAVEAT_*` constant when a shape is detected-but-not-yet-walked.
- At least one canary repo besides NPSP (to avoid single-repo
  overfitting of the resolver rules).

### 3.3 Option 3 — Pause Phase B, ship a second Layer-2 adapter

**Scope.** Build a second-language `Layer2Adapter` implementation
against the `graphengine-ra-ide-adapter` seam. The two
non-speculative candidates are Python (Jedi / pyright as backend)
and Java (Eclipse JDT / `javaparser` as backend). Pick one; the
other stays in the candidate pool.

**Effort.** ~3 weeks.

**What it proves.** The Layer-2 adapter seam generalises beyond
Rust — the trait, the project-model bootstrap, the measured-fallback
pattern, the per-query confidence enum. Ships a second
Authoritative-tier language, which shrinks [`DISCOVERY_REPORT.md §D1`](DISCOVERY_REPORT.md)'s
"single-canary heuristic-primary verdict" from n=1 to n=2.
Unblocks the Bar B "Python full-clone infra team" or "Java
monolith infra team" customer shape, whichever we choose.

**What it does NOT prove.** Apex customers get nothing from
this option. Dead-code confidence on Apex stays where T8 left
it. `declarative_wiring_unparsed` is untouched. If the primary
customer shape is Salesforce-grade Apex, this option actively
defers value.

**Prerequisites.** None engine-side; the adapter seam is in
place. Non-engine: the chosen language needs either a canary
customer repo or a cleared fixture strategy (Option 1 + Option 2
both use NPSP; Option 3 has no equivalent canary yet, which is
a measured honest risk).

**Exit criteria.**

- Second `Layer2Adapter` implementation ships behind a feature
  flag, default-off until dogfood lands.
- Dogfood on ≥ 1 real-world repo in the chosen language.
- `high_ratio_on_calls` measured and recorded in the same
  landing-band framework as [`tasks/T6-rust-layer2.md §10`](tasks/T6-rust-layer2.md);
  the 10-19 % band triggers ship-with-UF-FU, < 10 % triggers
  re-scope.
- D1 verdict in [`DISCOVERY_REPORT.md`](DISCOVERY_REPORT.md)
  updated from n=1 to n=2.

---

## 4. Recommendation

**Preferred: Option 1 (Narrow declarative-wiring Phase B).**

Reasoning (from evidence already in this repo):

1. The engine has exactly one Authoritative-tier language today —
   Rust at 10.17 % ([`INCEPTION_REPORT.md §3`](INCEPTION_REPORT.md)).
   Option 3 adds a second Authoritative language but does not
   close any existing fidelity-gap claim we are already making
   — it opens a new claim. Adding new claims before tightening
   existing ones is out of character for the sprint's
   "measured fidelity" principle.
2. The Bar B offer table in
   [`CUSTOMER_READINESS_GATE.md §5`](CUSTOMER_READINESS_GATE.md)
   shows the shortest customer path is **Rust full-clone
   design-partner pilot**, which is already Ready (pending
   WS-DESKTOP-D 0.5 d). That path doesn't need Phase B. The
   *next* shortest path is **Apex customer, heuristic-only,
   design-partner pilot**, which is also already Ready today
   if the customer accepts the heuristic narrative. Option 1
   converts that conditional "if accepts" into a "declarative
   wiring also answered" story without the 2–3 week WS-APEX-A
   + -D precondition that Option 2 requires.
3. Option 1 closes a measurable column
   (`declarative_wiring_unparsed`) in a bounded time. Option 2
   closes a larger column but with a larger prerequisite stack
   (WS-APEX-A first, then 2–3 weeks, and the prerequisite
   itself is not in this sprint's team). Option 3 opens a new
   column (`layer2_<lang>_adapter_coverage`) that is valuable
   but orthogonal to today's customer asks.
4. Option 1 also preserves Phase-C optionality. After Option 1
   lands, Option 2 is still on the table as a follow-on, and
   Option 3 remains the cleanest "prove generality" proof if
   the first pilot is Python-adjacent.

The recommendation is specifically **not** a rejection of
Options 2 or 3. It is a sequencing claim: declarative wiring
first, then (framework annotations OR second Layer-2), decided
by which customer signed next.

Explicit risk of Option 1 (noted honestly): LWC / Aura wiring
is a web of declarative bindings where "the caller" is an HTML
template attribute. Tree-sitter parsers for LWC / Aura exist
but may not expose every binding shape cleanly; expect at least
one follow-up row (analogous to UF-FU-013/017) for a shape we
discover and don't immediately walk. That is measured-fallback,
not a failure.

---

## 5. What this memo does NOT decide

To preserve scope hygiene:

- **Customer.** Picking a design-partner is a product / sales
  decision, not this memo's.
- **Pricing.** Pricing claims depend on GA-level certainty
  (Bar C), which none of the three options delivers alone.
- **Non-engine blockers.** WS-APEX-A, WS-APEX-D,
  WS-DESKTOP-A/B/C/D, WS-PROOF-R3 remain owned by their
  respective workstream docs. This memo does not promise
  effort against any of them.
- **Timing.** No Phase-B kickoff date is implied. The owner's
  reply triggers the follow-on design doc; the follow-on
  design doc triggers the ticket; the ticket triggers the
  branch.
- **T8.b.** Non-Apex extraction-coverage audit (see
  [`tasks/T8.b-non-apex-coverage.md`](tasks/T8.b-non-apex-coverage.md))
  is carried as a separate postponed item, not a Phase-B
  alternative. It co-ships whenever the customer of the day
  has a non-Apex stack that needs coverage claims honest.

---

## 6. Decision log

This section is empty until the owner replies. When they do,
append an entry with: date, chosen option (or written-in
alternative), reasoning (one paragraph), and the follow-on
ticket ID the owner wants opened.

Do not edit prior entries — append only.

| Date | Decider | Option chosen | Ticket opened | Notes |
| :--- | :------ | :------------ | :------------ | :---- |
| _pending_ | _pending_ | _pending_ | _pending_ | _pending_ |

---

## 7. Audit trail

- 2026-04-21 — Memo authored. Evidence cited: `INCEPTION_REPORT`
  §2, §3, §5; `CUSTOMER_READINESS_GATE` §3, §4, §5, §7;
  `tasks/T6-rust-layer2.md §10`;
  `tasks/T8-coverage-awareness.md §9`; `FOLLOWUPS.md`
  UF-FU-012 + UF-FU-015 + UF-FU-017 + UF-FU-018;
  `../apex/FRAMEWORK_RESOLVER_PLAN.md`.
- Phase 3 artefact commit: see SPRINT_PLAN WS-HONESTY row for
  the single-commit SHA once the close-sprint-open-phase-4 plan
  lands.
