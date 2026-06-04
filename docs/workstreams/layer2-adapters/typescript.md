# WS-LAYER2-TS — TypeScript / JavaScript Layer-2 adapter

> **Kind.** Design stub.
> **Parent.** [`README.md`](README.md) (WS-LAYER2-ADAPTERS index).
> **Status.** Unstaffed *on the Layer-2 axis*. The TypeScript and
> JavaScript heuristic extractors ship today
> (`graphengine-parsing/src/syntax/language/extractors/typescript.rs`
> and `javascript.rs`) with dedicated end-to-end tests covering
> extraction, FQN resolution, import-edge regression, config edge
> cases, LSP plumbing, progress events, and validation. Scans on
> TS / JS repos work; this stub tracks the unshipped semantic-
> confirmation layer that would let us promote those edges from
> heuristic to Layer-2 confirmed.
> **Target resolver.** [tsserver](https://github.com/microsoft/TypeScript/wiki/Standalone-Server-%28tsserver%29) — the TypeScript language service shipped with the `typescript` npm package, protocol-compatible with the TypeScript LSP.

## Why tsserver, not alternatives

- **Coverage.** tsserver is the canonical resolver for the
  TypeScript / JavaScript ecosystem. All other TS tooling (ESLint
  typescript-eslint, VSCode, WebStorm's TypeScript plugin, deno's
  checker) consumes it.
- **Maturity.** Microsoft-maintained since 2014; exhaustive
  handling of declaration-file resolution, `tsconfig.json` project
  references, paths-aliases, JSX, decorators.
- **Protocol.** tsserver exposes a JSON-over-stdin protocol (the
  legacy tsserver protocol) plus an LSP wrapper. Either works;
  legacy is slightly richer (has `references` batching).

## Expected install friction

- Requires Node.js 18+ (LTS) on the customer / developer host.
- Requires `typescript` in the project's `node_modules` or a
  globally installed version whose version matches the
  project's `tsconfig.json` `compilerOptions.target` semantics.
- Projects using `yarn pnp`, `pnpm`, or other non-standard
  resolutions need a project-specific tsserver bootstrap.

## Adapter contract specifics

- Must resolve `textDocument/definition` on a call-site byte range
  to the target symbol's declaration file + range.
- Must honour TypeScript's **structural typing**: a call on an
  interface-typed variable should resolve to every concrete impl
  that satisfies the interface, not just the declared type.
  Equivalent to the Apex R47.A subtype-dispatch shape (see
  `../proof-foundation-gap/FOLLOWUP_RISKS.md §R47`).
- Must handle **barrel re-exports** (`export { foo } from './bar';`)
  without inflating call-edge count.
- Must handle **dynamic `import()`** — these do NOT produce a
  Layer-2 edge, they produce `framework_annotation_unresolved`
  on the import site.

## Coverage-gap counterparts (Axis 2)

Shapes the extractor may drop silently today, which this adapter
should surface as gaps rather than drop:

- **Decorator bodies** — decorated class / method bodies where
  the decorator is user-defined rather than a known framework.
- **JSX closures** — inline event handlers inside JSX (`onClick={() => foo()}`).
- **Object spread** through which a function flows (`{ ...handlers, ...overrides }`).
- **`Object.defineProperty`** and `Reflect.defineProperty` setters.
- **`Proxy`** handlers (no static resolution possible).
- **String-based dispatch** via bracket access (`obj[methodName]()`).

Each of these should either produce a Layer-2 confirmed edge (if
tsserver knows), an explicit `CoverageGap::<Shape>` (if neither
heuristic nor Layer-2 can resolve), or a `framework_annotation_unresolved`
finding (if a known framework plugin owns the dispatch).

## Readiness strategy

tsserver's startup on large projects with many `@types/*` packages
is observably slow (5–30 s). The adapter must:

1. Start tsserver.
2. Send a `projectInfo` probe on the corpus root.
3. Block until either a successful `references` probe on a
   hand-picked canary symbol returns non-empty, or a timeout
   elapses.

Same pattern as the Apex `ProgressAndProbe` readiness strategy
(see Jorje P0 investigation), minus the H1 / H2 dichotomy we only
fully understood after instrumenting Apex.

## Benchmarks to target

- **Accuracy.** At least 40 % of call edges Layer-2 confirmed on a
  typical `tsconfig strict: true` project (vs Rust's 10 % on a
  heavily-macro'd corpus — TS is easier).
- **Cold-start overhead.** < 60 s of tsserver readiness on a
  10k-file project.
- **Incremental warm.** < 5 s per file on subsequent file scans
  (tsserver's incremental mode).

## Out of scope (v0)

- Web-worker / browser tsserver (we only support the Node host).
- Deno / Bun's built-in checkers — different protocol, re-
  stubbed if a customer asks.
- Flow (Facebook's type system). Dead ecosystem; skip.

## Overlooked risks

- **`tsconfig.json` ambiguity.** Projects with multiple
  `tsconfig.json` files (monorepos) require path disambiguation.
  The adapter must accept `--tsconfig <path>` and must emit a
  warning if it auto-picks one.
- **Version skew.** tsserver 5.x has different protocol variants
  than 4.x. Pin the TypeScript version we launch tsserver from,
  independent of the project's TS version, to keep the adapter
  API stable — but then separately honour the project's TS
  version for semantic-analysis behaviour.
- **Declaration-file-only targets.** A call that resolves to a
  `.d.ts`-only declaration (e.g. a Node builtin) is *structurally*
  resolved but semantically external. Label as
  `external_boundary`, not as a first-party Layer-2 confirmation.
