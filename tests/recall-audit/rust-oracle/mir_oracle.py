#!/usr/bin/env python3
"""Rust call-graph oracle from rustc's own MIR (independent of rust-analyzer).

For every workspace target, runs

    RUSTC_BOOTSTRAP=1 cargo rustc -p PKG --<kind> NAME --profile test -- \
        -Zunpretty=mir -Zmir-include-spans=yes -Ztrim-diagnostic-paths=false

and reads, per MIR body, every `Call` terminator and every function-item
constant used as a value. Names in MIR are resolved by rustc's type checker
(method calls, UFCS, `<T as Trait>::m`, trait objects); this script only maps
rustc's def paths back to source declarations.

Output: JSON list of references in the go-oracle shape
(`caller_file/caller_line/callee_file/callee_line/is_call/via_iface`), lines
being the line of the `fn` keyword of each declaration, files relative to the
workspace root.

Classification:
  * `<dyn Tr as Tr>::m`, `<T as Tr>::m` (T a type parameter) and
    `<Self as Tr>::m` (in a trait default body) are dispatch: one reference
    per in-workspace implementation of `Tr::m` (plus the trait's default
    body), `via_iface = true`.
  * `<Ty as Tr>::m` with a concrete `Ty` is a direct call to `Ty`'s impl of
    `m` (or the trait default when the impl does not override it).
  * Closure and async bodies (`f::{closure#0}`) are credited to `f`.
  * Derive-generated and macro-generated bodies have no `fn NAME` in source
    and are dropped as callers and callees.

Usage: mir_oracle.py WORKSPACE_ROOT [--skip PKG ...] [--target-dir DIR] [--all-features]
"""
import json
import os
import re
import subprocess
import sys
from collections import defaultdict

SPAN = r"(?P<file>[^:\s][^:]*):(?P<l1>\d+):(?P<c1>\d+): (?P<l2>\d+):(?P<c2>\d+)"
HEADER = re.compile(r"^fn (?P<path>.+?)\((?P<rest>.*)$")
RET_PLACE = re.compile(r"// return place in scope \d+ at " + SPAN)
TERMINATOR = re.compile(r"^\s+(?P<call>.+?) -> (?:\[return|unwind)")
CONST_FNDEF = re.compile(r"\+ const_: Const \{ ty: .*\{(?P<path>[^{}]+)\}, val: Value\((?P<val>.+)\) \}$")
# Extra `cargo rustc` flags (`--all-features`).
EXTRA_CARGO_ARGS = []
IMPL_AT = re.compile(r"<impl at " + SPAN + r">")
# `<T as std::clone::Clone>::clone`: dispatch through a standard-library trait.
STD_TRAIT = re.compile(r" as (?:std|core|alloc)::[^>]*>::")


def split_top(s, sep):
    """Split on `sep` at bracket depth 0 (`<>`, `()`, `[]`, `{}`)."""
    out, depth, cur, i = [], 0, "", 0
    while i < len(s):
        c = s[i]
        if depth == 0 and s.startswith(sep, i):
            out.append(cur)
            cur = ""
            i += len(sep)
            continue
        if c in "<([{":
            depth += 1
        elif c in ">)]}" and not (c == ">" and i > 0 and s[i - 1] == "-"):
            depth -= 1
        cur += c
        i += 1
    out.append(cur)
    return out


def strip_generics(path):
    """`a::Foo::<T>::m::<u32>` -> `a::Foo::m`; `Vec<u32>` -> `Vec`."""
    out, depth = "", 0
    for i, c in enumerate(path):
        if c == "<":
            depth += 1
            continue
        if c == ">" and not (i > 0 and path[i - 1] == "-"):
            depth -= 1
            continue
        if depth == 0:
            out += c
    return out.replace("::::", "::").rstrip(":")


def last_seg(path):
    """`&'a mut a::Foo<T>` -> `Foo`; `dyn a::Tr + Send` -> `Tr`."""
    p = strip_generics(path.strip())
    p = re.sub(r"^(?:[&*]\s*|'\w+\s+|mut\s+|const\s+|dyn\s+)+", "", p)
    p = split_top(p, " + ")[0]
    return p.split("::")[-1].strip()


class Workspace:
    def __init__(self, root):
        self.root = os.path.realpath(root)
        self.cache = {}

    def lines(self, rel):
        if rel not in self.cache:
            p = rel if os.path.isabs(rel) else os.path.join(self.root, rel)
            try:
                with open(p, encoding="utf-8", errors="replace") as fh:
                    self.cache[rel] = fh.read().split("\n")
            except OSError:
                self.cache[rel] = None
        return self.cache[rel]

    def rel(self, f):
        a = f if os.path.isabs(f) else os.path.join(self.root, f)
        a = os.path.realpath(a)
        return os.path.relpath(a, self.root) if a.startswith(self.root + os.sep) else None

    def text(self, f, l1, c1, l2, c2):
        ls = self.lines(f)
        if ls is None or l2 > len(ls):
            return None
        if l1 == l2:
            return ls[l1 - 1][c1 - 1:c2 - 1]
        parts = [ls[l1 - 1][c1 - 1:]] + ls[l1:l2 - 1] + [ls[l2 - 1][:c2 - 1]]
        return " ".join(parts)

    def decl_line(self, f, anchor, name, window=40):
        """Line of `fn NAME` at or above `anchor` (signature spans lines), or
        just below it when the anchor is an attribute macro (`#[tokio::test]`)."""
        ls = self.lines(f)
        if ls is None:
            return None
        pat = re.compile(r"\bfn\s+(r#)?" + re.escape(name) + r"\b")
        for line in range(min(anchor, len(ls)), max(0, anchor - window), -1):
            if pat.search(ls[line - 1]):
                return line
        if ls[anchor - 1].lstrip().startswith("#["):
            for line in range(anchor + 1, min(len(ls), anchor + 6) + 1):
                if pat.search(ls[line - 1]):
                    return line
        return None


def parse_impl_header(text):
    """`impl<T: X> Tr<T> for Ty<T> where ..` -> (trait_last_seg | None, type_last_seg)."""
    t = text.strip()
    if t.startswith("unsafe "):
        t = t[7:]
    if not t.startswith("impl"):
        return None
    t = t[4:]
    params = set()
    if t.startswith("<"):
        depth = 0
        for i, c in enumerate(t):
            depth += c == "<"
            depth -= c == ">"
            if depth == 0:
                for p in split_top(t[1:i], ","):
                    p = p.strip().split(":")[0].strip()
                    if p and not p.startswith("'") and not p.startswith("const "):
                        params.add(p)
                t = t[i + 1:]
                break
    t = split_top(t, " where ")[0].strip().rstrip("{").strip()
    parts = split_top(t, " for ")
    if len(parts) == 2:
        tr = parts[0].strip().lstrip("!")
        ty = last_seg(parts[1])
        # Blanket impl (`impl<R: Read> Read for &mut R`): any type.
        return (last_seg(tr), "*" if ty in params else ty)
    return (None, last_seg(parts[0]))


class Body:
    __slots__ = ("crate", "path", "name", "file", "line", "calls", "values",
                 "impl", "trait_owner")

    def __init__(self):
        self.calls, self.values = [], []
        self.impl = None          # (trait | None, type) for methods in impl blocks
        self.trait_owner = None   # trait name for default bodies in trait blocks


def run_mir(root, pkg, kind, name, target_dir):
    cmd = ["cargo", "rustc", "-q", "-p", pkg]
    if kind == "lib":
        cmd += ["--lib", "--profile", "test"]
    elif kind == "test":
        cmd += ["--test", name]
    elif kind == "bin":
        cmd += ["--bin", name, "--profile", "test"]
    elif kind == "example":
        cmd += ["--example", name]
    else:
        return None
    cmd += EXTRA_CARGO_ARGS
    cmd += ["--", "-Zunpretty=mir", "-Zmir-include-spans=yes", "-Ztrim-diagnostic-paths=false"]
    env = dict(os.environ, RUSTC_BOOTSTRAP="1", CARGO_TARGET_DIR=target_dir, CARGO_INCREMENTAL="0")
    p = subprocess.run(cmd, cwd=root, env=env, capture_output=True, text=True)
    if p.returncode != 0:
        sys.stderr.write(f"[oracle] {pkg} {kind} {name}: rustc failed\n{p.stderr[-2000:]}\n")
        return None
    return p.stdout


def bodies_from_mir(ws, crate, mir):
    bodies, cur, stmt_callee = [], None, None
    for line in mir.split("\n"):
        h = HEADER.match(line)
        if h:
            cur = Body()
            cur.crate, cur.path, cur.file, cur.line = crate, h.group("path"), None, None
            bodies.append(cur)
            stmt_callee = None
            continue
        if cur is None:
            continue
        if cur.file is None:
            m = RET_PLACE.search(line)
            if m:
                cur.file = m.group("file")
                cur.line = int(m.group("l1"))
        t = TERMINATOR.match(line)
        if t and not line.lstrip().startswith("//"):
            # Drop the destination place (`(((*_9) as variant#4).1: T) = f(..)`).
            call = split_top(t.group("call"), " = ")[-1]
            callee = split_top(call, "(")[0].strip()
            if " as fn" not in call:
                cur.calls.append(callee)
                stmt_callee = strip_generics(callee)
            continue
        if not line.lstrip().startswith("//"):
            stmt_callee = None
        c = CONST_FNDEF.search(line)
        if c:
            val = c.group("val").strip()
            if strip_generics(val) != stmt_callee:
                cur.values.append(val)
    return bodies


def annotate(ws, b):
    """Resolve a body's source declaration: (file, fn line, owner impl/trait)."""
    path = b.path
    path = re.sub(r"::\{(closure|coroutine|async block|constant|const|opaque)#\d+\}", "", path)
    path = re.sub(r"::promoted\[\d+\]", "", path)
    b.name = path.split("::")[-1]
    if b.file is None:
        return False
    rel = ws.rel(b.file)
    if rel is None or not rel.endswith(".rs"):
        return False
    impls = list(IMPL_AT.finditer(path))  # nested impls: the innermost owns the fn
    m = impls[-1] if impls else None
    if m:
        txt = ws.text(m.group("file"), *(int(m.group(k)) for k in ("l1", "c1", "l2", "c2")))
        if txt:
            b.impl = parse_impl_header(txt)
    else:
        segs = strip_generics(path).split("::")
        if len(segs) >= 2:
            b.trait_owner = segs[-2]
    # A closure or async body can sit far below its `fn` line.
    window = 5000 if "{closure" in b.path or "{coroutine" in b.path else 40
    decl = ws.decl_line(rel, b.line, b.name, window)
    if decl is None:
        return False
    b.file, b.line = rel, decl
    return True


def main():
    args = sys.argv[1:]
    root = os.path.realpath(args[0])
    skip = set()
    target_dir = os.path.join(root, "target", "rust-oracle")
    i = 1
    while i < len(args):
        if args[i] == "--skip":
            skip.add(args[i + 1]); i += 2
        elif args[i] == "--all-features":
            EXTRA_CARGO_ARGS.append("--all-features"); i += 1
        elif args[i] == "--target-dir":
            target_dir = args[i + 1]; i += 2
        else:
            i += 1
    ws = Workspace(root)
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=root, capture_output=True, text=True, check=True).stdout)
    members = set(meta["workspace_members"])
    bodies, covered = [], []
    for pkg in meta["packages"]:
        if pkg["id"] not in members or pkg["name"] in skip:
            continue
        for tgt in pkg["targets"]:
            kinds = tgt["kind"]
            kind = "lib" if any(k in ("lib", "rlib", "proc-macro") for k in kinds) else kinds[0]
            if kind not in ("lib", "bin", "test", "example"):
                continue
            crate = tgt["name"].replace("-", "_")
            # Parsed bodies are cached per target so an interrupted run resumes.
            cache = os.path.join(target_dir, "mir-cache", f"{pkg['name']}--{kind}--{tgt['name']}.json")
            raw = None
            if os.path.exists(cache):
                with open(cache) as fh:
                    raw = json.load(fh)
            else:
                spec = f"{pkg['name']}@{pkg['version']}"
                mir = run_mir(root, spec, kind, tgt["name"], target_dir)
                if mir is None:
                    continue
                raw = [dict(path=b.path, file=b.file, line=b.line, calls=b.calls, values=b.values)
                       for b in bodies_from_mir(ws, crate, mir)]
                del mir
                os.makedirs(os.path.dirname(cache), exist_ok=True)
                with open(cache, "w") as fh:
                    json.dump(raw, fh)
            covered.append(os.path.relpath(tgt["src_path"], root))
            for r in raw:
                b = Body()
                b.crate, b.path, b.file, b.line = crate, r["path"], r["file"], r["line"]
                b.calls, b.values = r["calls"], r["values"]
                if annotate(ws, b):
                    bodies.append(b)
            sys.stderr.write(f"[oracle] {pkg['name']} {kind} {tgt['name']}: {len(bodies)} bodies so far\n")

    # Definition tables.
    free = defaultdict(list)        # (crate, path) -> body
    inherent = defaultdict(list)    # (type, method) -> bodies
    trait_impl = defaultdict(list)  # (trait, type, method) -> bodies
    by_trait_method = defaultdict(list)  # (trait, method) -> impl bodies
    defaults = defaultdict(list)    # (trait, method) -> trait default bodies
    free_by_name = defaultdict(list)  # (crate, fn name) -> bodies
    seen_decl = set()

    def add(table, key, b):
        # A module shared by several crates (`tests/common/mod.rs`) has one
        # body per crate; keep one entry per declaration and table key.
        if (id(table), key, b.file, b.line) in seen_decl:
            return
        seen_decl.add((id(table), key, b.file, b.line))
        table[key].append(b)

    # Closure / async bodies count too: `#[async_trait]` moves a method's
    # span onto the attribute, and only its inner async body still points at
    # the `fn` line. Both resolve to the same declaration.
    for b in bodies:
        if b.impl:
            tr, ty = b.impl
            if tr:
                add(trait_impl, (tr, ty, b.name), b)
                add(by_trait_method, (tr, b.name), b)
            else:
                add(inherent, (ty, b.name), b)
        else:
            add(free, (b.crate, strip_generics(re.sub(r"::\{[^}]*\}", "", b.path))), b)
            add(free_by_name, (b.crate, b.name), b)
            if b.trait_owner:
                add(defaults, (b.trait_owner, b.name), b)
    known_types = ({k[1] for k in trait_impl} | {k[0] for k in inherent}) - {"*"}
    # Types declared anywhere in the compiled files (crate-root types print
    # without a path, like type parameters do).
    decl = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|union|type)\s+([A-Za-z_]\w*)")
    for f in {b.file for b in bodies}:
        for line in ws.lines(f) or []:
            m = decl.match(line)
            if m:
                known_types.add(m.group(1))

    def impl_module(b):
        """`a::b` for a body at `a::b::<impl at …>::m`, with its crate."""
        head = b.path.split("<impl at", 1)[0].rstrip(":")
        return f"{b.crate}::{head}" if head else b.crate

    def narrow(hits, type_path, crate):
        """Several impls for one bare type name: keep those in the module
        the (untrimmed) type path names, when that decides it."""
        if len({(b.file, b.line) for b in hits}) <= 1:
            return hits
        tp = re.sub(r"^(&'?\w*\s*|mut\s+|dyn\s+)+", "", type_path.strip())
        mod = "::".join(strip_generics(tp).split("::")[:-1])
        full = mod if mod.split("::")[0] in crate_names else f"{crate}::{mod}" if mod else crate
        same = [b for b in hits if impl_module(b) == full]
        return same or hits

    crate_names = {b.crate for b in bodies}

    def resolve(crate, callee):
        """-> (bodies, via_iface)"""
        c = callee.strip()
        if c.startswith("<"):
            inner = c[1:]
            depth, cut = 1, None
            for i, ch in enumerate(inner):
                if ch == "<":
                    depth += 1
                elif ch == ">" and not (i > 0 and inner[i - 1] == "-"):
                    depth -= 1
                    if depth == 0:
                        cut = i
                        break
            if cut is None:
                return [], False
            qual, rest = inner[:cut], inner[cut + 1:]
            method = last_seg(rest)
            parts = split_top(qual, " as ")
            if len(parts) != 2:
                ty = last_seg(parts[0])
                return inherent.get((ty, method), []), False
            self_ty, tr = parts[0].strip(), last_seg(parts[1])
            st = self_ty.lstrip("&").strip()
            # With untrimmed paths, workspace types print with their module
            # path (`shapes::Circle`); a bare capitalised name that no impl
            # mentions is a type parameter (`T`, `S`, `Self`).
            bare_param = (re.fullmatch(r"[A-Z]\w*", st) is not None
                          and (st not in known_types or len(st) <= 2))
            # `<S as Tr>::Assoc` (a projection) is as unknown as a parameter.
            if st.startswith("dyn ") or st == "Self" or bare_param or st.startswith("<"):
                return by_trait_method.get((tr, method), []) + defaults.get((tr, method), []), True
            ty = last_seg(st)
            hit = trait_impl.get((tr, ty, method))
            if hit:
                return narrow(hit, st, crate), False
            return defaults.get((tr, method)) or trait_impl.get((tr, "*", method), []), False
        # An inherent impl outside the type's module prints as
        # `module::<impl path::Type>::method`.
        at = c.find("::<impl ")
        if at >= 0:
            start = at + len("::<impl ")
            depth, end = 1, None
            for i in range(start, len(c)):
                if c[i] == "<":
                    depth += 1
                elif c[i] == ">" and c[i - 1] != "-":
                    depth -= 1
                    if depth == 0:
                        end = i
                        break
            if end is not None:
                ty_path = c[start:end]
                hit = inherent.get((last_seg(ty_path), last_seg(c[end + 1:])), [])
                return narrow(hit, ty_path, crate), False
        path = strip_generics(c)
        segs = path.split("::")
        for key in ((crate, path), (segs[0], "::".join(segs[1:]))):
            if key in free:
                return free[key], False
        if len(segs) >= 2:
            hit = inherent.get((segs[-2], segs[-1]))
            if hit:
                return narrow(hit, "::".join(segs[:-1]), crate), False
        # rustc prints another crate's items by their visible (re-exported)
        # path (`graphengine_parsing::load_config`); fall back to a unique
        # free function of that name in that crate.
        if len(segs) > 1 and segs[0] in crate_names and segs[0] != crate:
            cands = free_by_name.get((segs[0], segs[-1]), [])
            if len({(b.file, b.line) for b in cands}) == 1:
                return cands, False
        return [], False

    out, seen = [], set()
    for b in bodies:
        caller = (b.file, b.line)
        for kind, items in (("call", b.calls), ("value", b.values)):
            for callee in items:
                targets, via = resolve(b.crate, callee)
                for t in targets:
                    key = (caller, (t.file, t.line), kind == "call", via)
                    if key in seen or (t.file, t.line) == caller:
                        continue
                    seen.add(key)
                    out.append(dict(caller_file=b.file, caller_line=b.line, caller_name=b.name,
                                    callee_file=t.file, callee_line=t.line, callee_name=t.name,
                                    is_call=kind == "call", via_iface=via,
                                    std_trait=via and STD_TRAIT.search(callee) is not None,
                                    mir_callee=callee))
    json.dump(out, sys.stdout, indent=0)
    sys.stderr.write(f"[oracle] {len(out)} references; covered targets: {len(covered)}\n")
    os.makedirs(target_dir, exist_ok=True)
    with open(os.path.join(target_dir, "covered_files.json"), "w") as fh:
        json.dump(sorted({b.file for b in bodies}), fh)


if __name__ == "__main__":
    main()
