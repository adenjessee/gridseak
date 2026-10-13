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
        for (site, def) in occurrences {
            if !file_is_under(&def.file, &self.workspace_root)
                || !is_callable_symbol(&def.symbol_moniker)
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
            let callees: Vec<(&crate::domain::Node, crate::domain::Confidence, UseKind)> =
                match direct {
                    Some(node) => {
                        let via = use_kind_at(&mut source_cache, site);
                        vec![(node, def.confidence, via)]
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
/// `f(`, `x.f(`, `f[T](` (Go), `f<T>(` (TS), `f?.(` — else `Value`.
fn use_kind_at(
    cache: &mut std::collections::HashMap<PathBuf, Vec<String>>,
    site: &IndexTarget,
) -> UseKind {
    let lines = cache.entry(site.file.clone()).or_insert_with(|| {
        std::fs::read_to_string(&site.file)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    });
    let Some(line) = site.line.checked_sub(1).and_then(|i| lines.get(i as usize)) else {
        return UseKind::Value;
    };
    let rest: Vec<char> = line.chars().skip(site.col as usize).collect();
    let mut i = rest
        .iter()
        .take_while(|c| c.is_alphanumeric() || **c == '_' || **c == '$')
        .count();
    while rest.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    if rest.get(i) == Some(&'?') && rest.get(i + 1) == Some(&'.') {
        i += 2;
    }
    if let Some(open) = rest.get(i).copied().filter(|c| *c == '[' || *c == '<') {
        let close = if open == '[' { ']' } else { '>' };
        let mut depth = 0;
        while let Some(c) = rest.get(i) {
            if *c == open {
                depth += 1;
            } else if *c == close {
                depth -= 1;
                if depth == 0 {
                    i += 1;
                    break;
                }
            }
            i += 1;
        }
    }
    if rest.get(i) == Some(&'(') {
        UseKind::Call
    } else {
        UseKind::Value
    }
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
            self.resolve_from_occurrences(&occurrences, &symbol_index, &mut seen, &mut out);
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
}
