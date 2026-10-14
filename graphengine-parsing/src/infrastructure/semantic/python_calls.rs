//! Python call semantics on top of scip-python's index.
//!
//! scip-python (Pyright) gives every use the symbol of a name's *first*
//! declaration and records later declarations of the same name (`@overload`
//! implementations, property setters, platform branches, redefinitions) as
//! plain references; functions nested in different methods of one class share
//! one symbol; and `C(...)` names the class, not the `__init__` it runs. This
//! module maps such uses to the function that actually runs, using only the
//! index (symbols, relationships) plus the source text and syntax tree for
//! decorators and `if`/`try` branches.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::application::ports::{IndexTarget, SemanticIndex};
use crate::domain::{Node, Range};
use crate::infrastructure::semantic::symbol_mapping::SymbolIndex;

pub(super) type SourceCache = HashMap<PathBuf, Vec<String>>;

fn lines_of<'c>(cache: &'c mut SourceCache, file: &Path) -> &'c Vec<String> {
    cache.entry(file.to_path_buf()).or_insert_with(|| {
        std::fs::read_to_string(file)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    })
}

/// Whether the occurrence is the name in a `def`/`class` statement. scip-python
/// records a redeclaration's name as a reference to the first declaration;
/// it declares a function, it does not use one.
pub(super) fn is_declaration_site(cache: &mut SourceCache, site: &IndexTarget) -> bool {
    let lines = lines_of(cache, &site.file);
    let Some(line) = site.line.checked_sub(1).and_then(|i| lines.get(i as usize)) else {
        return false;
    };
    let before: String = line.chars().take(site.col as usize).collect();
    let before = before.trim_end();
    ["def", "class"].iter().any(|kw| {
        before.strip_suffix(kw).is_some_and(|rest| {
            rest.is_empty() || rest.ends_with(char::is_whitespace) || rest.ends_with(';')
        })
    })
}

/// The definition a use really names when scip-python misnames it.
///
/// A name reached through a star import (`httpx.Response`, where
/// `httpx/__init__.py` does `from ._models import *`) gets the symbol of
/// another star-imported name (`` `httpx._models`/Request# ``). When the
/// identifier at the use differs from the symbol's name, the use names the
/// symbol with the identifier's name in the same module, else the only
/// module-level class or function of that name in the same top-level package.
/// An alias (`import a as b`) has neither and is left alone.
pub(super) fn renamed_definition<I: SemanticIndex>(
    index: &I,
    cache: &mut SourceCache,
    site: &IndexTarget,
    def: &IndexTarget,
) -> Option<IndexTarget> {
    let lines = lines_of(cache, &site.file);
    let line = lines.get(site.line.checked_sub(1)? as usize)?;
    let text: String = line
        .chars()
        .skip(site.col as usize)
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    renamed_symbol(index, &def.symbol_moniker, &text)
}

/// [`renamed_definition`] for a symbol the index gave identifier `text`.
fn renamed_symbol<I: SemanticIndex>(index: &I, symbol: &str, text: &str) -> Option<IndexTarget> {
    let (stem, tail) = symbol
        .strip_suffix("().")
        .map(|s| (s, "()."))
        .or_else(|| symbol.strip_suffix('#').map(|s| (s, "#")))
        .or_else(|| symbol.strip_suffix('.').map(|s| (s, ".")))?;
    let cut = stem.rfind(|c: char| !(c.is_alphanumeric() || c == '_'))? + 1;
    let (prefix, name) = stem.split_at(cut);
    if text.is_empty() || text == name {
        return None;
    }
    let renamed = format!("{prefix}{text}{tail}");
    let text = text.to_string();
    if let Some(mut t) = index.definition_of(&renamed) {
        t.symbol_moniker = renamed;
        return Some(t);
    }
    // `` `httpx._transports.asgi`/ `` -> package `httpx`
    let package = |s: &str| -> Option<String> {
        let module = s.rsplit_once('/')?.0;
        let module = module.rsplit(' ').next()?.trim_matches('`');
        Some(module.split('.').next()?.to_string())
    };
    let root = package(symbol)?;
    let mut same_package: Vec<IndexTarget> = index
        .module_level_definitions(&text)
        .into_iter()
        .filter(|t| package(&t.symbol_moniker).as_deref() == Some(root.as_str()))
        .collect();
    (same_package.len() == 1).then(|| same_package.remove(0))
}

/// Decorator lines (trimmed, without `@`) directly above the statement on
/// 1-based `line`.
fn decorators_above(lines: &[String], line: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = line.saturating_sub(1) as usize;
    while i > 0 {
        i -= 1;
        let t = lines.get(i).map(|l| l.trim()).unwrap_or("");
        if let Some(d) = t.strip_prefix('@') {
            out.push(d.trim().to_string());
        } else if !t.starts_with('#') {
            break;
        }
    }
    out
}

/// `@overload` stubs and property setters/deleters: calling the name never
/// runs them (the implementation, or the getter, runs instead).
fn is_never_called_variant(cache: &mut SourceCache, node: &Node) -> bool {
    let lines = lines_of(cache, Path::new(&node.location.file));
    decorators_above(lines, node.location.start_line)
        .iter()
        .any(|d| {
            let d = d.split('(').next().unwrap_or(d);
            d == "overload"
                || d.ends_with(".overload")
                || d.ends_with(".setter")
                || d.ends_with(".deleter")
        })
}

/// Classes whose `__init__` is generated (dataclasses, attrs): `C(...)` runs
/// no `__init__` written in the repo.
fn generates_init(cache: &mut SourceCache, class_def: &IndexTarget) -> bool {
    let lines = lines_of(cache, &class_def.file);
    decorators_above(lines, class_def.line).iter().any(|d| {
        let d = d.split('(').next().unwrap_or(d);
        let last = d.rsplit('.').next().unwrap_or(d);
        matches!(last, "dataclass" | "define" | "frozen" | "mutable")
            || d == "attr.s"
            || d == "attr.attrs"
    })
}

/// External bases that define no `__init__` of their own, so the MRO walk
/// can pass them (`class C(Base, Generic[T])`).
fn is_trivial_external_base(symbol: &str) -> bool {
    if let Some(name) = symbol.strip_prefix("external ") {
        return matches!(name, "object" | "Generic" | "Protocol" | "ABC");
    }
    [
        " builtins/object#",
        " typing/Generic#",
        " typing/Protocol#",
        " typing_extensions/Protocol#",
        " abc/ABC#",
    ]
    .iter()
    .any(|s| symbol.ends_with(s))
}

/// `enum.Enum` and its standard subclasses, as an external base.
fn is_enum_base(symbol: &str) -> bool {
    let name = symbol.strip_prefix("external ").or_else(|| {
        symbol
            .strip_suffix('#')
            .and_then(|s| s.rsplit_once(" enum/").map(|(_, n)| n))
    });
    matches!(
        name,
        Some("Enum" | "IntEnum" | "StrEnum" | "Flag" | "IntFlag" | "ReprEnum")
    )
}

/// Per-resolve state: parsed trees, extra declaration sites, linearizations.
#[derive(Default)]
pub(super) struct PyResolver {
    trees: HashMap<PathBuf, Option<(tree_sitter::Tree, String)>>,
    /// symbol -> redeclaration sites (`def` names recorded as references).
    decl_sites: HashMap<String, Vec<IndexTarget>>,
    mro: HashMap<String, Vec<String>>,
}

impl PyResolver {
    pub(super) fn add_declaration_site(&mut self, symbol: &str, site: &IndexTarget) {
        self.decl_sites
            .entry(symbol.to_string())
            .or_default()
            .push(site.clone());
    }

    /// The functions a use of `def` at `site` can run, given that `direct`
    /// (the node of the symbol's first declaration) was found. When the name
    /// is declared more than once: drop `@overload` stubs and setters, prefer
    /// declarations nested in the function around the use (lexical scope),
    /// then those in the same `if`/`try` branch as the use. Whatever remains
    /// is a may-call set (e.g. one variant per platform branch).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn variants<'n, I: SemanticIndex>(
        &mut self,
        index: &I,
        symbols: &SymbolIndex<'n>,
        cache: &mut SourceCache,
        def: &IndexTarget,
        name: &str,
        direct: &'n Node,
        site: &IndexTarget,
    ) -> Vec<&'n Node> {
        let mut decls = index.declarations_of(&def.symbol_moniker);
        if decls.is_empty() {
            decls.push(def.clone());
        }
        if let Some(extra) = self.decl_sites.get(&def.symbol_moniker) {
            decls.extend(extra.iter().cloned());
        }
        if decls.len() < 2 {
            return vec![direct];
        }
        let mut nodes: Vec<&'n Node> = Vec::new();
        for d in &decls {
            if let Some(n) = symbols.find_definition_node(d, name) {
                if !nodes.iter().any(|x| x.id == n.id) {
                    nodes.push(n);
                }
            }
        }
        if nodes.len() < 2 {
            return if nodes.is_empty() {
                vec![direct]
            } else {
                nodes
            };
        }
        let callable: Vec<&'n Node> = nodes
            .iter()
            .copied()
            .filter(|n| !is_never_called_variant(cache, n))
            .collect();
        if !callable.is_empty() {
            nodes = callable;
        }
        let site_file = site.file.to_string_lossy().to_string();
        let at = Range::with_file(
            site.line,
            site.col,
            site.line,
            site.col + 1,
            site_file.clone(),
        );
        // Lexical scope: a nested `def` is visible inside its enclosing function.
        let scoped: Vec<(&'n Node, &'n Node)> = nodes
            .iter()
            .filter_map(|n| {
                symbols
                    .enclosing_function_of(n)
                    .filter(|p| symbols.encloses(p, &at))
                    .map(|p| (*n, p))
            })
            .collect();
        if let Some(inner) = scoped
            .iter()
            .map(|(_, p)| *p)
            .min_by_key(|p| p.location.end_line - p.location.start_line)
        {
            nodes = scoped
                .iter()
                .filter(|(_, p)| p.id == inner.id)
                .map(|(n, _)| *n)
                .collect();
        } else {
            let top: Vec<&'n Node> = nodes
                .iter()
                .copied()
                .filter(|n| symbols.enclosing_function_of(n).is_none())
                .collect();
            if !top.is_empty() {
                nodes = top;
            }
        }
        if nodes.len() > 1 {
            // Variants in the use's own file must share its branch; ones in
            // other files arrive by import and stay candidates.
            let path = self.branch_path(&site.file, site.line, site.col);
            let reachable: Vec<&'n Node> = nodes
                .iter()
                .copied()
                .filter(|n| {
                    n.location.file != site_file || {
                        let p = self.branch_path(
                            Path::new(&n.location.file),
                            n.location.start_line,
                            n.location.start_char,
                        );
                        compatible(&path, &p)
                    }
                })
                .collect();
            if !reachable.is_empty() {
                nodes = reachable;
            }
        }
        nodes
    }

    /// The in-repo `__init__` (or `__new__`) that `C(...)` runs, for the class
    /// symbol `class`: C's own, else the first one along C's method
    /// resolution order (C3 over the index's base-class relationships).
    /// `None` when an external base or a generated `__init__` comes first, and
    /// for enums: `E(value)` looks a member up and runs neither.
    pub(super) fn constructor_of<'n, I: SemanticIndex>(
        &mut self,
        index: &I,
        symbols: &SymbolIndex<'n>,
        cache: &mut SourceCache,
        class: &str,
    ) -> Option<(&'n Node, IndexTarget)> {
        let mro = self.linearize(index, class, 0);
        if mro.iter().any(|c| is_enum_base(c)) {
            return None;
        }
        for cls in mro {
            let Some(cdef) = index.definition_of(&cls) else {
                if is_trivial_external_base(&cls) {
                    continue;
                }
                return None;
            };
            if generates_init(cache, &cdef) {
                return None;
            }
            for m in ["__init__", "__new__"] {
                if let Some(d) = index.definition_of(&format!("{cls}{m}().")) {
                    if let Some(n) = symbols.find_definition_node(&d, m) {
                        return Some((n, d));
                    }
                }
            }
        }
        None
    }

    fn linearize<I: SemanticIndex>(&mut self, index: &I, class: &str, depth: usize) -> Vec<String> {
        if let Some(m) = self.mro.get(class) {
            return m.clone();
        }
        let mut out = vec![class.to_string()];
        if depth < 32 {
            let bases: Vec<String> = index
                .definition_of(class)
                .and_then(|d| self.bases_from_syntax(index, &d))
                .unwrap_or_else(|| index.supertypes_of(class))
                .into_iter()
                .filter(|b| (b.ends_with('#') || b.starts_with("external ")) && b != class)
                .collect();
            let mut seqs: Vec<Vec<String>> = bases
                .iter()
                .map(|b| self.linearize(index, b, depth + 1))
                .collect();
            seqs.push(bases);
            loop {
                seqs.retain(|s| !s.is_empty());
                if seqs.is_empty() {
                    break;
                }
                let head = seqs
                    .iter()
                    .map(|s| s[0].clone())
                    .find(|h| !seqs.iter().any(|s| s[1..].contains(h)));
                let Some(h) = head else {
                    // Inconsistent hierarchy: keep declaration order.
                    for x in seqs.into_iter().flatten() {
                        if !out.contains(&x) {
                            out.push(x);
                        }
                    }
                    break;
                };
                for s in seqs.iter_mut() {
                    if s[0] == h {
                        s.remove(0);
                    }
                }
                if !out.contains(&h) {
                    out.push(h);
                }
            }
        }
        self.mro.insert(class.to_string(), out.clone());
        out
    }

    /// Base classes of the class whose name is at `class_def`, in order: each
    /// operand in `class C(A, m.B, G[T])` resolved through the index, or
    /// `external <name>` when its definition is outside the index. scip-python
    /// omits the base-class relationships of some classes, so the syntax tree
    /// is read first.
    fn bases_from_syntax<I: SemanticIndex>(
        &mut self,
        index: &I,
        class_def: &IndexTarget,
    ) -> Option<Vec<String>> {
        let entry = self
            .trees
            .entry(class_def.file.clone())
            .or_insert_with(|| parse_py(&class_def.file));
        let (tree, src) = entry.as_ref()?;
        let point = tree_sitter::Point {
            row: class_def.line.saturating_sub(1) as usize,
            column: class_def.col as usize,
        };
        let name = tree.root_node().descendant_for_point_range(point, point)?;
        let class = name.parent().filter(|c| c.kind() == "class_definition")?;
        let mut operands = Vec::new();
        if let Some(args) = class.child_by_field_name("superclasses") {
            let mut c = args.walk();
            for arg in args.named_children(&mut c) {
                let mut e = arg;
                while e.kind() == "subscript" || e.kind() == "generic_type" {
                    match e.child_by_field_name("value").or_else(|| e.named_child(0)) {
                        Some(v) => e = v,
                        None => break,
                    }
                }
                let ident = match e.kind() {
                    "identifier" => e,
                    "attribute" => match e.child_by_field_name("attribute") {
                        Some(a) => a,
                        None => continue,
                    },
                    _ => continue,
                };
                let text = ident.utf8_text(src.as_bytes()).unwrap_or("").to_string();
                let p = ident.start_position();
                operands.push((p.row as u32, p.column as u32, text));
            }
        }
        Some(
            operands
                .into_iter()
                .map(|(row, col, text)| {
                    index
                        .definition_at(&class_def.file, row, col)
                        .map(|d| {
                            // `class C(pkg.Name)` through a star import is
                            // misnamed like any other use of it.
                            renamed_symbol(index, &d.symbol_moniker, &text)
                                .map_or(d.symbol_moniker, |t| t.symbol_moniker)
                        })
                        .unwrap_or_else(|| format!("external {text}"))
                })
                .collect(),
        )
    }

    /// Whether `node` (the declaration of `method`) is abstract: decorated
    /// `@abstractmethod`, or a method of a class that lists `Protocol`. Calls through it
    /// are dispatch, like calls through an interface method.
    pub(super) fn is_abstract<I: SemanticIndex>(
        &mut self,
        index: &I,
        cache: &mut SourceCache,
        node: &Node,
        method: &str,
    ) -> bool {
        let lines = lines_of(cache, Path::new(&node.location.file));
        if decorators_above(lines, node.location.start_line)
            .iter()
            .any(|d| d == "abstractmethod" || d.ends_with(".abstractmethod"))
        {
            return true;
        }
        let Some(class) = method
            .strip_suffix('.')
            .and_then(|m| m.rfind('#').map(|i| &m[..=i]))
        else {
            return false;
        };
        // Only a class that lists `Protocol` itself is a protocol; its
        // explicit implementations inherit from it but are concrete.
        let bases = index
            .definition_of(class)
            .and_then(|d| self.bases_from_syntax(index, &d))
            .unwrap_or_else(|| index.supertypes_of(class));
        bases.iter().any(|b| {
            b == "external Protocol"
                || b.ends_with(" typing/Protocol#")
                || b.ends_with(" typing_extensions/Protocol#")
        })
    }

    /// `(if/try statement id, branch)` for every `if`/`try` around the
    /// 1-based `line`, 0-based `col`. The condition belongs to no branch; a
    /// `try` body and its `else` share one.
    fn branch_path(&mut self, file: &Path, line: u32, col: u32) -> Vec<(usize, usize)> {
        let entry = self
            .trees
            .entry(file.to_path_buf())
            .or_insert_with(|| parse_py(file));
        let Some((tree, _)) = entry else {
            return Vec::new();
        };
        let point = tree_sitter::Point {
            row: line.saturating_sub(1) as usize,
            column: col as usize,
        };
        let Some(mut n) = tree.root_node().descendant_for_point_range(point, point) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        while let Some(p) = n.parent() {
            let branch = match p.kind() {
                "if_statement" if p.child_by_field_name("condition") != Some(n) => {
                    child_index(p, n)
                }
                "try_statement" => match n.kind() {
                    "block" | "else_clause" => Some(0),
                    "except_clause" | "except_group_clause" => child_index(p, n),
                    _ => None,
                },
                _ => None,
            };
            if let Some(b) = branch {
                out.push((p.id(), b));
            }
            n = p;
        }
        out
    }
}

/// Calls inside functions of `file` whose callee name carries no index
/// occurrence, as (1-based line, name): `x.name(...)` on a receiver the
/// checker typed `Any` (an unannotated parameter such as a pytest fixture),
/// or `name(...)` it could not bind. Such a call may reach any function
/// called `name`, which no compiler edge records. A member call the index
/// binds to a builtin class or function (`self.set(...)` recorded as
/// `builtins/set#`) is a misresolution and counts as unresolved too.
pub(super) fn unresolved_calls(
    file: &Path,
    occurrences: &std::collections::HashSet<(u32, u32)>,
    builtin_sites: &std::collections::HashSet<(u32, u32)>,
) -> Vec<(u32, String)> {
    let Some((tree, src)) = parse_py(file) else {
        return Vec::new();
    };
    let lines: Vec<&str> = src.lines().collect();
    // tree-sitter columns are bytes; index columns count characters.
    let char_col = |row: usize, byte: usize| -> u32 {
        lines
            .get(row)
            .and_then(|l| l.get(..byte))
            .map_or(byte, |prefix| prefix.chars().count()) as u32
    };
    let mut out = Vec::new();
    let mut stack = vec![(tree.root_node(), false)];
    while let Some((n, in_fn)) = stack.pop() {
        let in_fn = in_fn || n.kind() == "function_definition";
        if n.kind() == "call" && in_fn {
            let callee = n
                .child_by_field_name("function")
                .and_then(|f| match f.kind() {
                    "identifier" => Some(f),
                    "attribute" => f.child_by_field_name("attribute"),
                    _ => None,
                });
            if let Some(id) = callee {
                let p = id.start_position();
                let at = (p.row as u32, char_col(p.row, p.column));
                let member = id.kind() == "identifier"
                    && id.parent().is_some_and(|a| a.kind() == "attribute");
                if !occurrences.contains(&at) || (member && builtin_sites.contains(&at)) {
                    if let Ok(name) = id.utf8_text(src.as_bytes()) {
                        out.push((p.row as u32 + 1, name.to_string()));
                    }
                }
            }
        }
        let mut c = n.walk();
        stack.extend(n.children(&mut c).map(|ch| (ch, in_fn)));
    }
    out.sort();
    out
}

fn child_index(parent: tree_sitter::Node<'_>, child: tree_sitter::Node<'_>) -> Option<usize> {
    let mut c = parent.walk();
    let i = parent.children(&mut c).position(|x| x == child);
    i
}

/// Two positions can run in one execution unless some `if`/`try` around both
/// puts them in different branches.
fn compatible(a: &[(usize, usize)], b: &[(usize, usize)]) -> bool {
    a.iter()
        .all(|(id, br)| b.iter().all(|(id2, br2)| id != id2 || br == br2))
}

fn parse_py(path: &Path) -> Option<(tree_sitter::Tree, String)> {
    let src = std::fs::read_to_string(path).ok()?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(tree_sitter_python::language()).ok()?;
    let tree = parser.parse(&src, None)?;
    Some((tree, src))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Confidence;

    fn site(file: &Path, line: u32, col: u32) -> IndexTarget {
        IndexTarget {
            file: file.to_path_buf(),
            line,
            col,
            symbol_moniker: String::new(),
            confidence: Confidence::High,
        }
    }

    #[test]
    fn declaration_sites_and_decorators() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("m.py");
        std::fs::write(
            &f,
            "@t.overload\ndef f(x: int) -> int: ...\n# note\n@overload\ndef f(x): return g(x)\nclass C: pass\nasync def h(): f(1)\n",
        )
        .unwrap();
        let mut cache = SourceCache::new();
        assert!(is_declaration_site(&mut cache, &site(&f, 2, 4)));
        assert!(is_declaration_site(&mut cache, &site(&f, 6, 6)));
        assert!(is_declaration_site(&mut cache, &site(&f, 7, 10)));
        assert!(!is_declaration_site(&mut cache, &site(&f, 7, 15)));
        assert!(!is_declaration_site(&mut cache, &site(&f, 5, 17)));
        let lines = lines_of(&mut cache, &f).clone();
        assert_eq!(decorators_above(&lines, 5), vec!["overload".to_string()]);
        assert_eq!(decorators_above(&lines, 2), vec!["t.overload".to_string()]);
    }

    #[test]
    fn unresolved_calls_are_untyped_or_bound_to_a_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("m.py");
        // 0-based rows: 1 `x.go()`, 2 `self.set(k)`, 3 `len(x)`, 4 `y.read()`
        std::fs::write(
            &f,
            "def f(x, y, self):\n    x.go()\n    self.set(k)\n    len(x)\n    y.read()\nz.top()\n",
        )
        .unwrap();
        let occ: std::collections::HashSet<(u32, u32)> =
            [(2, 9), (3, 4), (4, 6)].into_iter().collect();
        let builtin: std::collections::HashSet<(u32, u32)> = [(2, 9), (3, 4)].into_iter().collect();
        // `go` has no occurrence; `self.set` is bound to builtins `set`; the
        // builtin `len(x)` is a real use; module-level `z.top()` has no caller.
        assert_eq!(
            unresolved_calls(&f, &occ, &builtin),
            vec![(2, "go".to_string()), (3, "set".to_string())]
        );
    }

    #[test]
    fn enum_bases_and_trivial_bases() {
        assert!(is_enum_base("external IntEnum"));
        assert!(is_enum_base(
            "scip-python python python-stdlib 3.11 enum/Enum#"
        ));
        assert!(!is_enum_base(
            "scip-python python pkg 1.0 `pkg.enum`/Thing#"
        ));
        assert!(is_trivial_external_base("external Generic"));
        assert!(is_trivial_external_base(
            "scip-python python python-stdlib 3.11 builtins/object#"
        ));
        assert!(!is_trivial_external_base("external Exception"));
    }

    #[test]
    fn branch_paths_separate_platform_variants() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("m.py");
        std::fs::write(
            &f,
            "if WIN:\n    def raw(): pass\n    def get(): raw()\nelse:\n    def raw(): pass\n    def get(): raw()\ndef use(): raw()\n",
        )
        .unwrap();
        let mut r = PyResolver::default();
        let win_def = r.branch_path(&f, 2, 4);
        let posix_def = r.branch_path(&f, 5, 4);
        let win_use = r.branch_path(&f, 3, 15);
        let posix_use = r.branch_path(&f, 6, 15);
        let top_use = r.branch_path(&f, 7, 11);
        assert!(compatible(&win_use, &win_def) && !compatible(&win_use, &posix_def));
        assert!(compatible(&posix_use, &posix_def) && !compatible(&posix_use, &win_def));
        assert!(compatible(&top_use, &win_def) && compatible(&top_use, &posix_def));
    }
}
