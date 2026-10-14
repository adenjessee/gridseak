# Python answer-key oracle and runtime witness

Checks `gridseak audit` answer keys on Python repos the way `go-oracle/` and
`ts-oracle/` do for Go and TypeScript. Reproduce with `run.sh` (usage in the
file header).

| File | What it is |
|---|---|
| `mypy_oracle.py` | Static oracle: every direct call of an in-repo function, method or class, from mypy (Jedi for calls mypy leaves untyped). |
| `runtime_witness.py` | pytest plugin: records caller → callee pairs while the repo's own tests run (`sys.monitoring`, Python ≥ 3.12). |
| `runtime_check.py` | Compares GridSeak's call edges with the runtime pairs the static rules count, and checks planned keys for runtime-only callers. |
| `../py_audit_keys.py` | Key checker (like `../ts_audit_keys.py`): every target the audit could plan, key vs. oracle closure, plus a direct-edge diff. |

## Why mypy (and not Pyright)

GridSeak's Python index comes from scip-python, a fork of Pyright. An oracle
on Pyright (its language server's call hierarchy) would share Pyright's
binding and inference, so agreement would prove little. The options:

- **mypy** (chosen): a separate implementation of semantic analysis and type
  checking, accurate on annotated code. `mypy.build` with `preserve_asts` and
  `export_types` gives the bound declaration of every name and the type of
  every receiver; the oracle resolves `obj.m` by looking `m` up along the
  receiver type's MRO itself.
- **Jedi**: independent too, but inference-based and weaker on typed code.
  Used only as a fallback for calls mypy types `Any`: mypy does not infer
  return types of unannotated functions or pytest fixture parameters, Jedi
  does (fixtures included), so the oracle sees *more* untyped calls than
  GridSeak, which is what exposes GridSeak's untyped-call gap.

Independence caveats, stated plainly:

- The *rules* are shared: both sides implement the same definitions (below);
  a definitional mistake would show in neither.
- mypy and Pyright read the same annotations and typeshed stubs; a wrong
  annotation misleads both. The runtime witness guards against that.
- Pyright infers more than mypy in untyped code (return types of unannotated
  functions, unannotated parameters of an override from the base method,
  narrowing on attribute assignment). Where Jedi does not fill the gap, those
  appear as GridSeak-only edges; the runtime witness confirms them.
- mypy analyses each platform separately and skips unreachable code; the
  oracle unions `sys.platform` = host and `win32`.

## Rules (what counts as a direct call)

Same as Go/TypeScript: direct calls of named functions and methods count;
calls through an abstract method (`@abstractmethod`, `Protocol`) are
dispatch; a function used as a value is not a call; calls inside a lambda or
comprehension count for the enclosing named function. Python specifics:

- `C(...)` calls the `__init__` (or `__new__`) found first along C's MRO; none
  when an external base or a generated `__init__` (dataclass, attrs) comes
  first. `E(value)` on an `Enum` looks a member up and calls neither (the
  runtime witness showed `codes(404)` never runs `codes.__new__`).
  `super().m(...)` calls the base class's `m`.
- `obj.m(...)` calls the `m` obj's declared or inferred type defines or
  inherits, not overrides in subclasses (runtime dispatch to an override is
  reported, not counted).
- A nested `def` is a named function. Decorator expressions and default
  values belong to the scope around the `def`. `@d(...)` calls `d`; a bare
  `@d` does not.
- `@overload` stubs are never called (the implementation is); property
  setters are not called by `obj.p(...)`.
- A name defined in several branches (`if WIN: def f ... else: def f ...`,
  `try: from x import f except ImportError: def f ...`) calls each
  definition whose branch is compatible with the call site.
- Not calls: reading a `@property`, `with`, iteration, operators, `__call__`
  on instances, callbacks passed as values.

## The untyped-call guard

A call on a receiver typed `Any` (an unannotated parameter such as a pytest
fixture: `runner.invoke(...)`) has no compiler edge, yet may call any
function of that name. GridSeak now records such calls (no index occurrence
on the callee name, or a member call bound to a builtin) in scan metadata
`resolution_compiler_unresolved_calls`, and `gridseak audit` does not ask
about a target whose key contains a function of one of those names. The
runtime witness shows this is necessary: on click, 353 executed test → library
calls go through fixtures and are invisible to any static engine but Jedi.

## Results (pinned corpora, scip-python 0.6.6, mypy 2.4.0 pure-Python, Jedi 0.20.1, Python 3.12)

| Repo @ commit | Eligible targets: exact vs. oracle | Planned tasks exact | Runtime: countable edges hit (excl. dispatch, untyped) |
|---|---:|---:|---:|
| pallets/click @ `2247b35` | 53/53 (was 28/155) | 6/6 (was 2/6) | 2112/2115 |
| pallets/flask @ `d086db8` | 20/20 (was 18/47) | 6/6 (was 4/6) | 621/623 |
| encode/httpx @ `b5addb6` | 14/14 (was 19/74) | 3/3 (was 2/6) | 1645/1649 |

"Was" is the engine before this change, checked against the same oracle with
the old eligibility rules. Every remaining runtime miss is a call GridSeak
records as unresolved, so no planned key depends on it.
