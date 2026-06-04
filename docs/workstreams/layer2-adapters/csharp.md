# WS-LAYER2-CSHARP — C# Layer-2 adapter

> **Kind.** Design stub.
> **Parent.** [`README.md`](README.md) (WS-LAYER2-ADAPTERS index).
> **Status.** Unstaffed *on the Layer-2 axis*. The C# heuristic
> extractor ships today
> (`graphengine-parsing/src/syntax/language/extractors/csharp.rs`).
> Scans on C# repos produce a full call graph and every graph-level
> metric; this stub tracks the unshipped semantic-confirmation
> layer. Per-edge precision on heuristic-only C# is unmeasured —
> there are no dedicated `csharp*.rs` integration tests yet (the
> ground-truth gap noted in `TRUST_AND_ACCURACY_MEMO.md §4`).
> **Target resolver.** [OmniSharp](https://github.com/OmniSharp/omnisharp-roslyn) (Roslyn-backed) — the open-source LSP for C# / .NET, used by VSCode's C# extension and the community Neovim / Emacs integrations.

## Why OmniSharp, not alternatives

- **Roslyn directly.** Building an adapter that consumes
  Roslyn APIs natively (C#-to-Rust interop via COM) is possible
  but extreme friction. OmniSharp wraps Roslyn for us with an
  LSP surface.
- **.NET Language Server Protocol (Razor)** — Razor-specific,
  not a general C# LSP.
- **JetBrains Rider's resolver.** Closed-source, non-starter.
- **VS-internal language service.** Windows-only, not a
  redistributable LSP.

## Expected install friction

- Requires .NET 8 SDK on host. Cross-platform (Linux / macOS /
  Windows).
- OmniSharp ships as a standalone binary; fetch via
  `scripts/download_omnisharp.sh` pattern. Pin SHA256.
- Customer projects must have `.csproj` / `.sln`. Bazel-
  assembled .NET projects require manual project-file hints.

## Adapter contract specifics

- Must resolve `textDocument/definition` on call sites.
- Must handle:
  - **Extension methods** — called as `obj.Method()` but
    defined as `public static Method(this Obj)`. Roslyn
    resolves; ensure our dual-metric counts them as first-
    party confirmed.
  - **Partial classes** — method defined in one file, called
    from another; OmniSharp resolves.
  - **`async` / `Task`-returning methods** — call site usually
    has an `await`; the await itself is not a call, but the
    `.Method()` before it is.
  - **Explicit interface implementations**
    (`void IFoo.Bar() { … }`) — the call site is typed via the
    interface, resolves to the explicit impl.

## Coverage-gap counterparts (Axis 2)

- **Source generators** (`.g.cs` output) — similar to Java
  annotation processors. Emit `generated: true` on nodes;
  exclude from dead-code candidacy.
- **`dynamic` types** (DLR) — `CoverageGap::DynamicDispatch`
  unconditionally; Roslyn cannot resolve `dynamic`.
- **LINQ expression trees** — `Expression<Func<T, bool>>` is
  not executed, it's a data structure. Calls inside an
  expression tree are *parsed but not invoked at runtime* — we
  should NOT emit a Call edge.
- **Reflection (`Type.GetMethod(...).Invoke(...)`)** —
  `CoverageGap::DynamicReflection`.
- **Attributes with runtime dispatch** (ASP.NET routing
  attributes, test-framework attributes) — framework-dispatch,
  emit `framework_annotation_unresolved`.

## Readiness strategy

OmniSharp on a typical ASP.NET + EF-core project: 30–90 s cold.

1. Start OmniSharp with `--stdio`.
2. Wait for the `projectAdded` events for each `.csproj` in the
   `.sln`.
3. Probe `definition` on a canary; retry on `IndexIncomplete`.

Timeout: 180 s.

## Benchmarks to target

- **Accuracy.** 60 % Layer-2-confirmed call edges on a typical
  ASP.NET Core project. Lower (40 %) on a project that uses
  `dynamic` or expression-tree-based DI heavily.
- **Cold-start.** < 3 min on 100k-file monorepo.

## Out of scope (v0)

- **.NET Framework (full-fat, pre-.NET Core).** Different
  project file format, different resolver behaviour. If a
  customer has a Framework-only project, document the
  limitation and scan with heuristic-only for that portion.
- **F# / VB.NET.** Different compilers; separate stubs if
  ever requested.

## Overlooked risks

- **Source-generator drift.** Roslyn source generators run at
  compile time and produce `.g.cs` files in `obj/`. A scan
  against `src/` alone sees the hand-written code; a scan
  including `obj/` sees the generated code. We should default
  to including `obj/` (post-build state) and document the
  choice.
- **`dynamic` escape hatches.** Enterprise C# projects
  sometimes use `dynamic` for ORMs / dynamic data. Layer-2
  confirmation rate on such projects is structurally lower
  than average; the fidelity strip will correctly warn but
  the customer should be primed for the number.
- **Nullable reference types (NRT) ambiguity.** Code pre-NRT
  and post-NRT resolves differently in edge cases; OmniSharp
  handles but the adapter should log the NRT setting it
  inferred from the customer's project file.
