# Commit 1 Scoping Decisions — TR-A.1 + TR-A.2 + R33

Supplement to `PHASE_A_EXECUTION_PLAN.md` §3, §8. Locks two design
questions that surfaced while scoping Commit 1 and were not fully
resolved in the execution plan. Both decisions are load-bearing for
Commit 1 PR review.

---

## Q1 — `CallSite.arg_types` shape: **B, with a preparatory type-move refactor**

### Evaluating the options as stated

**Option A (`ArgTypeRef` wrapper enum):**

```rust
pub enum ArgTypeRef {
    Unknown,
    Apex(ApexTypeRef),
}
```

Proponents' case: `CallSite` stays language-agnostic, future languages
add variants, `Unknown` is the neutral default. Reviewed against the
code: this creates an unambiguous layer inversion.
`application/ports.rs` today imports from `crate::domain` only
(verified: zero `use crate::syntax` / `use crate::infrastructure` in
that file). Importing `ApexTypeRef` from
`crate::syntax::language::apex::class_symbols` makes `application`
depend on `syntax::language::apex`, which reverses the architectural
arrow (application is supposed to sit above syntax). That is not a
detail; that is the entire reason the existing `class_symbols` hint
is carried as `(String, String)` — an already-serialised JSON
payload — specifically to keep Apex types out of the port surface.

Also: every consumer site in the Apex resolver pays the cost of
`match` unwrapping `ArgTypeRef::Apex(...)` on every arg, on every
call, every resolve. Small overhead, but zero benefit — we have one
language populating this, and if Java arrives in Phase B it will
carry its own Java signature types, not share Apex's. The enum
doesn't meaningfully share anything across languages.

**Option B (`Vec<ApexTypeRef>` direct):**

Proponents' case: simplest code, no unwrap. Stated downside: "pulls
Apex domain type into cross-language port surface." This is the
**same** layer-inversion concern as A — wrapping it doesn't fix it,
only hides it. Both options require `application/ports.rs` to import
an Apex-namespaced type. So the "cross-language port surface"
concern is a red herring when comparing A vs B; it applies to both
equally.

**Option C as stated — sidecar on `SyntaxResults`:**

Not listed in the original question, named here to reject explicitly
so it isn't picked up later:

```rust
pub struct SyntaxResults {
    ...
    pub call_sites: Vec<CallSite>,
    pub call_arg_types_apex: Vec<Vec<ApexTypeRef>>, // index-paired
}
```

Rejected. Parallel-index coupling (`call_sites.len() ==
call_arg_types_apex.len()` must be maintained by every mutation) is a
desync bomb. The orchestrator merges `SyntaxResults` across files in
`TreeSitterExtractor::extract`; that merge would need to be audited
for every new field. We already had one bug in that merge loop
during TR-A.0 (`class_symbols` not being merged). Paralleling
`call_sites` with a second Vec multiplies that risk.

### The real fix: move Apex symbol types to `domain::apex::`

The actual architectural defect is that **`ApexTypeRef` /
`ApexClassSymbols` live under `syntax::language::apex::class_symbols`
when they are, by definition, Apex domain types**, not syntax
machinery. Syntax machinery is tree-sitter nodes, queries, capture
names, extractors. `ApexTypeRef` is a language-semantic lattice —
exactly the kind of thing that belongs in `domain::`.

The move is:

```
graphengine-parsing/src/syntax/language/apex/class_symbols.rs
  → graphengine-parsing/src/domain/apex/class_symbols.rs
```

Scope of the move: `ApexTypeRef`, `ApexClassSymbols`, `ApexField`,
`ApexMethod`, `ApexConstructor`, `ApexParameter`, `ApexSymbolsMap`.
Everything in that one file. Plus a thin
`pub use domain::apex::class_symbols::*;` re-export from the old path
so existing call sites (`class_symbols_extractor.rs`,
`class_registry.rs`, `containment_walker.rs`, persistence code,
tests) compile unchanged.

After the move, Option B is clean:

```rust
use crate::domain::apex::class_symbols::ApexTypeRef;

pub struct CallSite {
    pub location: Range,
    pub function_name: String,
    pub receiver_range: Option<Range>,
    /// Inferred argument types for overload dispatch. Empty for
    /// languages that don't populate it. Apex populates in TR-A.1
    /// from literal and constructor-expression arguments only;
    /// identifier/field/return-type inference lands in TR-A.3/A.4.
    pub arg_types: Vec<ApexTypeRef>,
}
```

- `application` depends on `domain`. Layer arrow correct.
- `ApexTypeRef` lives where semantic types belong.
- Future Java overload dispatch (Phase B): move Java symbol types to
  `domain::java::` analogously, add a sibling `java_arg_types` field
  or evolve to a tagged sum at that point — but only when a real
  second consumer exists. **YAGNI** on the wrapper enum today.

### Cost accounting for the move

Mechanical, pure-move refactor, no behaviour change:

- 1 file relocated (`class_symbols.rs` → `domain/apex/class_symbols.rs`)
- 1 new `mod.rs` at `domain/apex/`
- ~10–15 `use` path updates across `syntax::language::apex::*` and
  Apex tests (every file that imports `ApexTypeRef` /
  `ApexClassSymbols` today).
- A `pub use` re-export at the old location for grep-time continuity,
  **removable after this commit** — rather than leave a compatibility
  shim hanging, grep-and-update the import paths in the same commit.

Acceptance: `cargo test --workspace` passes unchanged. The
byte-identical rev-6.1 CI gate passes unchanged (no runtime behaviour
moves, no schema touch). The move is verifiable by `git mv` + `sed`
on imports.

### Ship-order recommendation

**Commit 1a** (pre-work, ≤ 30 min): move `class_symbols.rs` to
`domain::apex::`, update imports, no other changes. Pure refactor PR
or first commit inside the Commit-1 branch.

**Commit 1** (the real work): `CallSite.arg_types: Vec<ApexTypeRef>`,
R33 fix, `this`/`super` fix, constructor resolver arm, fixtures,
tests.

If splitting into two commits feels like scope noise, both can land
as sequential commits inside a single PR with clear commit messages;
reviewers can still read the pure-move commit independently.

### If the move is overruled

Fallback: **Option A** (wrapper enum) with an explicit
`#[allow(clippy::module_name_repetitions)]`-style
`// FIXME(architecture): ApexTypeRef belongs in domain::apex, remove wrapper then`
comment on the enum. Acceptable compromise, not preferred. Reject
Option B-without-the-move — shipping the layer inversion without
labelling it is the worst outcome because it hides tech debt instead
of tracking it.

---

## Q2 — Enclosing class for `__self` / `__super`: **A, with one refinement**

### Evaluating the options

**Option A (reuse `SymbolIndex::find_enclosing_type_or_function` +
`registry.symbols_for(Node.fqn)`):**

Verified in the code:

- `SymbolIndex` at
  `graphengine-parsing/src/syntax/language/apex/heuristic_resolver.rs:366–462`
  already maintains `types_by_file: HashMap<&str, Vec<&Node>>`
  populated from every type node passed into `build`.
- Every Apex type declaration (`class_declaration`,
  `interface_declaration`, `enum_declaration`,
  `trigger_declaration`) runs through `apex_fqn::build_type_fqn` at
  extraction time and emits a `Node` with `location: Range` and
  `fqn: String`.
- `ApexClassRegistry::symbols_for(api_name)` (line 364) keys
  case-insensitively on the dotted api-name tail.
- Apex type FQN format per `fqn.rs:106–123`:
  `<workspace_path>::Outer.Inner` for types,
  `<workspace_path>::Outer.Inner::method(sig)` for methods.
  Extracting the dotted tail (everything after the last `::`) and
  stripping any trailing `(sig)` gives you the registry key.

**Option B (add `declaration_range: Range` to `ApexClassSymbols`):**

Rejected. Three concrete harms:

1. `ApexClassSymbols` is **persisted to SQLite** via
   `apex_class_symbols.symbols_json`. Adding a field changes the
   on-disk payload shape. That bumps `PARSE_META_SCHEMA_VERSION` from
   2 → 3.
2. A schema bump triggers `CAVEAT_STALE_PARSE_DB_V1` for every
   pre-Commit-1 parse DB in the wild — which is fine semantically
   but re-runs the entire byte-identical rev-6.1 regression gate
   (§2.5 A.0.8 in `PHASE_A_EXECUTION_PLAN`). That gate is the most
   expensive acceptance clause we have; re-opening it for data that
   is reconstructible in-memory is unjustified.
3. `Range` inside `ApexClassSymbols` is redundant data. The same
   location is already on the `Node` in `SymbolIndex::types_by_file`.
   Duplicating it in a second store (serialised into the parse DB)
   invites drift: two sources of truth for "where is class X
   declared."

Hard no on B.

**Option C (non-persisted per-file range table on
`ApexClassRegistry`):**

Rejected as redundant. `SymbolIndex::types_by_file` is already
exactly this: a per-file `Vec<&Node>` keyed by file, with
`node.location: Range`. Building a second parallel index on
`ApexClassRegistry` means:

- Two indexes holding overlapping information, populated at different
  pipeline points (SymbolIndex is built inside
  `ApexHeuristicResolver::build`; the registry is populated earlier
  during parse).
- The registry would need additional plumbing to receive per-class
  ranges, which means the extractor has to pass range data into
  `attach_symbols`. That's a registry API change just to avoid using
  an index we already have.

### The refinement A needs

`find_enclosing_type_or_function` as it stands has two features that
are **wrong** for the `__self` / `__super` use case:

1. It prefers the enclosing **function** over the enclosing **type**.
   For `this(args)` called inside a constructor, we want the
   enclosing class, not the enclosing constructor.
2. When it does return the function node, its FQN carries
   `::method(sig)` that needs stripping.

Fix: add a narrow sibling accessor that queries only
`types_by_file`:

```rust
// graphengine-parsing/src/syntax/language/apex/heuristic_resolver.rs
// alongside find_enclosing_function / find_enclosing_type_or_function

fn find_enclosing_type(&self, at: &Range) -> Option<&'a Node> {
    let types = self.types_by_file.get(at.file.as_str())?;
    let mut containing: Vec<&Node> = types
        .iter()
        .copied()
        .filter(|n| range_contains(&n.location, at))
        .collect();
    containing.sort_by_key(|n| range_span(&n.location));
    containing.into_iter().next()
}
```

Then derive the api-name from the returned node's FQN. The existing
helper `dotted_tail_lower` (`heuristic_resolver.rs:568`) already does
tail extraction for FQNs; reuse it, or inline the split since type
FQNs don't carry signature suffixes:

```rust
fn api_name_from_type_fqn(fqn: &str) -> &str {
    fqn.rsplit_once("::").map(|(_, tail)| tail).unwrap_or(fqn)
}
```

Full resolution path for `__self::new` / `__super::new`:

```rust
// Inside resolve_constructor_call
let (target_class, target_symbols) = match name {
    "__self" => {
        let enclosing_type = self.symbol_index.find_enclosing_type(&call_site.location)?;
        let api = api_name_from_type_fqn(&enclosing_type.fqn);
        let syms = self.class_registry.symbols_for(api)?;
        (api.to_string(), syms)
    }
    "__super" => {
        let enclosing_type = self.symbol_index.find_enclosing_type(&call_site.location)?;
        let api = api_name_from_type_fqn(&enclosing_type.fqn);
        let self_syms = self.class_registry.symbols_for(api)?;
        let parent = self_syms.parent_class.as_deref()?;
        let parent_syms = self.class_registry.symbols_for(parent)?;
        (parent.to_string(), parent_syms)
    }
    other => {
        // Regular "new Foo()" path — existing lookup by name.
        ...
    }
};
// Then signature-match over target_symbols.constructors.
```

### What this avoids

- **Zero TR-A.0 shape change.** `ApexClassSymbols` serialisation
  frozen, `schema_version` stays at 2, byte-identical gate not
  retriggered.
- **No redundant storage.** Single source of truth:
  `SymbolIndex::types_by_file` is already built.
- **Reuses existing infrastructure.** `symbols_for`, `parent_class`,
  `dotted_tail_lower`, `range_contains`, `range_span` — all exist.
- **~25 LOC in `heuristic_resolver.rs`**: one new
  `find_enclosing_type` method, one `api_name_from_type_fqn` helper
  (or inline), and the `match` arm in `resolve_constructor_call`.

### One caveat worth stating in the PR description

`SymbolIndex::types_by_file` picks the **innermost** containing type.
That means `this(...)` inside an inner class correctly resolves to
the inner class's own constructor, not the outer's. Good. But it
also means `super(...)` inside an inner class resolves to the
**inner class's parent**, not the outer's parent — again correct,
and worth asserting in a test fixture (one inner class with
`super(...)` to its own explicitly declared parent, to lock the
behaviour).

---

## Net effect on Commit 1 scope

Two deltas from the scope captured in
`PHASE_A_EXECUTION_PLAN.md` §3:

1. Add a **pure-move refactor commit** (or first commit in the
   Commit-1 branch) that relocates `ApexTypeRef` and friends to
   `domain::apex::`. Required for clean
   `CallSite.arg_types: Vec<ApexTypeRef>`.
2. Drop the previously proposed
   `ApexClassRegistry::enclosing_class_for(file, line)` helper.
   Replace with a **`SymbolIndex::find_enclosing_type`** sibling
   accessor in `heuristic_resolver.rs`. Registry grows nothing.

Updated file-touch list with the corrections:

| File | Change |
|---|---|
| `graphengine-parsing/src/domain/apex/class_symbols.rs` (**relocated**) | was `syntax::language::apex::class_symbols` |
| `graphengine-parsing/src/domain/apex/mod.rs` (**new**) | `pub mod class_symbols;` |
| `graphengine-parsing/src/domain/mod.rs` | `pub mod apex;` |
| ~10–15 Apex files | `use` path updates (pure mechanical) |
| `graphengine-parsing/src/application/ports.rs` | `CallSite.arg_types: Vec<ApexTypeRef>` + constructor update |
| `graphengine-parsing/src/syntax/extractors/call_site_extractor.rs` | R33 `"constructor"` arm, `"chained_ctor_keyword"` arm, `@args` dispatch hook |
| `graphengine-parsing/src/syntax/language/apex/arg_type_inferrer.rs` (**new**) | literal + ctor-expression inference |
| `graphengine-parsing/configs/apex.yaml` | `[(this) (super)] @chained_ctor_keyword` captures |
| `graphengine-parsing/src/syntax/language/apex/heuristic_resolver.rs` | `find_enclosing_type`, `resolve_constructor_call`, `__self`/`__super` expansion, exact+widening signature match |
| `graphengine-parsing/src/syntax/language/apex/resolver_dispatch.rs` | route constructor call sites to new arm |
| `graphengine-parsing/tests/extractor_constructor_fixtures.rs` (**new**) | 5 per-language + 2 Apex (`this`/`super`) |
| `graphengine-parsing/tests/fixtures/apex_resolver/r23_a*_*.cls` | §3.2 fixtures with literal discriminators |
| `graphengine-parsing/tests/apex_resolver_r23_ctor_fixtures.rs` (**new**) | fixture driver |

The domain-move adds maybe 30 minutes of import-chasing. Everything
else is unchanged from the original Commit 1 sizing.
