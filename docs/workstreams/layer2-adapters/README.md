# WS-LAYER2-ADAPTERS — Per-language semantic-resolver adapters (Seven-axis, Axis 1)

> **Scope clarifier (read this first).** This page tracks the
> **Layer-2 semantic-resolver agreement** axis only — i.e. wrapping
> a per-language LSP/compiler-frontend so the engine can stamp call
> edges as "double-checked by a production-grade resolver." It does
> NOT track which languages the engine *parses*. Heuristic
> tree-sitter extractors already ship for **Rust, Apex, TypeScript,
> JavaScript, Python, Go, Java, and C#**
> (`graphengine-parsing/src/syntax/language/extractors/`); those
> produce the call graph that every graph-level metric (cycles,
> coupling, hotspots, blast radius, god-functions, complexity)
> is computed from. "Unstaffed" rows below mean *no Layer-2
> resolver yet*, not *language unsupported*. A scan on a TS or
> Python repo today produces a full report; it just cannot populate
> the `high_only_*` confirmed-share columns until that language's
> adapter ships.
>
> **Kind.** Index / coordination page for per-language Layer 2
> adapter stubs. Each language-specific stub lives alongside this
> file.
> **Parent strategy.** [`TRUST_AND_ACCURACY_MEMO.md §5 Axis 1`](../../00-strategy/TRUST_AND_ACCURACY_MEMO.md)
> **Shipped today (Layer-2 only).** Rust (via `ra_ap_ide` /
> rust-analyzer library) at 10.17 % Layer-2-confirmed call-edge
> share on `gridseak-self`. All other languages are
> **heuristic-only on the resolver axis** — their tree-sitter
> extractors still emit a call graph, just without semantic
> confirmation.

## Why one adapter per language

Every production-grade semantic resolver on earth is language-
specific. TypeScript's tsserver is not going to resolve Python;
gopls is not going to resolve Java. The alternative to one adapter
per language is *"we roll our own semantic resolver"* — which is
what every previous attempt in this space has failed at. We do not
roll our own; we wrap the ecosystem's canonical one and measure
agreement.

## Common contract

Each adapter implements the `Layer2Adapter` trait (sketched in the
WS-HONESTY sprint plan, shipped for Rust). Contract:

- **Input.** A file-path and byte-range for a call-site.
- **Output.** `Option<Vec<Resolution>>` where each resolution is a
  candidate target FQN with a confidence tier. `None` means the
  adapter could not resolve (equivalent to "adapter disagreement"
  in the dual-metric logic).
- **Warm-up.** Each adapter must expose a `ReadinessStrategy` so
  the engine can block until the resolver's index is actually
  usable on the corpus (the Apex-LSP Jorje P0 arose precisely
  because we sent requests before the indexer was ready).
- **Error taxonomy.** Adapters return specific error variants
  (`AdapterError::NotInstalled`, `::ConfigMissing`, `::Timeout`,
  `::IndexIncomplete`, `::Unsupported`) so the engine can degrade
  honestly rather than silently fail.

## Per-language status

`Heuristic extractor` is the tree-sitter + custom-extractor path
that emits the call graph every metric is computed from.
`Layer-2 adapter` is the semantic-resolver-agreement path that
stamps a subset of those edges as confirmed.

| Language | Heuristic extractor | Layer-2 resolver | Layer-2 stub | Layer-2 status |
| :--- | :--- | :--- | :--- | :--- |
| Rust | shipped (`extractors/rust.rs`) | `ra_ap_ide` (rust-analyzer library) | — | **Shipped** (T6) — 10.17 % confirmed-edge share on `gridseak-self` |
| Apex | shipped (vendored `tree-sitter-sfapex`, deepest framework dispatch in the engine) | apex-jorje LS | Existing WS-APEX-A workstream | Jorje P0 open; see [VALIDATION_RESULTS §P0](../apex/VALIDATION_RESULTS.md) |
| TypeScript | shipped (`extractors/typescript.rs`, 8 dedicated test files) | tsserver (TS language service) | [`typescript.md`](typescript.md) | Unstaffed |
| JavaScript | shipped (`extractors/javascript.rs`) | tsserver (shared with TS) | [`typescript.md`](typescript.md) | Unstaffed |
| Python | shipped (`extractors/python.rs`, no dedicated integration tests) | pyright | [`python.md`](python.md) | Unstaffed |
| Go | shipped (`extractors/go.rs`, no dedicated integration tests) | gopls | [`go.md`](go.md) | Unstaffed |
| Java | shipped (`extractors/java.rs`, no dedicated integration tests) | eclipse.jdt.ls | [`java.md`](java.md) | Unstaffed |
| C# | shipped (`extractors/csharp.rs`, no dedicated integration tests) | OmniSharp / Roslyn | [`csharp.md`](csharp.md) | Unstaffed |
| C / C++ | not shipped | clangd | — | Backlog, not stubbed v0 |
| Kotlin | not shipped | kotlin-language-server | — | Backlog |
| Swift | not shipped | sourcekit-lsp | — | Backlog |

## Ordering rationale

TypeScript and Python are first because of install-base reach and
LSP maturity. Go is next because gopls is the cleanest LSP in the
ecosystem (tightest contract, fastest startup). Java and C# follow
because they unlock the enterprise Salesforce-adjacent segment
(Apex + Java monorepos are common). C / C++ / Kotlin / Swift are
backlog because either the LSP is less mature (kotlin-ls) or the
install base in our target segment is smaller.

## Trust consequence per adapter shipped

Each adapter moves one language from *"high-only column is
structurally zero on this language"* (apex today) to *"high-only
column carries N% of the call graph"*. The customer-facing claim
*"N % of your calls are Layer-2 confirmed"* is the promotion.

## Out of scope

- Replacing a language's adapter (e.g. swapping pyright for
  jedi). If a better resolver emerges per language, we re-adapt;
  but v0 has one resolver per language.
- **Rolling our own semantic resolver for any language.**
  Explicit anti-goal; it is the failure mode of every previous
  call-graph tool.

## Overlooked risks flagged per user rules

- **LSP install ergonomics vary wildly.** TS server is bundled
  with npm; pyright is a pip install; clangd requires system
  build tools; apex-jorje requires Temurin 17 (which surfaced in
  the Apex LSP setup friction). Each adapter's stub must lead
  with an install-prerequisite section, and the engine must
  detect missing prerequisites and emit `AdapterError::NotInstalled`
  cleanly — never a silent fallback.
- **Per-language coverage gap asymmetry.** Axis 2 (per-language
  coverage-gap audit) is a *prerequisite* for an honest claim
  about each adapter. A Layer-2 adapter that doesn't know what
  shapes it drops produces pretty numbers without the honesty
  frame.
