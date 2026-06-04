# WS-LAYER2-GO — Go Layer-2 adapter

> **Kind.** Design stub.
> **Parent.** [`README.md`](README.md) (WS-LAYER2-ADAPTERS index).
> **Status.** Unstaffed *on the Layer-2 axis*. The Go heuristic
> extractor ships today
> (`graphengine-parsing/src/syntax/language/extractors/go.rs`).
> Scans on Go repos produce a full call graph and every graph-level
> metric; this stub tracks the unshipped semantic-confirmation
> layer. Per-edge precision on heuristic-only Go is unmeasured —
> there are no dedicated `go_*.rs` integration tests yet (the
> ground-truth gap noted in `TRUST_AND_ACCURACY_MEMO.md §4`).
> **Target resolver.** [gopls](https://github.com/golang/tools/blob/master/gopls/README.md) — the official Go language server, Google-maintained, distributed with the `go` toolchain.

## Why gopls

Go has exactly one canonical language server. gopls is the
tightest LSP contract in the ecosystem: fast cold-start, excellent
incremental updates, well-documented behaviour on edge cases.
This adapter is expected to be the **easiest** of the five
unstaffed adapters.

## Expected install friction

- Requires `go` toolchain (1.21+). If absent, emit
  `AdapterError::NotInstalled { language: "go" }`.
- Customer projects must have a valid `go.mod` at corpus root.
  Workspaces (`go.work`) supported.
- No additional LSP binary install — `gopls` is fetched via
  `go install` on first use, cached under `$GOBIN`.

## Adapter contract specifics

- Must resolve `textDocument/definition` on call-sites,
  including:
  - Interface-method calls — gopls resolves these to every
    known implementer of the interface (Go's Apex R47.A
    counterpart — fortunately gopls handles it natively).
  - Struct embedding (`type Outer struct { Inner; … }`) —
    calls on embedded methods resolve through the embedding.
  - Method value / method expression syntax (`obj.Method` vs
    `Type.Method`) — both resolve correctly.
- Must handle **build-tag-gated files** — files guarded by
  `//go:build <tag>` are conditionally compiled. The adapter
  should pass `GOFLAGS` from the project's build environment
  so the right files are included.

## Coverage-gap counterparts (Axis 2)

- **Generated code (`//go:generate` output)** — emit
  `generated: true` on these nodes, exclude from dead-code
  candidacy.
- **`reflect.Value.Call` / `reflect.MakeFunc`** — always
  `CoverageGap::DynamicReflection`.
- **CGO (`import "C"`)** — FFI boundary, emit
  `extern_boundary`.
- **Protobuf-generated `.pb.go` files** — near-universally
  `@Generated` equivalent.

## Readiness strategy

gopls is fast:

1. Start gopls with `-mode stdio`.
2. Send `initialize` with the corpus root as `workspaceFolders`.
3. Wait for `initialized` + the first `workspace/diagnostic`
   completion (gopls exposes a progress event).
4. Probe `definition` on a canary; if the canary resolves, mark
   ready.

Expected readiness: < 30 s on a 100k-line Go project (gopls is
famously fast).

## Benchmarks to target

- **Accuracy.** At least 70 % Layer-2-confirmed call edges on a
  typical Go project. Go's type system is simpler than TS/Python,
  and gopls is strong — the high share is achievable.
- **Cold-start.** < 30 s.
- **Incremental warm.** < 1 s per file.

## Out of scope (v0)

- **cgo call-graph beyond the FFI boundary.** We do not attempt
  to analyse the C side.
- **Projects using `GOPATH` mode** (pre-modules). Deprecated
  since 1.16; skip.

## Overlooked risks

- **Go generics (1.18+).** Interface constraints on generic
  functions are the Go equivalent of "protocol dispatch". gopls
  handles the static cases; dynamic generic dispatch is rare but
  should be flagged as `CoverageGap::GenericDispatch` if we
  detect receivers whose type gopls cannot fully instantiate.
- **Build-tag differences between dev and prod.** A scan run
  with `GOOS=linux` will include different files than a scan
  with `GOOS=darwin`. The scan report must record
  `goflags` used so two scans on the same codebase can be
  compared fairly.
