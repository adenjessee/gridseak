# Discovery stub — Rust Layer 2 `SymbolIndex` caller/callee loss rate

> **Source follow-up.** [UF-FU-012 item (a)](../FOLLOWUPS.md).
> **Status.** Stub — no instrumentation landed yet.
> **Priority rationale.** Highest-leverage of UF-FU-012's three
> investigations: closing the 36 % loss alone takes `edges_emitted`
> from 6 550 → ~10 311, pushing `high_ratio_on_calls` from 10.17 %
> toward the 20 % ship-clean threshold without requiring any
> upstream `ra_ap_ide` change.

---

## 1. The measured shape

From the 2026-04-21 Gate 1.2 dogfood on the full `gridseak-graphengine`
workspace (release binaries, Rust 1.91.0, aarch64-apple-darwin):

- `high + medium = 10 311` (adapter resolved the callee)
- `edges_emitted = 6 550` (the edge actually landed in the graph)
- Difference: **3 761 resolutions lost** in the SymbolIndex lookup
  layer — specifically in one of the two calls to
  `graphengine-parsing/src/infrastructure/semantic/rust_layer2.rs`:
  - `SymbolIndex::find_enclosing_function(caller_range)` — finds the
    function containing the call site's byte range.
  - `SymbolIndex::find_callee_for(resolved_location)` — finds the
    symbol at the resolution target's byte range.

Either lookup failing drops the edge. The adapter itself did its job
(it returned a `goto_definition` target); the graph layer silently
discarded the result.

## 2. Working hypothesis

`rust-analyzer`'s line/column coordinates for `goto_definition`
targets do not align byte-for-byte with Tree-sitter's `Range` for
symbols inside:

- `impl` blocks (the `impl Foo { fn bar() {...} }` method position
  — rust-analyzer points at `bar`'s declaration, tree-sitter's
  `SymbolIndex` may have indexed the `impl_item`'s outer range).
- Macro invocation sites (both the call site, because the expanded
  callable lives on a synthetic range, and the callee definition if
  it is itself macro-generated — `#[derive]`).
- Nested `mod` scopes (the span may be attributed to the parent
  module's file byte range if the nested mod is inlined).

These are hypotheses, not measurements. The stub's job is to
disambiguate.

## 3. Proposed investigation

**One-shot instrumentation PR.** Land a diff that:

1. Wraps both `SymbolIndex` lookups with a miss-reason enum:
   - `CallerNotFound { call_site_range: Range }`
   - `CalleeNotFound { target_range: Range }`
   - `BothFound` (the happy path — reference count, not a miss)
2. Serialises a histogram keyed by the enclosing node shape:
   - `ImplBlockMethod` / `MacroInvocationSite` /
     `NestedInlineMod` / `FreeFn` / `TraitDefault` / `Other`.
   The shape classifier re-walks the tree-sitter AST to
   identify the enclosing construct for each miss's range.
3. Writes the histogram to the existing `ResolveSnapshot`
   record in `experiments/results/gate1-2-t6-pr2-dogfood/rollup.json`
   (gitignored; local artefact — same location UF-FU-012 named).

**Non-implementation.** No fix in this PR. Measurement-only.
Decide the fix shape after reading the histogram.

## 4. Success criteria for the stub

- Histogram covers ≥ 95 % of the 3 761 losses.
  (The remaining 5 % can fall into `Other` and become their own
  follow-up.)
- One category reveals itself as > 50 % of the loss — that becomes
  the first fix PR's scope.
- If no category is > 50 %, the stub graduates by documenting the
  top 3 categories and their respective shapes; each gets its own
  sub-ticket rather than one omnibus fix.

## 5. Non-goals

- Not a fix. Measurement only.
- Not a re-architecture of `SymbolIndex`. It is possible (even
  likely) the underlying bug is a single off-by-one in the
  byte-range comparison — the instrumentation PR must not
  prejudge that by rewriting the lookup shape.
- Not cross-language. This stub is scoped to the Rust Layer 2
  adapter; the `SymbolIndex` type itself is a generic seam, but
  its loss behaviour on other languages is out-of-scope until
  a second Layer 2 adapter exists.

## 6. Graduation path

When the histogram lands and the dominant miss-category is
named, promote to `../tasks/T6.a-symbol-index-fix.md` (or
similar) with the fix plan, and update UF-FU-012's
`Recommended next step` column to cite the new design doc.
