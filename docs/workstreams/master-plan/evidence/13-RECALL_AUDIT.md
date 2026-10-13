# 13 — Caller-recall audit (compiler tier vs. independent type checkers)

Date: 2026-10-10/11. Reproduce: `tests/recall-audit/run.sh` (writes `target/recall-audit/`).

## Question

Before publishing any agent-breakage number, the gate's own ground truth has
to be right: when GridSeak says a symbol has callers (or has none), is that
true? The bar set in the GTM plan is ≥ 98% caller recall.

## Method

Oracles that never read SCIP or GridSeak output:

- **Go (chi @ `71307f9`)** — `tests/recall-audit/go-oracle`: `go/types` over
  `./...` with tests; every use of an in-repo `*types.Func` inside a function
  body, split into direct calls, interface-method calls and function values.
- **TypeScript (zod @ `e308703`, `src/`)** — `tests/recall-audit/ts-oracle`:
  the TypeScript checker; every identifier resolving (through aliases) to an
  in-repo function, method, accessor, constructor or function-valued const.

GridSeak edges are matched on (enclosing function → defining function).

## Results

| Metric | chi before | chi after | zod before | zod after |
|---|---:|---:|---:|---:|
| Direct-call edge recall | 0.828 | **1.000** | 0.889 | **0.997** |
| Function-value / getter use recall | 0.095 | **1.000** | 0.135 | **0.998** |
| Symbols with users flagged as having callers | 0.879 | **1.000** | 0.498 | **0.988** |
| Symbols with *every* caller listed | 0.788 | **1.000** | 0.495 | **0.985** |
| Compiler edges absent from oracle | 67 | 27 (all interface dispatch) | 574 | 544 (515 same-name dispatch/overload, 29 `new X()` / re-export) |
| Heuristic (TreeSitter) Call edges | 140, all false | 0 | 0 | 0 |

"Before" is the pre-audit engine (`ffde7be`); "after" is this change.

## What was wrong

1. **Caret mapping, not the compiler, decided callers.** Each tree-sitter call
   site was mapped to one SCIP occurrence by caret position, then to a node by
   fuzzy name/line search. Misplaced carets produced wrong-callee edges
   (e.g. `TestStripSlashes → TestBasicWritesTeesWritesWithoutDiscard`), and
   non-call uses (function values, getters) were never seen.
   Fix: `IndexBackedSemanticResolver` now sweeps every SCIP reference
   occurrence (`ScipIndex::reference_occurrences`) and maps it strictly:
   enclosing function → the node that contains the definition *and* has its
   name (`SymbolIndex::find_definition_node`). Indexed files no longer fall
   back to name-matching at all.
2. **SCIP descriptor parsing.** `util/objectValues.` (namespace-nested terms),
   `` `<get>shape`(). `` (accessors) and `` `<constructor>`(). `` were mangled
   into file names. Fix: `last_descriptor_name` parses the SCIP descriptor
   grammar; parameters `(p)` / type parameters `[T]` are never callees.
3. **Document-local symbols.** `local N` monikers collide across files and were
   dropped; locally declared helpers (`const customMap = …`) had no callers.
   Fix: locals are paired within their own document and named from source.

## Not covered

- Interface/abstract dispatch is reported as may-call edges to every
  implementation (Medium confidence when > 1). The oracle records the abstract
  declaration instead, so these appear as "extra" rather than "recall".
- Python, Java, C#, Rust and Apex were not audited here.
- Uses outside any function body (module top level) are attributed to the
  file-scope node.
