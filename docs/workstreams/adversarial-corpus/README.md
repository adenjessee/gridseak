# WS-ADVERSARIAL — Adversarial test corpus (Seven-axis, Axis 7)

> **Kind.** Design stub, not an implementation plan.
> **Parent strategy.** [`TRUST_AND_ACCURACY_MEMO.md §5 Axis 7`](../../00-strategy/TRUST_AND_ACCURACY_MEMO.md)
> **Status.** Unstaffed. The existing fixture suites in
> `graphengine-parsing/tests/` and `graphengine-analysis/tests/` are
> *regression* fixtures, not *adversarial* fixtures — they lock
> known-good behaviour, they do not stress graceful degradation on
> known-bad inputs.
> **Why this axis matters.** When a prospect says *"we have a lot of
> reflection / macros / generated code — how does your engine
> handle it?"* we need a benchmark number, not a promise. Graceful
> degradation must be *measured*, not assumed.

---

## 1. What this workstream owns

A curated set of intentionally-pathological code repositories with
an expected-behaviour contract for each one. The engine is run
against each, and the output is asserted to **degrade gracefully**:
emit `ResolutionDegraded` at the appropriate severity, stamp
`CoverageGap::<ShapeName>`, never silently lie.

It does **not** own:

- *solving* any of the adversarial shapes. Those fixes are tracked
  in the relevant language workstream (e.g. proc-macro support is
  UF-FU-012b under WS-LAYER2-RUST).
- production corpus scans (those use the hand-audit log and Axis 5
  ground-truth datasets).

---

## 2. Corpus families (v0 proposal)

| Family | Representative corpora | Language | Expected failure shape |
| :--- | :--- | :--- | :--- |
| Heavy reflection | Spring PetClinic (`@Autowired`, `@EventListener`); Django with `__getattr__` proxy middleware; Ruby `method_missing` patterns (future) | Java / Python | `framework_annotation_unresolved` on N% of calls, `ResolutionDegraded::Medium` |
| Macro-generated code | serde_derive / tokio-macros heavy Rust; C preprocessor macro soup; TypeScript with Babel plugins | Rust / C / TS | `CoverageGap::MacroExpansion`, proc-macro FQNs present in `dropped_during_resolution` |
| Dynamic eval | Python test fixture using `exec`/`eval`; Node vm.runInContext example; Ruby eval (future) | Python / TS | Static analyser cannot reach inside; emit `CoverageGap::DynamicEval`, no silent edges |
| Obfuscated / minified | Uglified JS bundle; WebPack output; Rollup optimised output | JS | extractor emits `CoverageGap::MinifiedOrObfuscated`, analysis layer suppresses dead-code on these files |
| `@Generated` / .pb.go / .d.ts | Protobuf-generated Go; .d.ts declaration-only TS files; JPA entity-generated Java | Any | extractor emits `generated: true` property on nodes; analyser excludes from dead-code candidacy |
| Reflection-dispatched test frameworks | Apex `@isTest` classes; JUnit `@Test`; pytest; xUnit | Apex / Java / Python / C# | Per-language framework detector keeps these out of dead-code; scan reports explain why |
| Polyglot / FFI boundaries | Rust with C FFI extern blocks; Node with native addon; Python C extension | Rust / Node / Python | boundary emits `framework_annotation_unresolved` or `extern_boundary`; never a silent wrong edge |
| Circular or self-modifying build | Build scripts that generate code that generates code; bootstrap compilers | Any | graph still produces; invariant: no panic, no infinite loop, honest degradation |

---

## 3. Contract per adversarial fixture

Each corpus ships with `EXPECTED.md` containing:

- **Expected report characteristics**: severity of
  `ResolutionDegraded`, count of findings by reason, specific
  `CoverageGap::*` variants that must appear, specific invariant
  statuses.
- **Explicit anti-assertions**: no finding of type X on file Y
  (e.g. no `no_callers_high_confidence` on a `__generated.pb.go`).
- **A narrative**: *what would a lying tool do on this corpus,
  and how do we verify we don't?*

The CI gate is: *for each adversarial fixture, running the engine
produces output whose shape matches `EXPECTED.md`*. A shape deviation
fails the build.

---

## 4. How this differs from the regression fixtures we already have

| Today | This workstream |
| :--- | :--- |
| `apex_r41_field_initializer_e2e.rs` locks *a specific known-good extraction* — we expect exactly this graph out of exactly this Apex source. | An adversarial fixture says *we expect the engine to produce any graph, but it MUST carry these degradation flags and MUST NOT carry these false-confidence claims.* |
| One focussed input shape. | Realistic multi-file codebases, intentionally hostile. |
| Passes or regresses engine code. | Passes or regresses the *honesty machinery*. Degradation is the success case, not the failure case. |

---

## 5. Success criteria

- 15+ adversarial fixtures across the language matrix.
- Every fixture's `EXPECTED.md` is asserted by the benchmark runner
  in CI.
- A customer can point at any of the published fixtures and ask
  *"run that against your engine, here's the expected output"* —
  and we do it in under 2 minutes with byte-matching results.
- The phrase *"what about our macro / reflection / obfuscation
  situation?"* in a customer call gets answered with
  *"here's the adversarial benchmark for that shape"*, not a
  promise.

---

## 6. Dependencies + prerequisites

- Determinism gate (R35, shipped). Adversarial fixtures are only
  asserted byte-identically if the engine is deterministic on
  them. Re-verify R35 holds on each adversarial corpus as it lands.
- CI runtime budget. Adversarial corpora are larger than
  regression fixtures; expect 30–120 s per fixture. Gate on PR,
  not on every push; run nightly on main.

---

## 7. Out of scope (v0)

- **Fuzzing** the parser / extractor. That is a separate workstream
  (parser robustness via cargo-fuzz). Overlaps but does not
  subsume this.
- **Security adversarial corpora** (SQL injection test suites,
  XSS samples). This axis is for *semantic adversarial* — code
  that is hard to analyse, not code that is attacking.
- **Pathologically large corpora** (1M-line repos). Those go to
  a separate scalability workstream.

---

## 8. Overlooked risks flagged per user rules

- **Coverage != completeness.** 15 adversarial fixtures is not a
  proof that the engine degrades gracefully on *all* pathological
  inputs — it is a proof that it degrades gracefully on 15. The
  value grows with the corpus.
- **Expected.md drift.** If we fix a bug and the expected output
  changes, we must update `EXPECTED.md` in the same PR with a
  human-readable narrative. The file is not a freeze, it is a
  living contract.
- **Misuse as a marketing benchmark.** We should publish the
  adversarial scorecard transparently, but we must not market it
  as "the engine handles reflection" — we market it as "here is
  exactly what the engine does on reflection, and here is where it
  stops".
