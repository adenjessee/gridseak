# Rust call-graph oracle (rustc MIR)

An answer key for GridSeak's Rust call graph that does not come from
rust-analyzer. GridSeak's Rust compiler tier reads a `rust-analyzer scip`
index, so checking it against rust-analyzer (LSP, `ra_ap_ide`, another SCIP
run) would only show that rust-analyzer agrees with itself. This oracle asks
**rustc** instead, the way `go-oracle` asks `go/types` and `py-oracle` asks
mypy.

## How it works

`mir_oracle.py` runs, for every target of every workspace member (lib with
`cfg(test)`, bins, integration tests, examples):

```
RUSTC_BOOTSTRAP=1 cargo rustc -p PKG@VER --<kind> NAME [--profile test] -- \
    -Zunpretty=mir -Zmir-include-spans=yes -Ztrim-diagnostic-paths=false
```

`RUSTC_BOOTSTRAP=1` unlocks the `-Z` flags on the pinned stable toolchain; no
nightly, no `rustc-dev` component and no network beyond what `cargo build`
already needs. Every MIR body lists its `Call` terminators with the callee
rustc's type checker chose (method calls, UFCS, `<T as Trait>::m`,
`<dyn Trait as Trait>::m`, calls written in macro arguments **and** calls a
macro body expands to), plus function items used as values. The script only
maps rustc's def paths back to source declarations:

- **Caller**: the body's header path (`a::<impl at FILE:L:C>::m`), located by
  its return-place span and the `fn NAME` line above it. Closure and async
  bodies (`f::{closure#0}`) are credited to `f`. Derive- and macro-generated
  bodies have no `fn NAME` in source and are dropped.
- **Callee**: free functions by def path (or a unique same-named function
  when rustc prints a re-exported path); inherent methods by
  `(type, method)`, narrowed by the type's module; `<Ty as Tr>::m` with a
  concrete `Ty` to `Ty`'s impl (or the trait default, or a blanket impl).
- **Dispatch** (`via_iface`): `<dyn Tr as Tr>::m`, `<T as Tr>::m` for a type
  parameter `T`, `<Self as Tr>::m` in a default body and `<S as Tr>::Assoc`
  projections fan out to every workspace impl of `Tr::m` plus its default
  body. `std_trait` marks dispatch through `std`/`core`/`alloc` traits, which
  GridSeak leaves out by design (see below).

Output is the go-oracle shape: `caller_file, caller_line, callee_file,
callee_line, is_call, via_iface` with `fn`-line declaration keys.

## Running

```
python3 mir_oracle.py WS --skip <private-crate> --target-dir /tmp/rust-oracle > truth.json
python3 compare_rust.py truth.json /tmp/rust-oracle/covered_files.json GRAPH.sqlite WS
python3 check_audit_keys.py truth.json /tmp/rust-oracle/covered_files.json WS/.gridseak/audit/<run>/tasks.json
```

The target dir needs a full debug build of the workspace (about 16 GB for
GridSeak). Parsed bodies are cached in `<target-dir>/mir-cache`, so reruns
only re-resolve. `--all-features` builds with every feature on. Only callers
in files the oracle compiled are scored.

## Results (2026-10-13)

GridSeak on itself (copy of `difficulty-search`, 681 files, 120 targets
compiled, one private crate skipped), SCIP tier:

| category | oracle edges | GridSeak recall | GridSeak precision |
|---|---|---|---|
| direct calls | 9,682 | 99.90% | 99.42% (9,701 `via=call` edges) |
| trait dispatch (non-std) | 616 | 97.9% | 100% (601 `via=dispatch`) |
| function values | 54 | 100% | 95.7% (46 `via=value`) |

Every caller listed for 99.6% of the 3,247 symbols that have callers.
`gridseak audit --plan-only --tasks 6`: **6/6 answer keys exact** (one key
entry lies in a bench, which the oracle does not compile).

serde_json v1.0.140 (`762783414e6c`), default features:

| category | oracle edges | GridSeak recall | GridSeak precision |
|---|---|---|---|
| direct calls | 1,258 | 80.7% | 98.0% |
| trait dispatch (non-std) | 801 | 98.8% | 100% |
| function values | 59 | 93.2% | 100% |

Audit keys: 4/6 exact. `mul` misses 11 callers that call un-overridden
default methods of `trait Math` on a concrete `Bigint` (rust-analyzer names
those `Math#m().`, so GridSeak labels them dispatch); `peek` has 8 extra
callers in `#[cfg(feature = "raw_value")]` code that rust-analyzer indexed but
the default-feature build does not compile.

Where serde_json's direct-call recall goes: calls rustc inserts for operators
and desugaring (`==` -> `PartialEq::eq`, `x[i]` -> `Index::index`, `*r` ->
`Deref::deref`, `for` -> `IntoIterator::into_iter`), which have no call
syntax and are not in the SCIP index; and test functions where rust-analyzer's
inference gave up (`tests/test.rs::test_deserialize_from_stream` has no
resolved occurrence at all). The oracle is what surfaced both.

## Known oracle limits

- Path mapping is textual: same-named types in one module tree reached
  through a re-export (two `MockSyntaxExtractor`s in one crate) resolve to
  both candidates; impl self types match by last path segment.
- Type parameters are told from crate-root types by name (`T`, or a name no
  `struct`/`enum`/`type` declares).
- Benches and one private crate are not compiled. A handful of calls inside
  `#[tokio::main]` bodies are still unmapped (GridSeak's 56 call edges not
  in the oracle on itself include these and the re-export ambiguity).
- One feature set per run (default, or `--all-features`); code behind other
  `cfg`s is invisible.

## Toward an exact oracle

A `rustc_driver` callback (`after_analysis`) would remove every heuristic
above: walk `tcx.mir_keys`, take each `TerminatorKind::Call`'s `FnDef(def_id,
args)`, resolve it with `Instance::try_resolve` (direct impl vs. trait
object vs. still-generic), and read both spans with `tcx.def_span` /
`tcx.def_ident_span`, so no paths are printed or re-parsed. It needs the
`rustc-dev` and `llvm-tools` components for the pinned toolchain (`rustup
component add rustc-dev llvm-tools --toolchain 1.91`, then
`RUSTC_BOOTSTRAP=1` and `#![feature(rustc_private)]`), run as
`RUSTC_WORKSPACE_WRAPPER`. Not built here because installing the component
was out of scope for this change; the MIR text oracle already agrees with
GridSeak to within the limits listed above.
