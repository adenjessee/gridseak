//! Index-backed semantic resolver (SCIP / batch compiler indexes).
//!
//! Implements [`SemanticResolver`] using a [`SemanticIndex`] port and the
//! shared [`SymbolIndex`] caller/callee mapper from `symbol_mapping`.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tracing::{debug, warn};

use crate::application::ports::{
    CallSite, IndexTarget, ResolvedEdges, SemanticIndex, SemanticResolver, SessionMetricsSnapshot,
    SyntaxResults, UnresolvedReference,
};
use crate::application::resolution_disclosure::ResolutionDisclosure;
use crate::domain::{Edge, EdgeKind, Provenance, ProvenanceSource, Range, UseKind};
use crate::infrastructure::semantic::python_calls::{self, is_declaration_site, PyResolver};
use crate::infrastructure::semantic::symbol_mapping::SymbolIndex;

/// Outcome of one SCIP-backed call-site lookup.
enum SiteHit {
    Mapped(Edge, Range),
    ResolvedOnly(Range),
}

/// Resolves call sites via an offline semantic index (SCIP, …).
pub struct IndexBackedSemanticResolver<I: SemanticIndex + Send + Sync> {
    index: I,
    workspace_root: PathBuf,
    language: String,
    emitted_edges: std::sync::atomic::AtomicU64,
}

impl<I: SemanticIndex + Send + Sync> IndexBackedSemanticResolver<I> {
    pub fn new(index: I, workspace_root: PathBuf, language: impl Into<String>) -> Self {
        Self {
            index,
            workspace_root,
            language: language.into(),
            emitted_edges: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn resolve_one(
        &self,
        call_site: &CallSite,
        symbol_index: &SymbolIndex<'_>,
        seen: &mut std::collections::HashSet<(String, String)>,
    ) -> Option<SiteHit> {
        let file = PathBuf::from(&call_site.location.file);
        let (line, col) = crate::infrastructure::semantic::caret::caret_for_callee(call_site);
        let scip_line = line.saturating_sub(1);
        let target = self.index.definition_at(&file, scip_line, col)?;
        let location = call_site.location.clone();

        let Some(caller) = symbol_index.find_enclosing_function(&call_site.location) else {
            return Some(SiteHit::ResolvedOnly(location));
        };
        let index_target = index_target_from_semantic(&self.workspace_root, &target);
        if !file_is_under(&index_target.file, &self.workspace_root) {
            debug!(
                target: "index_backed::resolve",
                "SCIP hit external {} at {}; suppressing heuristic fallback",
                target.symbol_moniker,
                index_target.file.display()
            );
            return Some(SiteHit::ResolvedOnly(location));
        }
        let Some(callee) = symbol_index.find_callee_for(&index_target) else {
            debug!(
                target: "index_backed::resolve",
                "SCIP hit {} but no in-repo callee; suppressing heuristic fallback",
                target.symbol_moniker
            );
            return Some(SiteHit::ResolvedOnly(location));
        };

        if caller.id == callee.id {
            debug!(
                target: "index_backed::resolve",
                "self-recursion at {} → {}; dropping edge",
                caller.fqn,
                callee.fqn
            );
            return Some(SiteHit::ResolvedOnly(location));
        }

        let key = (caller.id.clone(), callee.id.clone());
        if !seen.insert(key) {
            return Some(SiteHit::ResolvedOnly(location));
        }

        let edge = Edge::new(
            caller.id.clone(),
            callee.id.clone(),
            EdgeKind::Call,
            Provenance::new(ProvenanceSource::Compiler, target.confidence),
        );
        Some(SiteHit::Mapped(edge, location))
    }

    /// Map every compiler-resolved use to `(enclosing function → defining
    /// function)`. Uses through an interface/abstract method fan out to its
    /// implementations at reduced confidence when there is more than one.
    fn resolve_from_occurrences(
        &self,
        occurrences: &[(IndexTarget, IndexTarget)],
        symbol_index: &SymbolIndex<'_>,
        seen: &mut std::collections::HashSet<(String, String)>,
        out: &mut ResolvedEdges,
    ) {
        let mut source_cache: std::collections::HashMap<PathBuf, Vec<String>> =
            std::collections::HashMap::new();
        let mut uses: std::collections::BTreeMap<
            (String, String),
            (UseKind, crate::domain::Confidence),
        > = std::collections::BTreeMap::new();
        let python = language_matches("python", &self.language);
        let rust = language_matches("rust", &self.language);
        let mut py = PyResolver::default();
        if python {
            // A redeclaration's name is recorded as a use of the first one.
            for (site, def) in occurrences {
                if is_declaration_site(&mut source_cache, site) {
                    py.add_declaration_site(&def.symbol_moniker, site);
                }
            }
        }
        for (site, def) in occurrences {
            let renamed = python
                .then(|| {
                    python_calls::renamed_definition(&self.index, &mut source_cache, site, def)
                })
                .flatten();
            let def = renamed.as_ref().unwrap_or(def);
            if !file_is_under(&def.file, &self.workspace_root)
                || !is_callable_symbol(&def.symbol_moniker)
                || (python && is_declaration_site(&mut source_cache, site))
            {
                continue;
            }
            let site_file = site.file.to_string_lossy().to_string();
            let site_range =
                Range::with_file(site.line, site.col, site.line, site.col + 1, site_file);
            // Top-level uses (Go `var x = f()`, TS module scope) have no
            // enclosing function; credit the enclosing declaration so the
            // callee never looks unused.
            let Some(caller) = symbol_index
                .find_enclosing_function(&site_range)
                .or_else(|| symbol_index.find_enclosing_declaration(&site_range))
            else {
                continue;
            };
            let name = if def.symbol_moniker.starts_with("local ") {
                // Local monikers carry no name; read the identifier at the
                // definition site so only a same-named function node matches.
                match identifier_at(&mut source_cache, def) {
                    Some(n) => n,
                    None => continue,
                }
            } else {
                short_symbol_name(&def.symbol_moniker)
            };
            let direct = symbol_index.find_definition_node(def, &name);
            // `new C(…)` on a class that declares no constructor: SCIP names
            // the class, not a constructor; the inherited one runs.
            let inherited_ctor = || {
                (direct.is_none()
                    && def.symbol_moniker.ends_with('#')
                    && language_matches("typescript", &self.language)
                    && is_new_site(&mut source_cache, site)
                    && use_kind_at(&mut source_cache, site) == UseKind::Call)
                    .then(|| self.nearest_constructor(symbol_index, def.clone()))
                    .flatten()
            };
            let callees: Vec<(&crate::domain::Node, crate::domain::Confidence, UseKind)> =
                match direct.or_else(inherited_ctor) {
                    Some(node) if python => {
                        let via = use_kind_at(&mut source_cache, site);
                        let run = py.variants(
                            &self.index,
                            symbol_index,
                            &mut source_cache,
                            def,
                            &name,
                            node,
                            site,
                        );
                        let confidence = if run.len() > 1 {
                            crate::domain::Confidence::Medium
                        } else {
                            def.confidence
                        };
                        if py.is_abstract(&self.index, &mut source_cache, node, &def.symbol_moniker)
                        {
                            // A call of an abstract / Protocol method may run
                            // any override: dispatch, not a direct call.
                            let via = if via == UseKind::Call {
                                UseKind::Dispatch
                            } else {
                                via
                            };
                            let mut out = vec![(node, confidence, via)];
                            for imp in self.index.implementations_of(&def.symbol_moniker) {
                                let imp_name = short_symbol_name(&imp.symbol_moniker);
                                if let Some(n) = symbol_index.find_definition_node(&imp, &imp_name)
                                {
                                    out.push((n, imp.confidence, via));
                                }
                            }
                            out
                        } else {
                            run.into_iter().map(|n| (n, confidence, via)).collect()
                        }
                    }
                    // Rust: a use resolved to the trait's own method
                    // (`Trait#m().`) went through `dyn Trait` or a generic
                    // bound — any impl may run, so it is dispatch to the
                    // default body and every implementation. Calls on a
                    // concrete type resolve to `impl#[T][Trait]m().` and
                    // stay direct calls. Known imprecision: a concrete
                    // receiver calling a default method its type does not
                    // override is also named `Trait#m().`, so it reads as
                    // dispatch (on serde_json, 20 such edges against 83
                    // generic calls the opposite rule would call direct).
                    Some(node) if rust && is_rust_trait_method(&def.symbol_moniker) => {
                        let impls = self.index.implementations_of(&def.symbol_moniker);
                        let via = match use_kind_at(&mut source_cache, site) {
                            UseKind::Call => UseKind::Dispatch,
                            other => other,
                        };
                        let mut out = vec![(node, def.confidence, via)];
                        for imp in impls {
                            let imp_name = short_symbol_name(&imp.symbol_moniker);
                            if let Some(n) = symbol_index.find_definition_node(&imp, &imp_name) {
                                out.push((n, imp.confidence, via));
                            }
                        }
                        out
                    }
                    Some(node) => {
                        let via = use_kind_at(&mut source_cache, site);
                        vec![(node, def.confidence, via)]
                    }
                    // `C(...)` runs the `__init__` C's MRO finds.
                    None if python
                        && def.symbol_moniker.ends_with('#')
                        && use_kind_at(&mut source_cache, site) == UseKind::Call =>
                    {
                        py.constructor_of(
                            &self.index,
                            symbol_index,
                            &mut source_cache,
                            &def.symbol_moniker,
                        )
                        .map(|(init, init_def)| {
                            let m = short_symbol_name(&init_def.symbol_moniker);
                            py.variants(
                                &self.index,
                                symbol_index,
                                &mut source_cache,
                                &init_def,
                                &m,
                                init,
                                site,
                            )
                        })
                        .unwrap_or_default()
                        .into_iter()
                        .map(|n| (n, def.confidence, UseKind::Call))
                        .collect()
                    }
                    None => self
                        .index
                        .implementations_of(&def.symbol_moniker)
                        .iter()
                        .filter_map(|imp| {
                            let imp_name = short_symbol_name(&imp.symbol_moniker);
                            symbol_index
                                .find_definition_node(imp, &imp_name)
                                .map(|n| (n, imp.confidence, UseKind::Dispatch))
                        })
                        .collect(),
                };
            for (callee, confidence, via) in callees {
                if caller.id == callee.id {
                    continue;
                }
                // Keep the strongest use per pair: Call > Dispatch > Value.
                let entry = uses
                    .entry((caller.id.clone(), callee.id.clone()))
                    .or_insert((via, confidence));
                if via < entry.0 {
                    *entry = (via, confidence);
                }
            }
        }
        for ((caller, callee), (via, confidence)) in uses {
            if !seen.insert((caller.clone(), callee.clone())) {
                continue;
            }
            out.add_call_edge(Edge::new(
                caller,
                callee,
                EdgeKind::Call,
                Provenance::new(ProvenanceSource::Compiler, confidence).with_via(via),
            ));
            out.stats.compiler_edges += 1;
            self.emitted_edges
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// `super(…)` in a TS/JS constructor calls the base class's constructor,
    /// but SCIP records no occurrence on the `super` keyword. Resolve the
    /// class's `extends` operand through the index and link the enclosing
    /// function to the nearest base class that declares a constructor.
    fn resolve_super_calls(
        &self,
        hints: &SyntaxResults,
        symbol_index: &SymbolIndex<'_>,
        seen: &mut std::collections::HashSet<(String, String)>,
        out: &mut ResolvedEdges,
    ) {
        if !language_matches("typescript", &self.language) {
            return;
        }
        for file in &hints.source_files {
            let path = PathBuf::from(file);
            if !self.index.has_file(&path) {
                continue;
            }
            let Some((tree, src)) = parse_ts(&path) else {
                continue;
            };
            if !src.contains("super") {
                continue;
            }
            // (caller, base-class operand) per `super(…)` call.
            let mut found = Vec::new();
            let mut stack = vec![tree.root_node()];
            while let Some(n) = stack.pop() {
                let mut c = n.walk();
                stack.extend(n.children(&mut c));
                let is_super_call = n.kind() == "call_expression"
                    && n.child_by_field_name("function")
                        .is_some_and(|f| f.kind() == "super");
                let Some(class) = is_super_call.then(|| ancestor_class(n)).flatten() else {
                    continue;
                };
                let p = n.start_position();
                let (row, col) = (p.row as u32 + 1, p.column as u32);
                let site = Range::with_file(row, col, row, col + 5, file.clone());
                if let Some(caller) = symbol_index.find_enclosing_function(&site) {
                    found.push((caller, extends_operand(class).map(|o| (path.clone(), o))));
                }
            }
            for (caller, operand) in found {
                let Some((op_file, (row, col))) = operand else {
                    continue;
                };
                let Some(def) = self.index.definition_at(&op_file, row, col) else {
                    continue;
                };
                let def = index_target_from_semantic(&self.workspace_root, &def);
                let Some(callee) = self.nearest_constructor(symbol_index, def) else {
                    continue;
                };
                if caller.id == callee.id || !seen.insert((caller.id.clone(), callee.id.clone())) {
                    continue;
                }
                out.add_call_edge(Edge::new(
                    caller.id.clone(),
                    callee.id.clone(),
                    EdgeKind::Call,
                    Provenance::new(ProvenanceSource::Compiler, crate::domain::Confidence::High)
                        .with_via(UseKind::Call),
                ));
                out.stats.compiler_edges += 1;
                self.emitted_edges
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Constructor that runs for `new C(…)` where `class_def` is C's SCIP
    /// definition: C's own `constructor`, else the nearest base class's (a
    /// class without one inherits its base's). Each `extends` operand is
    /// resolved through the index, so the result stays compiler-sourced.
    fn nearest_constructor<'n>(
        &self,
        symbol_index: &SymbolIndex<'n>,
        class_def: IndexTarget,
    ) -> Option<&'n crate::domain::Node> {
        let mut def = class_def;
        for _ in 0..16 {
            let at = Range::with_file(
                def.line,
                def.col,
                def.line,
                def.col + 1,
                def.file.to_string_lossy(),
            );
            let (class, ctor) = symbol_index.class_constructor_at(&at)?;
            if ctor.is_some() {
                return ctor;
            }
            let (row, col) = parse_ts(&def.file).and_then(|(t, _)| {
                class_declared_at(t.root_node(), class.location.start_line, def.line)
                    .and_then(extends_operand)
            })?;
            let next = self.index.definition_at(&def.file, row, col)?;
            def = index_target_from_semantic(&self.workspace_root, &next);
        }
        None
    }

    /// Parsed source files the index has no document for. Calls made in them
    /// produce no edges, so they lie outside what the call graph certifies.
    fn unindexed_files(&self, hints: &SyntaxResults) -> Vec<String> {
        let mut files: Vec<String> = hints
            .source_files
            .iter()
            .filter(|f| !self.index.has_file(Path::new(f)))
            .map(|f| {
                Path::new(f)
                    .strip_prefix(&self.workspace_root)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| f.clone())
            })
            .collect();
        files.sort();
        files.dedup();
        files
    }

    /// Python calls the index left unresolved (see
    /// [`python_calls::unresolved_calls`]), workspace-relative.
    fn unresolved_calls(&self, hints: &SyntaxResults) -> Vec<(String, u32, String)> {
        let mut out = Vec::new();
        for file in &hints.source_files {
            let path = Path::new(file);
            let Some(starts) = self.index.occurrence_starts(path) else {
                continue;
            };
            let builtins: std::collections::HashSet<(u32, u32)> = starts
                .iter()
                .filter(|(_, _, s)| is_builtin_global(s))
                .map(|(l, c, _)| (*l, *c))
                .collect();
            let starts: std::collections::HashSet<(u32, u32)> =
                starts.into_iter().map(|(l, c, _)| (l, c)).collect();
            let rel = path
                .strip_prefix(&self.workspace_root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| file.clone());
            for (line, name) in python_calls::unresolved_calls(path, &starts, &builtins) {
                out.push((rel.clone(), line, name));
            }
        }
        out.sort();
        out
    }

    fn resolve_imports_heuristic(&self, hints: &SyntaxResults, out: &mut ResolvedEdges) {
        let all_ranges: Vec<Range> = hints
            .import_specs
            .iter()
            .map(|spec| spec.range.clone())
            .collect();
        if let Ok(edges) =
            crate::infrastructure::lsp::resolvers::import_resolver::ImportResolver::resolve_with_heuristics(
                hints,
                &all_ranges,
            )
        {
            for edge in edges {
                out.add_import_edge(edge);
            }
        }
        let module_dep_edges = crate::infrastructure::lsp::resolvers::module_dependency_resolver::ModuleDependencyResolver::resolve_relative_module_imports(
            hints,
            match self.language.as_str() {
                "python" => vec![".py".to_string()],
                "rust" => vec![".rs".to_string()],
                _ => vec![
                    ".ts".to_string(),
                    ".tsx".to_string(),
                    ".js".to_string(),
                ],
            }
            .as_slice(),
        );
        for edge in module_dep_edges {
            out.add_import_edge(edge);
        }
    }
}

/// Rust `use` paths name functions (`use crate::util::helper;`) but call
/// nothing; drop occurrences inside the scan's `use` declarations so a
/// module never looks like it calls everything it imports.
fn drop_import_uses(
    occurrences: Vec<(IndexTarget, IndexTarget)>,
    hints: &SyntaxResults,
) -> Vec<(IndexTarget, IndexTarget)> {
    let mut by_file: std::collections::HashMap<&str, Vec<&Range>> =
        std::collections::HashMap::new();
    for spec in &hints.import_specs {
        by_file
            .entry(spec.range.file.as_str())
            .or_default()
            .push(&spec.range);
    }
    if by_file.is_empty() {
        return occurrences;
    }
    occurrences
        .into_iter()
        .filter(|(site, _)| {
            let file = site.file.to_string_lossy();
            let Some(ranges) = by_file.get(file.as_ref()) else {
                return true;
            };
            let at = (site.line, site.col);
            !ranges
                .iter()
                .any(|r| (r.start_line, r.start_char) <= at && at < (r.end_line, r.end_char))
        })
        .collect()
}

/// rust-analyzer symbol of a method declared in a trait (`…/Trait#m().`),
/// as opposed to an inherent or trait-impl method (`…/impl#[T]…m().`).
fn is_rust_trait_method(symbol: &str) -> bool {
    matches!(
        graphengine_scip_adapter::rust_method_shape(symbol),
        Some(graphengine_scip_adapter::RustMethod::TraitDecl { .. })
    )
}

/// A top-level class or function of Python's `builtins` (`builtins/set#`).
fn is_builtin_global(symbol: &str) -> bool {
    symbol
        .rsplit(' ')
        .next()
        .and_then(|d| d.strip_prefix("builtins/"))
        .is_some_and(|rest| {
            !rest.contains('#') || (rest.ends_with('#') && rest.matches('#').count() == 1)
        })
}

/// Parse a TS/JS file with the TypeScript grammar (TSX for `.tsx`/`.jsx`).
fn parse_ts(path: &Path) -> Option<(tree_sitter::Tree, String)> {
    let src = std::fs::read_to_string(path).ok()?;
    let tsx = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("tsx") | Some("jsx")
    );
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(if tsx {
            tree_sitter_typescript::language_tsx()
        } else {
            tree_sitter_typescript::language_typescript()
        })
        .ok()?;
    let tree = parser.parse(&src, None)?;
    Some((tree, src))
}

fn is_class_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration" | "abstract_class_declaration" | "class"
    )
}

fn ancestor_class(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if is_class_kind(n.kind()) {
            return Some(n);
        }
        cur = n.parent();
    }
    None
}

/// 0-based (row, col) of the name in a class's `extends` operand:
/// `Base`, `ns.Base` (the `Base`), `Base<T>`.
fn extends_operand(class: tree_sitter::Node<'_>) -> Option<(u32, u32)> {
    let mut c = class.walk();
    let heritage = class
        .children(&mut c)
        .find(|n| n.kind() == "class_heritage")?;
    let mut c = heritage.walk();
    let extends = heritage
        .children(&mut c)
        .find(|n| n.kind() == "extends_clause")?;
    let value = extends.child_by_field_name("value")?;
    let name = match value.kind() {
        "identifier" => value,
        "member_expression" => value.child_by_field_name("property")?,
        _ => return None,
    };
    let p = name.start_position();
    Some((p.row as u32, p.column as u32))
}

/// The class node starting on `start_line` whose name sits on `name_line`
/// (1-based), i.e. the declaration a `Struct` node and its SCIP definition
/// point at.
fn class_declared_at(
    root: tree_sitter::Node<'_>,
    start_line: u32,
    name_line: u32,
) -> Option<tree_sitter::Node<'_>> {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        let (s, e) = (
            n.start_position().row as u32 + 1,
            n.end_position().row as u32 + 1,
        );
        if s > name_line || e < start_line {
            continue;
        }
        if is_class_kind(n.kind())
            && n.child_by_field_name("name")
                .is_some_and(|nm| nm.start_position().row as u32 + 1 == name_line)
        {
            return Some(n);
        }
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
    }
    None
}

fn index_target_from_semantic(workspace_root: &Path, target: &IndexTarget) -> IndexTarget {
    let file = if target.file.is_absolute() {
        target.file.clone()
    } else {
        workspace_root.join(&target.file)
    };
    IndexTarget {
        file,
        line: target.line,
        col: target.col,
        symbol_moniker: short_symbol_name(&target.symbol_moniker),
        confidence: target.confidence,
    }
}

fn file_is_under(file: &Path, root: &Path) -> bool {
    if file.starts_with(root) {
        return true;
    }
    match (file.canonicalize(), root.canonicalize()) {
        (Ok(f), Ok(r)) => f.starts_with(r),
        _ => false,
    }
}

fn language_matches(expected: &str, got: &str) -> bool {
    expected == got
        || (expected == "typescript" && got == "javascript")
        || (expected == "javascript" && got == "typescript")
}

/// Last SCIP descriptor identifier (`NewRouter`, `_parse`, `Use`).
///
/// SCIP monikers wrap the *file* in backticks (`src/`types.ts`/ZodType#_parse().`).
/// Taking the backtick interior used to return `types.ts` / the Go module
/// path, so `find_callee_for` missed in-repo functions even after a hit.
pub(crate) fn short_symbol_name(symbol: &str) -> String {
    if let Some(name) = last_descriptor_name(symbol) {
        return name;
    }
    let descriptor = match symbol.rfind('`') {
        Some(end) => &symbol[end + 1..],
        None => symbol,
    };
    let descriptor = descriptor.trim_start_matches('/');
    let name_part = match descriptor.rsplit_once('#') {
        Some((_, after)) if !after.is_empty() && !after.starts_with('(') => after,
        Some((before, _)) => before
            .rsplit(['/', '.'])
            .find(|s| !s.is_empty())
            .unwrap_or(before),
        None => descriptor,
    };
    let ident = name_part
        .split('(')
        .next()
        .unwrap_or(name_part)
        .trim_matches('.');
    if !ident.is_empty()
        && ident
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    {
        return ident.to_string();
    }
    symbol
        .rsplit(['.', ':'])
        .find(|s| !s.is_empty())
        .unwrap_or(symbol)
        .trim_matches('`')
        .to_string()
}

/// `Call` when the identifier at `site` is immediately invoked —
/// `f(`, `x.f(`, `f[T](` (Go), `f<T>(` (TS), `f?.(`, `new F(`, and a TS
/// tagged template `` f`…` `` (the tag is called with the template), and a TS
/// string-literal element access `obj["f"](` (SCIP records the member's
/// occurrence on the literal) — else `Value`.
fn use_kind_at(
    cache: &mut std::collections::HashMap<PathBuf, Vec<String>>,
    site: &IndexTarget,
) -> UseKind {
    let lines = cache.entry(site.file.clone()).or_insert_with(|| {
        std::fs::read_to_string(&site.file)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    });
    let Some(first) = site.line.checked_sub(1).map(|i| i as usize) else {
        return UseKind::Value;
    };
    let Some(line) = lines.get(first) else {
        return UseKind::Value;
    };
    // The site's line from the identifier on; type arguments may continue
    // onto later lines (`f<{ a: A }>(x)` split over lines), appended on demand.
    let mut rest: Vec<char> = line.chars().skip(site.col as usize).collect();
    let mut i = match rest.first() {
        // `obj["name"](…)`: SCIP puts the member's occurrence on the string
        // literal of a TS element access; skip it and the closing `]`.
        Some(&q) if q == '"' || q == '\'' || q == '`' => {
            let Some(end) = string_literal_end(&rest, q) else {
                return UseKind::Value;
            };
            let mut j = end;
            while rest.get(j).is_some_and(|c| c.is_whitespace()) {
                j += 1;
            }
            if rest.get(j) != Some(&']') {
                return UseKind::Value;
            }
            j + 1
        }
        // `#name` is a TS private member; its occurrence starts at the `#`.
        first => {
            let mut i = usize::from(first == Some(&'#'));
            i += rest[i..]
                .iter()
                .take_while(|c| c.is_alphanumeric() || **c == '_' || **c == '$')
                .count();
            i
        }
    };
    while rest.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    if rest.get(i) == Some(&'?') && rest.get(i + 1) == Some(&'.') {
        i += 2;
    }
    // Rust turbofish `f::<T>(…)`.
    if rest.get(i) == Some(&':') && rest.get(i + 1) == Some(&':') && rest.get(i + 2) == Some(&'<') {
        i += 2;
    }
    if let Some(open) = rest.get(i).copied().filter(|c| *c == '[' || *c == '<') {
        let close = if open == '[' { ']' } else { '>' };
        let mut depth = 0;
        let mut next_line = first + 1;
        loop {
            let Some(c) = rest.get(i).copied() else {
                // Unclosed at the end of the line: only a type argument list
                // continues onto the next one.
                match lines.get(next_line) {
                    Some(l) if open == '<' && depth > 0 && next_line <= first + 40 => {
                        rest.push('\n');
                        rest.extend(l.chars());
                        next_line += 1;
                        continue;
                    }
                    _ => break,
                }
            };
            if c == open {
                depth += 1;
            } else if c == close && !(open == '<' && i > 0 && matches!(rest[i - 1], '=' | '-')) {
                // The `>` of `=>` in `<(e: E) => void>` (or Rust's `->` in
                // `f::<fn() -> u32>`) closes nothing.
                depth -= 1;
                if depth == 0 {
                    i += 1;
                    break;
                }
            }
            i += 1;
        }
    }
    if matches!(rest.get(i), Some('(') | Some('`')) {
        UseKind::Call
    } else {
        UseKind::Value
    }
}

/// Index just past the string literal opening with `quote` at `chars[0]`.
fn string_literal_end(chars: &[char], quote: char) -> Option<usize> {
    let mut i = 1;
    while let Some(&c) = chars.get(i) {
        match c {
            '\\' => i += 2,
            c if c == quote => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// Whether the occurrence at `site` is the class operand of `new`:
/// `new C(`, `new ns.C(`.
fn is_new_site(
    cache: &mut std::collections::HashMap<PathBuf, Vec<String>>,
    site: &IndexTarget,
) -> bool {
    let lines = cache.entry(site.file.clone()).or_insert_with(|| {
        std::fs::read_to_string(&site.file)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    });
    let Some(line) = site.line.checked_sub(1).and_then(|i| lines.get(i as usize)) else {
        return false;
    };
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$' || c == '.';
    let before: String = line.chars().take(site.col as usize).collect();
    let before = before.trim_end_matches(ident).trim_end();
    before
        .strip_suffix("new")
        .is_some_and(|rest| !rest.chars().next_back().is_some_and(ident))
}

/// Identifier text starting at an index target's (1-based line, 0-based UTF-16
/// col) position. ASCII identifiers make the UTF-16 col equal the char index.
fn identifier_at(
    cache: &mut std::collections::HashMap<PathBuf, Vec<String>>,
    target: &IndexTarget,
) -> Option<String> {
    let lines = cache.entry(target.file.clone()).or_insert_with(|| {
        std::fs::read_to_string(&target.file)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    });
    let line = lines.get(target.line.checked_sub(1)? as usize)?;
    let ident: String = line
        .chars()
        .skip(target.col as usize)
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    (!ident.is_empty()).then_some(ident)
}

/// Parameters `(p)`, type parameters `[T]`, namespaces `ns/` and meta `m:`
/// never name a function; only terms, methods and locals can.
fn is_callable_symbol(symbol: &str) -> bool {
    symbol.starts_with("local ")
        || !(symbol.ends_with(')')
            || symbol.ends_with(']')
            || symbol.ends_with('/')
            || symbol.ends_with(':'))
}

/// Name of the last term/method/type descriptor in a SCIP symbol, parsed per
/// the SCIP grammar (`ns/`, `Type#`, `term.`, `method(disambig).`, `` `escaped` ``).
/// Namespace-nested symbols (`util/objectValues.`) need this: the naive
/// split on `.` returned the file name instead of the function.
fn last_descriptor_name(symbol: &str) -> Option<String> {
    let parts: Vec<&str> = symbol.splitn(5, ' ').collect();
    let descriptors = match parts.len() {
        5 => parts[4],
        _ => *parts.last()?,
    };
    let chars: Vec<char> = descriptors.chars().collect();
    let mut i = 0;
    let mut last: Option<String> = None;
    while i < chars.len() {
        let mut name = String::new();
        if chars[i] == '`' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '`' {
                    if chars.get(i + 1) == Some(&'`') {
                        name.push('`');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                name.push(chars[i]);
                i += 1;
            }
        } else {
            while i < chars.len() && (chars[i].is_alphanumeric() || "_$+-".contains(chars[i])) {
                name.push(chars[i]);
                i += 1;
            }
        }
        let suffix = chars.get(i).copied()?;
        match suffix {
            '/' | ':' | '!' => i += 1,
            '#' | '.' => {
                i += 1;
                last = Some(name);
            }
            '(' => {
                // `name(disambiguator).` is a method; `(param)` is a parameter.
                let close = chars[i..].iter().position(|c| *c == ')')? + i;
                i = close + 1;
                if !name.is_empty() && chars.get(i) == Some(&'.') {
                    i += 1;
                    last = Some(name);
                }
            }
            '[' => {
                let close = chars[i..].iter().position(|c| *c == ']')? + i;
                i = close + 1;
            }
            _ => return None,
        }
    }
    let name = last?;
    // A string-literal member name (`"~validate"() {}`) is escaped as
    // `` `"~validate"` ``; the syntax layer names the node without quotes.
    let name = match name.as_bytes() {
        [q @ (b'"' | b'\''), .., e] if q == e => name[1..name.len() - 1].to_string(),
        _ => name,
    };
    let name = match name.as_str() {
        "<constructor>" => "constructor".to_string(),
        _ => name
            .strip_prefix("<get>")
            .or_else(|| name.strip_prefix("<set>"))
            .map(str::to_string)
            .unwrap_or(name),
    };
    (!name.is_empty()).then_some(name)
}

#[async_trait]
impl<I: SemanticIndex + Send + Sync> SemanticResolver for IndexBackedSemanticResolver<I> {
    async fn resolve(&self, hints: &SyntaxResults) -> anyhow::Result<ResolvedEdges> {
        let mut out = ResolvedEdges::new();
        let lang = hints.language.as_deref().unwrap_or("");
        if !language_matches(&self.language, lang) {
            warn!(
                target: "index_backed::resolve",
                "IndexBackedSemanticResolver expected {}, got {lang:?}; returning empty edges",
                self.language
            );
            return Ok(out);
        }

        let symbol_index = SymbolIndex::new(&hints.symbols);
        let mut seen = std::collections::HashSet::new();

        if let Some(occurrences) = self.index.reference_occurrences() {
            // The index enumerates every use the compiler resolved, so it is
            // the sole authority: no caret guessing, no name-match fallback.
            for reference in &hints.references {
                if let UnresolvedReference::Call(call_site) = reference {
                    out.mark_call_site_resolved(call_site.location.clone());
                }
            }
            let occurrences = if language_matches("rust", &self.language) {
                drop_import_uses(occurrences, hints)
            } else {
                occurrences
            };
            self.resolve_from_occurrences(&occurrences, &symbol_index, &mut seen, &mut out);
            self.resolve_super_calls(hints, &symbol_index, &mut seen, &mut out);
            out.stats.compiler_unindexed_files = self.unindexed_files(hints);
            if language_matches("python", &self.language) {
                out.stats.compiler_unresolved_calls = self.unresolved_calls(hints);
            }
            // Heuristic import edges build a per-file name table that is
            // O(files x imported symbols) and, on Kubernetes-size Go repos,
            // runs for hours. Callers above never use it. Opt-out until the
            // binding table is rebuilt to scale.
            if std::env::var_os("GRIDSEAK_SKIP_IMPORT_HEURISTICS").is_none() {
                self.resolve_imports_heuristic(hints, &mut out);
            }
            return Ok(out);
        }

        for reference in &hints.references {
            let UnresolvedReference::Call(call_site) = reference else {
                continue;
            };
            let file = Path::new(&call_site.location.file);
            if !self.index.has_file(file) {
                out.mark_call_site_resolved(call_site.location.clone());
                continue;
            }
            match self.resolve_one(call_site, &symbol_index, &mut seen) {
                Some(SiteHit::Mapped(edge, location)) => {
                    out.mark_call_site_resolved(location);
                    out.add_call_edge(edge);
                    out.stats.compiler_edges += 1;
                    self.emitted_edges
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                Some(SiteHit::ResolvedOnly(location)) => {
                    // SCIP named the callee (often stdlib / another module).
                    // Suppress TreeSitter name-match siblings.
                    out.mark_call_site_resolved(location);
                }
                None => {}
            }
        }

        self.resolve_imports_heuristic(hints, &mut out);
        Ok(out)
    }

    fn supported_language(&self) -> &str {
        &self.language
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn session_metrics(&self) -> Option<SessionMetricsSnapshot> {
        None
    }

    async fn resolution_disclosure(&self) -> Option<ResolutionDisclosure> {
        let emitted = self
            .emitted_edges
            .load(std::sync::atomic::Ordering::Relaxed);
        Some(ResolutionDisclosure::layer2_active(&self.language, emitted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::SemanticIndex;
    use crate::domain::{Confidence, Node};

    struct MockIndex {
        target: Option<IndexTarget>,
        language: String,
    }

    impl SemanticIndex for MockIndex {
        fn definition_at(&self, _file: &Path, _line: u32, _col: u32) -> Option<IndexTarget> {
            self.target.clone()
        }

        fn language(&self) -> &str {
            &self.language
        }
    }

    #[tokio::test]
    async fn emits_compiler_edge_for_resolved_call() {
        let root = PathBuf::from("/fixture");
        let callee_range = Range::with_file(2, 0, 4, 1, "/fixture/src/index.ts");
        let caller_range = Range::with_file(5, 0, 7, 1, "/fixture/src/index.ts");
        let caller = Node::function("index.caller".into(), caller_range.clone());
        let callee = Node::function("index.callee".into(), callee_range);

        let call_site = CallSite {
            location: Range::with_file(6, 4, 6, 10, "/fixture/src/index.ts"),
            function_name: "callee".into(),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        };

        let index = MockIndex {
            target: Some(IndexTarget {
                file: PathBuf::from("/fixture/src/index.ts"),
                line: 2,
                col: 16,
                symbol_moniker: "callee".into(),
                confidence: Confidence::High,
            }),
            language: "typescript".into(),
        };
        let resolver = IndexBackedSemanticResolver::new(index, root, "typescript");

        let mut hints = SyntaxResults::new();
        hints.language = Some("typescript".into());
        hints.symbols = vec![caller.clone(), callee.clone()];
        hints
            .references
            .push(UnresolvedReference::Call(call_site.clone()));

        let edges = resolver.resolve(&hints).await.unwrap();
        assert_eq!(edges.call_edges.len(), 1);
        assert_eq!(
            edges.call_edges[0].provenance.source,
            ProvenanceSource::Compiler
        );
        assert_eq!(edges.stats.compiler_edges, 1);
        assert!(edges.resolved_call_sites.contains(&call_site.location));
    }

    /// Occurrence-mode index whose definitions are keyed by 0-based position.
    struct PositionIndex {
        defs: Vec<((u32, u32), IndexTarget)>,
        occurrences: Vec<(IndexTarget, IndexTarget)>,
    }

    impl SemanticIndex for PositionIndex {
        fn definition_at(&self, _file: &Path, line: u32, col: u32) -> Option<IndexTarget> {
            self.defs
                .iter()
                .find(|(at, _)| *at == (line, col))
                .map(|(_, t)| t.clone())
        }

        fn language(&self) -> &str {
            "typescript"
        }

        fn reference_occurrences(&self) -> Option<Vec<(IndexTarget, IndexTarget)>> {
            Some(self.occurrences.clone())
        }
    }

    #[tokio::test]
    async fn new_on_class_without_constructor_links_inherited_constructor() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.ts");
        std::fs::write(
            &file,
            "class Base {\n  constructor() { go() }\n}\nclass Mid extends Base {}\nfunction make() {\n  return new Mid()\n}\nfunction other() {\n  return Mid.name\n}\n",
        )
        .unwrap();
        let f = file.to_string_lossy().to_string();
        let at = |line, col, symbol: &str| IndexTarget {
            file: file.clone(),
            line,
            col,
            symbol_moniker: symbol.into(),
            confidence: Confidence::High,
        };
        let mid = "scip-typescript npm a 1.0 `a.ts`/Mid#";
        let index = PositionIndex {
            // `Base` in `class Mid extends Base` (0-based position).
            defs: vec![((3, 18), at(1, 6, "scip-typescript npm a 1.0 `a.ts`/Base#"))],
            // `new Mid()` and the value use `Mid.name`, both naming the class.
            occurrences: vec![
                (at(6, 13, mid), at(4, 6, mid)),
                (at(9, 9, mid), at(4, 6, mid)),
            ],
        };
        let resolver =
            IndexBackedSemanticResolver::new(index, dir.path().to_path_buf(), "typescript");
        let strukt = |name: &str, r: Range| {
            Node::new(
                crate::domain::NodeKind::Struct,
                format!("a::{name}"),
                r,
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
            )
        };
        let base_ctor = Node::function(
            "a::Base::constructor".into(),
            Range::with_file(2, 2, 2, 24, &f),
        );
        let make = Node::function("a::make".into(), Range::with_file(5, 0, 7, 1, &f));
        let mut hints = SyntaxResults::new();
        hints.language = Some("typescript".into());
        hints.source_files = vec![f.clone()];
        hints.symbols = vec![
            strukt("Base", Range::with_file(1, 0, 3, 1, &f)),
            base_ctor.clone(),
            strukt("Mid", Range::with_file(4, 0, 4, 25, &f)),
            make.clone(),
            Node::function("a::other".into(), Range::with_file(8, 0, 10, 1, &f)),
        ];
        let edges = resolver.resolve(&hints).await.unwrap();
        assert_eq!(edges.call_edges.len(), 1, "{:?}", edges.call_edges);
        let e = &edges.call_edges[0];
        assert_eq!((&e.from_id, &e.to_id), (&make.id, &base_ctor.id));
        assert_eq!(e.provenance.source, ProvenanceSource::Compiler);
        assert_eq!(e.provenance.via, Some(UseKind::Call));
    }

    #[tokio::test]
    async fn string_literal_element_access_call_links_the_member() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.ts");
        std::fs::write(
            &file,
            "class T {\n  \"~validate\"(d) { return d }\n  constructor() {\n    this.s = { validate: (d) => this[\"~validate\"](d) }\n  }\n}\n",
        )
        .unwrap();
        let f = file.to_string_lossy().to_string();
        let validate = "scip-typescript npm a 1.0 `a.ts`/T#`\"~validate\"`().";
        let at = |line, col| IndexTarget {
            file: file.clone(),
            line,
            col,
            symbol_moniker: validate.into(),
            confidence: Confidence::High,
        };
        let index = PositionIndex {
            defs: Vec::new(),
            // The occurrence sits on the string literal inside `this[…]`.
            occurrences: vec![(at(4, 37), at(2, 2))],
        };
        let resolver =
            IndexBackedSemanticResolver::new(index, dir.path().to_path_buf(), "typescript");
        let member = Node::function("a::~validate".into(), Range::with_file(2, 2, 2, 30, &f));
        let ctor = Node::function("a::constructor".into(), Range::with_file(3, 2, 5, 3, &f));
        let mut hints = SyntaxResults::new();
        hints.language = Some("typescript".into());
        hints.source_files = vec![f.clone()];
        hints.symbols = vec![member.clone(), ctor.clone()];
        let edges = resolver.resolve(&hints).await.unwrap();
        assert_eq!(edges.call_edges.len(), 1, "{:?}", edges.call_edges);
        let e = &edges.call_edges[0];
        assert_eq!((&e.from_id, &e.to_id), (&ctor.id, &member.id));
        assert_eq!(e.provenance.source, ProvenanceSource::Compiler);
        assert_eq!(e.provenance.via, Some(UseKind::Call));
    }

    #[tokio::test]
    async fn super_call_links_to_nearest_base_constructor() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.ts");
        std::fs::write(
            &file,
            "class Base {\n  constructor() { go() }\n}\nclass Mid extends Base {}\nclass Sub extends Mid {\n  constructor() {\n    super()\n  }\n}\n",
        )
        .unwrap();
        let f = file.to_string_lossy().to_string();
        let class_at = |line| IndexTarget {
            file: file.clone(),
            line,
            col: 6,
            symbol_moniker: String::new(),
            confidence: Confidence::High,
        };
        let index = PositionIndex {
            // `Mid` in `class Sub extends Mid`, `Base` in `class Mid extends Base`.
            defs: vec![((4, 18), class_at(4)), ((3, 18), class_at(1))],
            occurrences: Vec::new(),
        };
        let resolver =
            IndexBackedSemanticResolver::new(index, dir.path().to_path_buf(), "typescript");
        let strukt = |name: &str, r: Range| {
            Node::new(
                crate::domain::NodeKind::Struct,
                format!("a::{name}"),
                r,
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
            )
        };
        let base_ctor = Node::function(
            "a::Base::constructor".into(),
            Range::with_file(2, 2, 2, 24, &f),
        );
        let sub_ctor = Node::function(
            "a::Sub::constructor".into(),
            Range::with_file(6, 2, 8, 3, &f),
        );
        let mut hints = SyntaxResults::new();
        hints.language = Some("typescript".into());
        hints.source_files = vec![f.clone()];
        hints.symbols = vec![
            strukt("Base", Range::with_file(1, 0, 3, 1, &f)),
            base_ctor.clone(),
            strukt("Mid", Range::with_file(4, 0, 4, 25, &f)),
            strukt("Sub", Range::with_file(5, 0, 9, 1, &f)),
            sub_ctor.clone(),
        ];
        let edges = resolver.resolve(&hints).await.unwrap();
        assert_eq!(edges.call_edges.len(), 1, "{:?}", edges.call_edges);
        let e = &edges.call_edges[0];
        assert_eq!((&e.from_id, &e.to_id), (&sub_ctor.id, &base_ctor.id));
        assert_eq!(e.provenance.via, Some(UseKind::Call));
    }

    #[test]
    fn use_kind_classifies_typescript_use_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.ts");
        std::fs::write(
            &file,
            "f(1)\nx.f<T>(a)\nf?.(1)\nnew F(1)\nhtml`<p>${v}</p>`\nfoo(f)\nf.call(t)\n<F />\nthis.#m(1)\nuse<(e: E) => void>(h)\nget<{\n  a: A\n}>(c)\nthis[\"~v\"](d)\nx = this['k'].y\nthis[ \"a\" ] (1)\n",
        )
        .unwrap();
        let mut cache = std::collections::HashMap::new();
        let kind = |cache: &mut _, line: u32, col: u32| {
            use_kind_at(
                cache,
                &IndexTarget {
                    file: file.clone(),
                    line,
                    col,
                    symbol_moniker: String::new(),
                    confidence: Confidence::High,
                },
            )
        };
        for (line, col) in [
            (1, 0),
            (2, 2),
            (3, 0),
            (4, 4),
            (5, 0),
            (9, 5),
            (10, 0),
            (11, 0),
            (14, 5),
            (16, 6),
        ] {
            assert_eq!(kind(&mut cache, line, col), UseKind::Call, "line {line}");
        }
        for (line, col) in [(6, 4), (7, 0), (8, 1), (15, 9)] {
            assert_eq!(kind(&mut cache, line, col), UseKind::Value, "line {line}");
        }
    }

    #[test]
    fn short_symbol_name_reads_scip_descriptor_not_file() {
        assert_eq!(
            short_symbol_name("scip-typescript npm zod 3.24.2 src/`types.ts`/ZodType#_parse()."),
            "_parse"
        );
        assert_eq!(
            short_symbol_name(
                "scip-go gomod github.com/go-chi/chi/v5 v5.2.1 `github.com/go-chi/chi/v5`/NewRouter()."
            ),
            "NewRouter"
        );
        assert_eq!(
            short_symbol_name(
                "scip-go gomod github.com/go-chi/chi/v5 v5.2.1 `github.com/go-chi/chi/v5`/Mux#Use()."
            ),
            "Use"
        );
        assert_eq!(
            short_symbol_name("scip-typescript npm . `index`.callee()."),
            "callee"
        );
        assert_eq!(
            short_symbol_name(
                "scip-go gomod github.com/golang/go/src go1.20 `net/http`/HandlerFunc#"
            ),
            "HandlerFunc"
        );
    }

    #[test]
    fn short_symbol_name_handles_namespace_nested_terms() {
        assert_eq!(
            short_symbol_name(
                "scip-typescript npm zod 3.24.2 src/helpers/`util.ts`/util/objectValues."
            ),
            "objectValues"
        );
        assert_eq!(
            short_symbol_name(
                "scip-typescript npm zod 3.24.2 src/`types.ts`/ZodType#`<constructor>`()."
            ),
            "constructor"
        );
        assert_eq!(
            short_symbol_name("scip-python python pkg 1.0 `pkg.mod`/Klass#method()."),
            "method"
        );
        assert_eq!(
            short_symbol_name(
                "scip-typescript npm zod 3.24.2 src/`types.ts`/ZodType#`\"~validate\"`()."
            ),
            "~validate"
        );
    }

    #[tokio::test]
    async fn scip_hit_without_in_repo_callee_still_marks_resolved() {
        let root = PathBuf::from("/fixture");
        let caller_range = Range::with_file(5, 0, 7, 1, "/fixture/src/lib.go");
        let caller = Node::function("lib.wrap".into(), caller_range);
        let call_site = CallSite {
            location: Range::with_file(6, 4, 6, 20, "/fixture/src/lib.go"),
            function_name: "method_call:HandlerFunc".into(),
            receiver_range: Some(Range::with_file(6, 4, 6, 8, "/fixture/src/lib.go")),
            receiver_text: Some("http".into()),
            arg_types: Vec::new(),
        };
        let index = MockIndex {
            target: Some(IndexTarget {
                file: PathBuf::from("/usr/lib/go/src/net/http/server.go"),
                line: 10,
                col: 0,
                symbol_moniker:
                    "scip-go gomod github.com/golang/go/src go1.20 `net/http`/HandlerFunc#".into(),
                confidence: Confidence::High,
            }),
            language: "go".into(),
        };
        let resolver = IndexBackedSemanticResolver::new(index, root, "go");
        let mut hints = SyntaxResults::new();
        hints.language = Some("go".into());
        hints.symbols = vec![caller];
        hints
            .references
            .push(UnresolvedReference::Call(call_site.clone()));

        let edges = resolver.resolve(&hints).await.unwrap();
        assert!(edges.call_edges.is_empty());
        assert!(edges.resolved_call_sites.contains(&call_site.location));
    }

    struct EmptyIndex;

    impl SemanticIndex for EmptyIndex {
        fn definition_at(&self, _file: &Path, _line: u32, _col: u32) -> Option<IndexTarget> {
            None
        }

        fn has_file(&self, _file: &Path) -> bool {
            false
        }

        fn language(&self) -> &str {
            "typescript"
        }
    }

    #[tokio::test]
    async fn unindexed_file_suppresses_fallback_without_inventing_edge() {
        let root = PathBuf::from("/fixture");
        let call_site = CallSite {
            location: Range::with_file(6, 4, 6, 10, "/fixture/deno/lib/types.ts"),
            function_name: "method_call:parse".into(),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        };
        let resolver = IndexBackedSemanticResolver::new(EmptyIndex, root, "typescript");
        let mut hints = SyntaxResults::new();
        hints.language = Some("typescript".into());
        hints
            .references
            .push(UnresolvedReference::Call(call_site.clone()));
        let edges = resolver.resolve(&hints).await.unwrap();
        assert!(edges.call_edges.is_empty());
        assert!(edges.resolved_call_sites.contains(&call_site.location));
    }

    /// scip-python-shaped index: definitions by symbol, uses by position.
    struct PyIndex {
        file: PathBuf,
        defs: Vec<(String, (u32, u32))>,
        /// 0-based position -> symbol, for `definition_at` (base classes).
        at: Vec<((u32, u32), String)>,
        /// 1-based use sites.
        uses: Vec<((u32, u32), String)>,
    }

    impl PyIndex {
        fn target(&self, symbol: &str, (line, col): (u32, u32)) -> IndexTarget {
            IndexTarget {
                file: self.file.clone(),
                line,
                col,
                symbol_moniker: symbol.into(),
                confidence: Confidence::High,
            }
        }
    }

    impl SemanticIndex for PyIndex {
        fn definition_at(&self, _file: &Path, line: u32, col: u32) -> Option<IndexTarget> {
            let (_, sym) = self.at.iter().find(|(p, _)| *p == (line, col))?;
            self.definition_of(sym)
        }
        fn definition_of(&self, symbol: &str) -> Option<IndexTarget> {
            let (_, pos) = self.defs.iter().find(|(s, _)| s == symbol)?;
            Some(self.target(symbol, *pos))
        }
        fn declarations_of(&self, symbol: &str) -> Vec<IndexTarget> {
            self.definition_of(symbol).into_iter().collect()
        }
        fn language(&self) -> &str {
            "python"
        }
        fn reference_occurrences(&self) -> Option<Vec<(IndexTarget, IndexTarget)>> {
            Some(
                self.uses
                    .iter()
                    .filter_map(|(pos, sym)| {
                        Some((self.target(sym, *pos), self.definition_of(sym)?))
                    })
                    .collect(),
            )
        }
        fn occurrence_starts(&self, _file: &Path) -> Option<Vec<(u32, u32, String)>> {
            let mut out: Vec<(u32, u32, String)> = self
                .uses
                .iter()
                .map(|((l, c), s)| (l - 1, *c, s.clone()))
                .collect();
            out.extend(self.defs.iter().map(|(s, (l, c))| (l - 1, *c, s.clone())));
            Some(out)
        }
    }

    #[tokio::test]
    async fn python_calls_follow_runtime_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("m.py");
        std::fs::write(
            &file,
            "class Base:\n    def __init__(self): pass\nclass Mid(Base):\n    pass\n@overload\ndef f(x: int) -> int: ...\ndef f(x): return x\nclass A(ABC):\n    @abstractmethod\n    def run(self): ...\nclass Request:\n    def __init__(self): pass\nclass Response:\n    def __init__(self): pass\ndef make(a, pkg):\n    Mid()\n    f(1)\n    a.run()\n    pkg.Response()\n    a.unknown()\n    return Mid\n",
        )
        .unwrap();
        let f = file.to_string_lossy().to_string();
        let s = |d: &str| format!("scip-python python pkg 1.0 `m`/{d}");
        let index = PyIndex {
            file: file.clone(),
            defs: vec![
                (s("Base#"), (1, 6)),
                (s("Base#__init__()."), (2, 8)),
                (s("Mid#"), (3, 6)),
                (s("f()."), (6, 4)),
                (s("A#"), (8, 6)),
                (s("A#run()."), (10, 8)),
                (s("Request#"), (11, 6)),
                (s("Request#__init__()."), (12, 8)),
                (s("Response#"), (13, 6)),
                (s("Response#__init__()."), (14, 8)),
            ],
            // `Base` in `class Mid(Base)`; `ABC` is external (no definition).
            at: vec![((2, 10), s("Base#"))],
            uses: vec![
                ((16, 4), s("Mid#")),
                ((17, 4), s("f().")),
                // the implementation's own name, recorded as a use of the stub
                ((7, 4), s("f().")),
                ((18, 6), s("A#run().")),
                // a star-imported name misnamed as its module's first name
                ((19, 8), s("Request#")),
                ((21, 11), s("Mid#")),
            ],
        };
        let resolver = IndexBackedSemanticResolver::new(index, dir.path().to_path_buf(), "python");
        let strukt = |name: &str, r: Range| {
            Node::new(
                crate::domain::NodeKind::Struct,
                format!("m::{name}"),
                r,
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
            )
        };
        let base_init = Node::function(
            "m::Base::__init__".into(),
            Range::with_file(2, 4, 2, 28, &f),
        );
        let stub = Node::function("m::stub::f".into(), Range::with_file(6, 0, 6, 25, &f));
        let imp = Node::function("m::f".into(), Range::with_file(7, 0, 7, 18, &f));
        let run = Node::function("m::A::run".into(), Range::with_file(10, 4, 10, 22, &f));
        let req_init = Node::function(
            "m::Request::__init__".into(),
            Range::with_file(12, 4, 12, 28, &f),
        );
        let resp_init = Node::function(
            "m::Response::__init__".into(),
            Range::with_file(14, 4, 14, 28, &f),
        );
        let make = Node::function("m::make".into(), Range::with_file(15, 0, 21, 14, &f));
        let mut hints = SyntaxResults::new();
        hints.language = Some("python".into());
        hints.source_files = vec![f.clone()];
        hints.symbols = vec![
            strukt("Base", Range::with_file(1, 0, 2, 28, &f)),
            base_init.clone(),
            strukt("Mid", Range::with_file(3, 0, 4, 8, &f)),
            stub.clone(),
            imp.clone(),
            strukt("A", Range::with_file(8, 0, 10, 22, &f)),
            run.clone(),
            strukt("Request", Range::with_file(11, 0, 12, 28, &f)),
            req_init.clone(),
            strukt("Response", Range::with_file(13, 0, 14, 28, &f)),
            resp_init.clone(),
            make.clone(),
        ];
        let edges = resolver.resolve(&hints).await.unwrap();
        let mut got: Vec<(String, String, Option<UseKind>)> = edges
            .call_edges
            .iter()
            .map(|e| (e.from_id.clone(), e.to_id.clone(), e.provenance.via))
            .collect();
        got.sort();
        let mut want = vec![
            // `Mid()` runs the `__init__` Mid inherits from Base.
            (make.id.clone(), base_init.id.clone(), Some(UseKind::Call)),
            // `f(1)` runs the implementation, not the `@overload` stub.
            (make.id.clone(), imp.id.clone(), Some(UseKind::Call)),
            // `a.run()` goes through an abstract method.
            (make.id.clone(), run.id.clone(), Some(UseKind::Dispatch)),
            // `pkg.Response()` is Response, whatever the index named it.
            (make.id.clone(), resp_init.id.clone(), Some(UseKind::Call)),
        ];
        want.sort();
        assert_eq!(got, want);
        assert_eq!(
            edges.stats.compiler_unresolved_calls,
            vec![("m.py".to_string(), 20, "unknown".to_string())]
        );
    }
}
