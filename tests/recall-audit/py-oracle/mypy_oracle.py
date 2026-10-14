"""Static answer-key oracle for `gridseak audit` on Python, built on mypy's
semantic analyser and type checker (an engine independent of Pyright, which
scip-python and therefore GridSeak's index are built on).

usage: mypy_oracle.py <repo-root> <repo-venv-python> <out.json> [--platform P]... [--no-jedi]

Walks every in-repo `.py` file and records each direct call of an in-repo
function, method or class:

  kind=call   `f(...)`, `obj.m(...)`, `Cls.m(...)` where mypy binds the callee
              name to a function/method declaration (receiver types come from
              mypy's type map: instance, class object, union, type variable bound)
  kind=init   `C(...)` where `C` names a class: the `__init__`/`__new__` mypy
              uses to type the call (C's own or the nearest inherited one)
  kind=super  `super().m(...)`
  kind=prop   `obj.p(...)` where `p` is a @property (reading the getter)

Callers are the enclosing named `def` (lambdas and comprehensions credit the
enclosing named function; decorator expressions and default values belong to
the scope around the `def`; module and class-body code have no caller).
`via_iface` marks calls of abstract or Protocol methods (dispatch); `union`
marks calls whose receiver is a union reaching several methods. A name defined
more than once in one scope (platform branches, `try`/`except` fallbacks,
redefinitions) calls every definition whose `if`/`try` branch is compatible
with the call site; mypy itself binds only the first.

Calls mypy cannot resolve (receiver typed `Any`: unannotated parameters such as
pytest fixtures, results of unannotated functions) are handed to Jedi, a third
engine with its own inference (return types of unannotated functions, pytest
fixtures); its rows carry resolver=jedi. What neither resolves is written to
`unresolved` with the called name.

`--platform` (repeatable, default: host and win32) re-runs the analysis per
`sys.platform`, because mypy skips code it proves unreachable; edges are the
union. Files with clashing module names are analysed in separate builds.
"""
import json
import os
import sys
from collections import defaultdict

from mypy import build
from mypy.modulefinder import BuildSource
from mypy.nodes import (
    IS_ABSTRACT,
    IMPLICITLY_ABSTRACT,
    Decorator,
    FuncDef,
    LambdaExpr,
    MemberExpr,
    NameExpr,
    OverloadedFuncDef,
    RefExpr,
    SuperExpr,
    TypeInfo,
    Var,
)
from mypy.options import Options
from mypy.traverser import TraverserVisitor
from mypy.types import (
    AnyType,
    CallableType,
    Instance,
    LiteralType,
    Overloaded,
    TupleType,
    TypedDictType,
    TypeType,
    TypeVarType,
    UnionType,
    get_proper_type,
)

SKIP_DIRS = {"node_modules", "venv", "build", "dist", "site-packages", "__pycache__"}


def repo_files(root):
    out = []
    for d, dirs, files in os.walk(root):
        dirs[:] = sorted(x for x in dirs if not x.startswith(".") and x not in SKIP_DIRS)
        out += [os.path.join(d, f) for f in sorted(files) if f.endswith(".py")]
    return out


def module_name(path):
    """Module name by the import rules: climb while the directory is a package."""
    d, base = os.path.split(path)
    parts = [base[:-3]]
    # A directory without __init__.py inside a package is a namespace subpackage.
    while os.path.exists(os.path.join(d, "__init__.py")) or os.path.exists(
            os.path.join(os.path.dirname(d), "__init__.py")):
        d, pkg = os.path.split(d)
        parts.append(pkg)
    if parts[0] == "__init__":
        parts = parts[1:]
    return ".".join(reversed(parts)), d


def plan_builds(files):
    """Split files into builds where module names are unique (mypy refuses duplicates)."""
    builds = []
    for f in files:
        mod, base = module_name(f)
        for b in builds:
            if mod not in b["mods"]:
                break
        else:
            b = {"mods": {}, "bases": set()}
            builds.append(b)
        b["mods"][mod] = f
        b["bases"].add(base)
    return builds


def member_nodes(t, name):
    """SymbolTableNode.node for `name` on a receiver of type t (MRO lookup); 'ANY' if unknown."""
    t = get_proper_type(t)
    if t is None or isinstance(t, AnyType):
        return ["ANY"]
    if isinstance(t, TupleType):
        return member_nodes(t.partial_fallback, name)
    if isinstance(t, (LiteralType, TypedDictType)):
        return member_nodes(t.fallback, name)
    if isinstance(t, TypeVarType):
        return member_nodes(t.upper_bound, name)
    if isinstance(t, UnionType):
        out = []
        for it in t.items:
            out += member_nodes(it, name)
        return out
    info = None
    if isinstance(t, Instance):
        info = t.type
    elif isinstance(t, TypeType) and isinstance(get_proper_type(t.item), Instance):
        info = get_proper_type(t.item).type
    elif isinstance(t, (CallableType, Overloaded)) and t.is_type_obj():
        info = t.type_object()
    if info is None:
        return []
    sym = info.get(name)
    return [sym.node] if sym and sym.node is not None else []


def _info(f):
    from mypy.nodes import FakeInfo
    i = f.info
    return None if isinstance(i, FakeInfo) else i


def func_of(node):
    if isinstance(node, Decorator):
        return node.func, ("prop" if node.var.is_property else None)
    if isinstance(node, OverloadedFuncDef):
        impl = node.impl
        if isinstance(impl, Decorator):
            impl = impl.func
        if impl is None and node.items:
            it = node.items[0]
            impl = it.func if isinstance(it, Decorator) else it
        return impl, ("prop" if node.is_property else None)
    if isinstance(node, FuncDef):
        return node, None
    return None, None


class Scoped(TraverserVisitor):
    """Tracks the enclosing scope (module, class or def) and the `if`/`try`
    branches around the current node: two places run in one execution unless
    some `if`/`try` around both puts them in different branches."""

    def __init__(self):
        super().__init__()
        self.scopes = []
        self.branch = ()

    def _in(self, key, node):
        saved = self.branch
        self.branch = saved + (key,)
        node.accept(self)
        self.branch = saved

    def visit_mypy_file(self, f):
        self.scopes.append(f)
        super().visit_mypy_file(f)
        self.scopes.pop()

    def visit_class_def(self, c):
        for d in c.decorators:
            d.accept(self)
        self.scopes.append(c)
        c.defs.accept(self)
        self.scopes.pop()

    def visit_if_stmt(self, s):
        for i, (cond, body) in enumerate(zip(s.expr, s.body)):
            cond.accept(self)
            self._in((id(s), i), body)
        if s.else_body is not None:
            self._in((id(s), len(s.body)), s.else_body)

    def visit_try_stmt(self, s):
        self._in((id(s), 0), s.body)
        for i, h in enumerate(s.handlers):
            self._in((id(s), i + 1), h)
        if s.else_body is not None:
            self._in((id(s), 0), s.else_body)
        if s.finally_body is not None:
            s.finally_body.accept(self)


def compatible(a, b):
    ids = dict(a)
    return all(ids.get(k, v) == v for k, v in b)


class Collector(Scoped):
    def __init__(self, ctx, relfile):
        super().__init__()
        self.ctx = ctx
        self.rel = relfile
        self.stack = []

    # --- scopes -----------------------------------------------------------
    def visit_func_def(self, d):
        for a in d.arguments:
            if a.initializer is not None:
                a.initializer.accept(self)
        self.stack.append(d)
        self.scopes.append(d)
        d.body.accept(self)
        self.scopes.pop()
        self.stack.pop()

    def visit_decorator(self, dec):
        for e in dec.decorators:
            e.accept(self)
        dec.func.accept(self)

    def visit_overloaded_func_def(self, o):
        for it in o.items:
            it.accept(self)
        if o.impl is not None:
            o.impl.accept(self)

    def visit_lambda_expr(self, e):
        for a in e.arguments:
            if a.initializer is not None:
                a.initializer.accept(self)
        e.body.accept(self)

    # --- calls ------------------------------------------------------------
    def visit_call_expr(self, e):
        self.record(e)
        super().visit_call_expr(e)

    def self_class(self, expr):
        """Class of `self`/`cls` when `expr` names a method's first parameter.
        mypy keeps no types for bodies it checks once per value of a
        constrained type variable (`AnyStr`), so `self.m()` there is resolved
        from the enclosing class."""
        if not isinstance(expr, NameExpr):
            return None
        for f in reversed(self.stack):
            info = _info(f)
            if info is not None and f.arguments and f.arguments[0].variable is expr.node and not f.is_static:
                return info
        return None

    def variants(self, f):
        """`f` plus same-named functions declared in the same scope (platform
        branches, redefinitions) that could be bound here."""
        sibs = self.ctx["scope_funcs"].get(self.ctx["scope_of"].get(id(f)), [])
        if len(sibs) < 2:
            return [f]
        out = []
        for g, rel, branch in sibs:
            if rel != self.rel or compatible(self.branch, branch):
                out.append(g)
        return out or [f]

    def record(self, e):
        if not self.stack:
            return
        caller = self.stack[-1]
        c = e.callee
        types = self.ctx["types"]
        targets = []  # (FuncDef, kind)
        union = False
        if isinstance(c, SuperExpr):
            t = get_proper_type(types.get(c))
            d = getattr(t, "definition", None)
            if isinstance(d, FuncDef):
                targets.append((d, "super"))
            elif c.info is not None:
                for base in c.info.mro[1:]:
                    sym = base.names.get(c.name)
                    if sym is not None:
                        f, _ = func_of(sym.node)
                        if f is not None:
                            targets.append((f, "super"))
                        break
        elif isinstance(c, RefExpr):
            node = c.node
            if isinstance(c, MemberExpr) and (node is None or (isinstance(node, Var) and not node.is_property and c.kind is None)):
                if c.expr in types and types[c.expr] is not None:
                    nodes = member_nodes(types.get(c.expr), c.name)
                    # a receiver of union type reaching several methods
                    union = len({id(x) for x in nodes if x not in (None, "ANY")}) > 1
                else:
                    info = self.self_class(c.expr)
                    sym = info.get(c.name) if info is not None else None
                    nodes = [sym.node] if sym is not None and sym.node is not None else (["ANY"] if info is None else [])
            elif isinstance(node, Var):
                # `f = None` then `def f(...)` in another branch: mypy binds the
                # name to the variable but types it as the function.
                t = get_proper_type(types.get(c))
                d = getattr(t, "definition", None)
                nodes = [d] if isinstance(d, FuncDef) and d.name == node.name else []
            else:
                nodes = [node]
            for n in nodes:
                if n == "ANY" or n is None:
                    if n == "ANY" or isinstance(c, NameExpr):
                        if isinstance(c, MemberExpr) and c.end_line is not None and c.end_column is not None:
                            at = (c.end_line, c.end_column - len(c.name))
                        else:
                            at = (c.line, c.column)
                        self.ctx["unresolved"].append({
                            "file": self.rel, "line": e.line, "name": c.name,
                            "name_line": at[0], "name_col": at[1], "ref_col": e.column,
                            "caller_file": self.rel, "caller_line": caller.line, "caller_name": caller.name,
                        })
                    continue
                if isinstance(n, TypeInfo):
                    if n.is_enum:
                        continue  # `E(value)` looks a member up; no __init__/__new__ runs
                    t = get_proper_type(types.get(c))
                    d = None
                    if isinstance(t, CallableType):
                        d = t.definition
                    elif isinstance(t, Overloaded) and t.items:
                        d = t.items[0].definition
                    if isinstance(d, FuncDef) and d.name in ("__init__", "__new__"):
                        targets.append((d, "init"))
                    continue
                f, special = func_of(n)
                if f is not None:
                    for g in self.variants(f):
                        targets.append((g, special or "call"))
        for f, kind in targets:
            callee = self.ctx["defs"].get(id(f))
            if callee is None:
                continue
            iface = f.abstract_status in (IS_ABSTRACT, IMPLICITLY_ABSTRACT) or (_info(f) is not None and _info(f).is_protocol)
            self.ctx["rows"].append({
                "ref_file": self.rel, "ref_line": e.line, "ref_col": e.column,
                "callee_file": callee[0], "callee_line": callee[1], "callee_name": callee[2],
                "kind": kind, "via_iface": bool(iface), "union": union,
                "caller_file": self.rel, "caller_line": caller.line, "caller_name": caller.name,
            })


class DefCollector(Scoped):
    """Every in-repo `def`: identity, and its scope and branch so redefinitions
    of one name in one scope can be grouped."""

    def __init__(self, rel, ctx):
        super().__init__()
        self.rel, self.ctx = rel, ctx

    def visit_overloaded_func_def(self, o):
        # Overload items and property accessors are one function to callers.
        for it in o.items:
            self._def(it.func if isinstance(it, Decorator) else it, group=False)
        if o.impl is not None:
            self._def(o.impl.func if isinstance(o.impl, Decorator) else o.impl, group=False)

    def visit_decorator(self, dec):
        for e in dec.decorators:
            e.accept(self)
        self._def(dec.func)

    def visit_func_def(self, d):
        self._def(d)

    def visit_class_def(self, c):
        from mypy.nodes import FakeInfo
        if not isinstance(c.info, FakeInfo):
            self.ctx["classes"][(self.rel, c.line)] = c.info
        super().visit_class_def(c)

    def _def(self, d, group=True):
        self.ctx["defs"][id(d)] = (self.rel, d.line, d.name)
        if group:
            key = (id(self.scopes[-1]), d.name)
            self.ctx["scope_of"][id(d)] = key
            self.ctx["scope_funcs"].setdefault(key, []).append((d, self.rel, self.branch))
        self.scopes.append(d)
        for a in d.arguments:
            if a.initializer is not None:
                a.initializer.accept(self)
        d.body.accept(self)
        self.scopes.pop()


def run_build(root, py, files_by_mod, bases, platform):
    o = Options()
    o.check_untyped_defs = True
    o.preserve_asts = True
    o.export_types = True
    o.incremental = False
    o.follow_imports = "normal"
    o.ignore_missing_imports = True
    o.python_executable = py
    o.platform = platform
    o.mypy_path = sorted(bases)
    o.show_traceback = True
    sources = [BuildSource(f, m) for m, f in files_by_mod.items()]
    res = build.build(sources, o)
    rels = {}
    for mod, tree in res.files.items():
        p = os.path.realpath(tree.path) if tree.path else ""
        if p.startswith(root):
            rels[mod] = p[len(root):]
    ctx = {"types": res.types, "defs": {}, "scope_of": {}, "scope_funcs": {}, "rows": [], "unresolved": [], "classes": {}}
    for mod, rel in rels.items():
        res.files[mod].accept(DefCollector(rel, ctx))
    # `C(...)` runs the first __init__/__new__ along C's MRO; none in the repo
    # when an external class provides it first.
    ctx["class_init"] = {}
    for key, info in ctx["classes"].items():
        target = None
        for base in ([] if info.is_enum else info.mro):
            sym = base.names.get("__init__") or base.names.get("__new__")
            if sym is not None:
                f, _ = func_of(sym.node)
                target = ctx["defs"].get(id(f)) if f is not None else None
                break
        ctx["class_init"][key] = target
    for mod in files_by_mod:
        if mod in rels:
            res.files[mod].accept(Collector(ctx, rels[mod]))
    return ctx, len(res.errors)


def jedi_fallback(root, py, unresolved, class_init):
    """Resolve the calls mypy left untyped with Jedi, a third engine (its own
    inference, including return types of unannotated functions and pytest
    fixtures). Rows are marked resolver=jedi."""
    try:
        import jedi
    except ImportError:
        print("jedi not installed: no fallback for untyped calls", file=sys.stderr)
        return [], unresolved
    project = jedi.Project(root, environment_path=py)
    by_file = defaultdict(list)
    for u in unresolved:
        by_file[u["file"]].append(u)
    rows, still = [], []
    for rel, us in sorted(by_file.items()):
        code = open(root + rel, encoding="utf-8").read()
        for u in us:
            try:
                # A fresh Script per query: a reused one stops resolving pytest
                # fixtures after its first goto (jedi 0.20). The cursor goes
                # inside the name; before its first letter Jedi finds nothing.
                script = jedi.Script(code, path=root + rel, project=project)
                names = script.goto(u["name_line"], u["name_col"] + (1 if len(u["name"]) > 1 else 0), follow_imports=True)
            except Exception:
                names = []
            hit = False
            for n in names:
                mp = str(n.module_path or "")
                if not mp.startswith(root):
                    continue
                nrel = mp[len(root):]
                callee, kind = None, "call"
                if n.type == "function":
                    callee = (nrel, n.line, n.name)
                elif n.type == "class":
                    callee, kind = class_init.get((nrel, n.line)), "init"
                if callee is None:
                    continue
                hit = True
                rows.append({
                    "ref_file": rel, "ref_line": u["line"], "ref_col": u["ref_col"],
                    "callee_file": callee[0], "callee_line": callee[1], "callee_name": callee[2],
                    "kind": kind, "via_iface": False, "resolver": "jedi",
                    "caller_file": u["caller_file"], "caller_line": u["caller_line"], "caller_name": u["caller_name"],
                })
            if not hit:
                still.append(u)
    return rows, still


def main():
    args = sys.argv[1:]
    platforms = []
    while "--platform" in args:
        i = args.index("--platform")
        platforms.append(args[i + 1])
        del args[i:i + 2]
    root = os.path.realpath(args[0]) + os.sep
    py, outp = args[1], args[2]
    platforms = platforms or [sys.platform, "win32"]
    files = repo_files(root)
    builds = plan_builds(files)
    # Every build can import every package root (examples import their own
    # top-level packages, e.g. `flaskr`).
    all_bases = set().union(*(b["bases"] for b in builds))
    rows, unresolved, seen, seen_u = [], [], set(), set()
    class_init = {}
    for plat in platforms:
        for b in builds:
            ctx, nerr = run_build(root, py, b["mods"], all_bases, plat)
            print(f"[{plat}] build of {len(b['mods'])} modules: {len(ctx['rows'])} call rows, {nerr} mypy messages", file=sys.stderr)
            class_init.update(ctx["class_init"])
            for r in ctx["rows"]:
                k = (r["ref_file"], r["ref_line"], r["ref_col"], r["callee_file"], r["callee_line"], r["kind"])
                if k not in seen:
                    seen.add(k)
                    r["platform"] = plat
                    rows.append(r)
            for u in ctx["unresolved"]:
                k = (u["file"], u["line"], u["name"])
                if k not in seen_u:
                    seen_u.add(k)
                    unresolved.append(u)
    # Untyped on one platform but resolved on another: resolved.
    resolved_at = {(r["ref_file"], r["ref_line"], r["callee_name"]) for r in rows}
    unresolved = [u for u in unresolved if (u["file"], u["line"], u["name"]) not in resolved_at]
    for r in rows:
        r.setdefault("resolver", "mypy")
    if "--no-jedi" not in sys.argv:
        jrows, unresolved = jedi_fallback(root, py, unresolved, class_init)
        print(f"jedi resolved {len(jrows)} rows for calls mypy left untyped", file=sys.stderr)
        rows += jrows
    with open(outp, "w") as fh:
        json.dump({"rows": rows, "unresolved": unresolved, "files": [f[len(root):] for f in files]}, fh)
    print(f"{len(rows)} call rows, {len(unresolved)} unresolved call sites -> {outp}", file=sys.stderr)


if __name__ == "__main__":
    main()
