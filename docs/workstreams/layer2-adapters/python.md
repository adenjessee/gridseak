# WS-LAYER2-PYTHON — Python Layer-2 adapter

> **Kind.** Design stub.
> **Parent.** [`README.md`](README.md) (WS-LAYER2-ADAPTERS index).
> **Status.** Unstaffed *on the Layer-2 axis*. The Python heuristic
> extractor ships today
> (`graphengine-parsing/src/syntax/language/extractors/python.rs`).
> Scans on Python repos produce a full call graph and every
> graph-level metric; this stub tracks the unshipped
> semantic-confirmation layer. Per-edge precision on heuristic-only
> Python is unmeasured — there are no dedicated `python_*.rs`
> integration tests yet (the ground-truth gap noted in
> `TRUST_AND_ACCURACY_MEMO.md §4`).
> **Target resolver.** [pyright](https://github.com/microsoft/pyright) — Microsoft's static type checker with LSP surface.

## Why pyright, not alternatives

- **jedi** (used by Jedi-language-server) is simpler but weaker
  on type inference; it frequently emits `None` for typed calls
  where pyright resolves correctly. We want maximum Layer-2
  signal, not lowest latency.
- **mypy** has the strongest type inference but no LSP surface
  in a usable form. We would have to build one; that is
  explicitly the anti-goal of this axis.
- **PyCharm's resolver** is closed-source; non-starter.

## Expected install friction

- `npm install -g pyright` or `pip install pyright` (both ship
  the same binary via npm). Node required either way.
- pyright expects a `pyrightconfig.json` or a `pyproject.toml`
  `[tool.pyright]` section. Projects without either fall back to
  reasonable defaults but the adapter should emit a warning:
  *"pyright configuration not found; type-stub inference may be
  incomplete."*
- Virtualenvs: the adapter must pass `--pythonversion` and
  `--pythonpath` to pyright matching the project's venv, or
  pyright resolves against its bundled type stubs rather than
  the project's actual dependencies.

## Adapter contract specifics

- Must resolve `textDocument/definition` on a call site.
- Must handle **duck-typed dispatch** — a call on an object whose
  type pyright infers as `Protocol[…]` should resolve to every
  concrete implementation of that protocol.
- Must handle **decorator-wrapped functions** — `@functools.wraps`,
  `@dataclass`, `@contextmanager`. The decorator layer sometimes
  obscures the call-target; pyright usually resolves through
  these.
- Must NOT resolve through `__getattr__` / `__getattribute__`
  proxies — those emit `framework_annotation_unresolved`.

## Coverage-gap counterparts (Axis 2)

- **Decorator bodies** where the decorator is a user-defined
  function (not `@dataclass` / `@functools.*`).
- **Metaclass `__call__`** — classes whose metaclass overrides
  `__call__` can dispatch to methods pyright does not see.
- **`exec`/`eval`** — always a `CoverageGap::DynamicEval`.
- **`__getattr__`** proxies (Django `settings.__getattr__`,
  SQLAlchemy dynamic-attribute models).
- **Monkey-patching** (`some_module.some_function = my_func`) —
  pyright does not track mutations to imported modules.

## Readiness strategy

pyright's cold-start is fast (<5 s on small projects, <30 s on
large). The adapter should:

1. Start pyright in watch mode.
2. Wait for the initial `workspace/configuration` + first
   `textDocument/publishDiagnostics` pair on a canary file.
3. Probe `definition` on a canary symbol; on success, mark
   ready.

Timeout: 120 s on a 10k-file project.

## Benchmarks to target

- **Accuracy.** At least 50 % of call edges Layer-2 confirmed on
  a typed Python 3.10+ project. Lower on untyped / duck-typed
  projects (30 %).
- **Cold-start.** < 30 s readiness on 10k-file project.

## Out of scope (v0)

- Python 2 codebases. pyright supports; we won't v0.
- **Cython / C-extension** call resolution. Those are FFI
  boundaries — emit `extern_boundary`, do not attempt to resolve
  into the C side.
- Jupyter notebooks. Different execution model; separate path.

## Overlooked risks

- **Untyped Python is the common case.** If the customer's
  codebase is heavily untyped, pyright will produce a lot of
  `Unknown` resolutions. The adapter must emit a quality
  statistic alongside each scan: *"35 % of calls in this codebase
  were against typed receivers; 65 % were against `Any` / `Unknown`
  — the Layer-2 confirmed share is bounded by the first number."*
- **`__init__.py` vs namespace packages.** Python's two packaging
  styles both exist in the wild; pyright handles them but we must
  pick up both during corpus walk and pass them to pyright in
  the right shape.
- **Stub-only targets (`.pyi`).** A call that resolves only into
  a `.pyi` declaration is an external-boundary call; tag as such,
  not as first-party Layer-2.
