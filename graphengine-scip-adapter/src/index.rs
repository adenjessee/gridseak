//! In-memory SCIP occurrence table and definition-at-position queries.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use protobuf::Enum;
use protobuf::Message;
use scip::types::{Document, Index, Occurrence, SymbolRole};
use thiserror::Error;

/// Errors loading or querying a SCIP index.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScipIndexError {
    #[error("failed to read index file: {0}")]
    Io(String),
    #[error("failed to parse SCIP index: {0}")]
    Parse(String),
    #[error("index metadata missing project_root")]
    MissingProjectRoot,
}

/// Half-open `[start, end)` range in SCIP coordinates (0-based line/character).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedRange {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

impl DecodedRange {
    pub fn contains(&self, line: u32, character: u32) -> bool {
        if line < self.start_line || line > self.end_line {
            return false;
        }
        if line == self.start_line && character < self.start_character {
            return false;
        }
        if line == self.end_line && character >= self.end_character {
            return false;
        }
        true
    }
}

/// A definition occurrence resolved from the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScipDefinition {
    pub relative_path: String,
    pub line: u32,
    pub character: u32,
    pub symbol: String,
}

#[derive(Debug, Clone)]
struct DocumentOccurrences {
    relative_path: String,
    occurrences: Vec<Occurrence>,
}

/// Upper bound on documents stat'ed by [`ScipIndex::workspace_coverage`].
pub const COVERAGE_SAMPLE_LIMIT: usize = 2000;

/// Below this fraction of document paths present in the workspace, an index
/// is treated as belonging to another checkout.
pub const MIN_WORKSPACE_COVERAGE: f64 = 0.5;

/// Result of checking an index's document paths against a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentCoverage {
    /// Documents in the index.
    pub total: usize,
    /// Documents whose path was checked (sampled when `total` is large).
    pub checked: usize,
    /// Checked documents whose file exists under the workspace.
    pub present: usize,
}

impl DocumentCoverage {
    /// Fraction of checked documents present; `None` when the index is empty.
    pub fn ratio(&self) -> Option<f64> {
        (self.checked > 0).then(|| self.present as f64 / self.checked as f64)
    }

    /// True when the index is empty or fewer than [`MIN_WORKSPACE_COVERAGE`]
    /// of its documents exist in the workspace.
    pub fn looks_foreign(&self) -> bool {
        self.ratio().is_none_or(|r| r < MIN_WORKSPACE_COVERAGE)
    }
}

/// Parsed SCIP index with occurrence lookup tables.
#[derive(Debug, Clone)]
pub struct ScipIndex {
    project_root: PathBuf,
    by_relative_path: HashMap<String, DocumentOccurrences>,
    definitions_by_symbol: HashMap<String, ScipDefinition>,
    /// Every definition occurrence of symbols defined more than once.
    /// scip-python reuses one symbol for same-named functions nested in
    /// different methods of a class (`Group#decorator().`).
    repeated_definitions: HashMap<String, Vec<ScipDefinition>>,
    implementations_by_symbol: HashMap<String, Vec<String>>,
    /// Reverse of `implementations_by_symbol`: method -> interface methods it satisfies.
    implemented_by_symbol: HashMap<String, Vec<String>>,
    references_by_symbol: HashMap<String, Vec<ScipDefinition>>,
}

impl ScipIndex {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ScipIndexError> {
        let bytes = std::fs::read(path.as_ref()).map_err(|e| ScipIndexError::Io(e.to_string()))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ScipIndexError> {
        let index =
            Index::parse_from_bytes(bytes).map_err(|e| ScipIndexError::Parse(e.to_string()))?;
        Self::from_index(index)
    }

    pub fn from_index(index: Index) -> Result<Self, ScipIndexError> {
        let project_root = normalize_project_root(&index.metadata.project_root);
        if project_root.as_os_str().is_empty() {
            return Err(ScipIndexError::MissingProjectRoot);
        }

        let mut by_relative_path = HashMap::new();
        let mut definitions_by_symbol: HashMap<String, ScipDefinition> = HashMap::new();
        let mut repeated_definitions: HashMap<String, Vec<ScipDefinition>> = HashMap::new();
        let mut implementations_by_symbol: HashMap<String, Vec<String>> = HashMap::new();
        let mut implemented_by_symbol: HashMap<String, Vec<String>> = HashMap::new();
        let mut references_by_symbol: HashMap<String, Vec<ScipDefinition>> = HashMap::new();
        for doc in index.documents {
            for info in &doc.symbols {
                for rel in &info.relationships {
                    if rel.is_implementation {
                        implementations_by_symbol
                            .entry(rel.symbol.clone())
                            .or_default()
                            .push(info.symbol.clone());
                        implemented_by_symbol
                            .entry(info.symbol.clone())
                            .or_default()
                            .push(rel.symbol.clone());
                    }
                }
            }
            let entry = document_entry(doc);
            for occ in &entry.occurrences {
                let Some(range) = decode_occurrence_range(occ) else {
                    continue;
                };
                let loc = ScipDefinition {
                    relative_path: entry.relative_path.clone(),
                    line: range.start_line,
                    character: range.start_character,
                    symbol: occ.symbol.clone(),
                };
                if has_definition_role(occ.symbol_roles) {
                    if let Some(prev) =
                        definitions_by_symbol.insert(occ.symbol.clone(), loc.clone())
                    {
                        let all = repeated_definitions.entry(occ.symbol.clone()).or_default();
                        if all.is_empty() {
                            all.push(prev);
                        }
                        all.push(loc);
                    }
                } else {
                    references_by_symbol
                        .entry(occ.symbol.clone())
                        .or_default()
                        .push(loc);
                }
            }
            by_relative_path.insert(normalize_rel_path(&entry.relative_path), entry);
        }
        link_rust_trait_impls(
            &definitions_by_symbol,
            &references_by_symbol,
            &mut implementations_by_symbol,
            &mut implemented_by_symbol,
        );
        Ok(Self {
            project_root,
            by_relative_path,
            definitions_by_symbol,
            repeated_definitions,
            implementations_by_symbol,
            implemented_by_symbol,
            references_by_symbol,
        })
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Override the index metadata root (e.g. align a committed fixture with the scan workspace).
    pub fn with_project_root(mut self, root: PathBuf) -> Self {
        self.project_root = root;
        self
    }

    /// Test/diagnostics: list occurrences in a document by relative path.
    #[doc(hidden)]
    pub fn occurrences_in(&self, relative_path: &str) -> Vec<(String, i32, DecodedRange)> {
        let key = normalize_rel_path(relative_path);
        let Some(doc) = self.by_relative_path.get(&key) else {
            return Vec::new();
        };
        doc.occurrences
            .iter()
            .filter_map(|occ| {
                decode_occurrence_range(occ).map(|r| (occ.symbol.clone(), occ.symbol_roles, r))
            })
            .collect()
    }

    /// Number of documents in the index.
    pub fn document_count(&self) -> usize {
        self.by_relative_path.len()
    }

    /// How many of the index's document paths exist under `workspace`.
    ///
    /// Checks at most [`COVERAGE_SAMPLE_LIMIT`] documents (evenly spaced over
    /// the sorted paths) so huge indexes stay cheap. Use
    /// [`DocumentCoverage::looks_foreign`] to decide whether the index was
    /// generated for a different checkout.
    pub fn workspace_coverage(&self, workspace: &Path) -> DocumentCoverage {
        let mut paths: Vec<&str> = self
            .by_relative_path
            .values()
            .map(|d| d.relative_path.as_str())
            .collect();
        paths.sort_unstable();
        let total = paths.len();
        let stride = total.div_ceil(COVERAGE_SAMPLE_LIMIT).max(1);
        let mut checked = 0;
        let mut present = 0;
        for rel in paths.iter().step_by(stride) {
            checked += 1;
            if workspace.join(rel).is_file() {
                present += 1;
            }
        }
        DocumentCoverage {
            total,
            checked,
            present,
        }
    }

    /// True when `file` maps to a document that actually exists in the index.
    pub fn has_document(&self, file: &Path) -> bool {
        self.relative_path_for(file)
            .is_some_and(|rel| self.by_relative_path.contains_key(&rel))
    }

    /// Resolve the definition of the symbol referenced at `(line, character)`.
    ///
    /// Coordinates follow SCIP: **0-based** line and character offsets.
    pub fn definition_at(&self, file: &Path, line: u32, character: u32) -> Option<ScipDefinition> {
        let rel = self.relative_path_for(file)?;
        let doc = self.by_relative_path.get(&rel)?;
        for symbol in symbols_at_call_site(doc, line, character) {
            if let Some(def) = self.definitions_by_symbol.get(&symbol) {
                return Some(def.clone());
            }
        }
        None
    }

    /// Definition occurrence for a SCIP moniker, if present.
    pub fn definition_of(&self, symbol: &str) -> Option<ScipDefinition> {
        self.definitions_by_symbol.get(symbol).cloned()
    }

    /// Module-level classes and functions named `name` (`…/name#`,
    /// `…/name().`) that the index defines.
    pub fn module_level_definitions(&self, name: &str) -> Vec<ScipDefinition> {
        let class = format!("/{name}#");
        let func = format!("/{name}().");
        let mut out: Vec<ScipDefinition> = self
            .definitions_by_symbol
            .iter()
            .filter(|(s, _)| s.ends_with(&class) || s.ends_with(&func))
            .map(|(_, d)| d.clone())
            .collect();
        out.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        out
    }

    /// Start position (0-based line, character) and symbol of every
    /// occurrence in `file`'s document, or `None` when the index has no
    /// document for it.
    pub fn occurrence_starts(&self, file: &Path) -> Option<Vec<(u32, u32, String)>> {
        let rel = self.relative_path_for(file)?;
        let doc = self.by_relative_path.get(&rel)?;
        Some(
            doc.occurrences
                .iter()
                .filter_map(|o| {
                    decode_occurrence_range(o)
                        .map(|r| (r.start_line, r.start_character, o.symbol.clone()))
                })
                .collect(),
        )
    }

    /// Every definition occurrence of `symbol`: usually one, several when the
    /// indexer gave distinct same-named declarations one symbol.
    pub fn definitions_of(&self, symbol: &str) -> Vec<ScipDefinition> {
        match self.repeated_definitions.get(symbol) {
            Some(all) => all.clone(),
            None => self.definition_of(symbol).into_iter().collect(),
        }
    }

    /// Symbols that implement `symbol` (SCIP `is_implementation` relationships).
    /// Callers split confidence 1/n across the may-call set.
    pub fn implementations_of(&self, symbol: &str) -> Vec<String> {
        self.implementations_by_symbol
            .get(symbol)
            .cloned()
            .unwrap_or_default()
    }

    /// Interface/abstract methods that `symbol` implements, in or outside the
    /// repo. Removing such a method can break the type's contract even when
    /// nothing calls it by name.
    pub fn interfaces_implemented_by(&self, symbol: &str) -> Vec<String> {
        self.implemented_by_symbol
            .get(symbol)
            .cloned()
            .unwrap_or_default()
    }

    /// Non-definition occurrences of `symbol` (second witness for callers).
    pub fn references_of(&self, symbol: &str) -> Vec<ScipDefinition> {
        self.references_by_symbol
            .get(symbol)
            .cloned()
            .unwrap_or_default()
    }

    /// Every non-definition occurrence paired with its in-index definition.
    ///
    /// This is the compiler's full use graph: calls, function values, and
    /// method references alike. Symbols defined outside the index (stdlib,
    /// dependencies) are omitted because they have no definition here.
    pub fn reference_occurrences(&self) -> Vec<(ScipDefinition, ScipDefinition)> {
        let mut out = Vec::new();
        for (symbol, refs) in &self.references_by_symbol {
            // `local N` monikers are document-scoped and collide across files.
            if symbol.starts_with("local ") {
                continue;
            }
            let Some(def) = self
                .definitions_by_symbol
                .get(symbol)
                .cloned()
                .or_else(|| self.external_trait_method_anchor(symbol))
            else {
                continue;
            };
            let def = &def;
            // rust-analyzer names symbols by package, not by crate target, so
            // `helper()` at the root of two integration tests (or two
            // `src/bin/*.rs`) is one symbol with two definitions. Pair each
            // use with the definition nearest to it: same file first, then
            // the longest shared directory prefix.
            let repeated = symbol
                .starts_with("rust-analyzer ")
                .then(|| self.repeated_definitions.get(symbol))
                .flatten();
            for r in refs {
                let def = repeated
                    .and_then(|all| nearest_definition(all, &r.relative_path))
                    .unwrap_or(def);
                out.push((r.clone(), def.clone()));
            }
        }
        for doc in self.by_relative_path.values() {
            let mut local_defs: HashMap<&str, ScipDefinition> = HashMap::new();
            let mut local_refs: Vec<(&str, ScipDefinition)> = Vec::new();
            for occ in &doc.occurrences {
                if !occ.symbol.starts_with("local ") {
                    continue;
                }
                let Some(range) = decode_occurrence_range(occ) else {
                    continue;
                };
                let loc = ScipDefinition {
                    relative_path: doc.relative_path.clone(),
                    line: range.start_line,
                    character: range.start_character,
                    symbol: occ.symbol.clone(),
                };
                if has_definition_role(occ.symbol_roles) {
                    local_defs.insert(occ.symbol.as_str(), loc);
                } else {
                    local_refs.push((occ.symbol.as_str(), loc));
                }
            }
            for (symbol, r) in local_refs {
                if let Some(def) = local_defs.get(symbol) {
                    out.push((r, def.clone()));
                }
            }
        }
        out
    }

    /// A use of a trait method the index does not define (`serde::Serializer`
    /// via `S: Serializer`) that the workspace implements: anchored at the
    /// first in-index implementation but keeping the trait method's symbol,
    /// so the resolver treats it as dispatch and fans out to every impl.
    fn external_trait_method_anchor(&self, symbol: &str) -> Option<ScipDefinition> {
        if !matches!(
            rust_method_shape(symbol),
            Some(RustMethod::TraitDecl { .. })
        ) {
            return None;
        }
        let first = self.implementations_by_symbol.get(symbol)?.first()?;
        let at = self.definitions_by_symbol.get(first)?;
        Some(ScipDefinition {
            symbol: symbol.to_string(),
            ..at.clone()
        })
    }

    fn relative_path_for(&self, file: &Path) -> Option<String> {
        if let Ok(rel) = file.strip_prefix(&self.project_root) {
            return Some(normalize_rel_path(&rel.to_string_lossy()));
        }
        let file_name = file.file_name()?.to_string_lossy();
        for key in self.by_relative_path.keys() {
            if key.ends_with(file_name.as_ref()) || file_name.ends_with(key.as_str()) {
                return Some(key.clone());
            }
        }
        self.by_relative_path
            .get(&normalize_rel_path(&file.to_string_lossy()))
            .map(|d| d.relative_path.clone())
    }
}

/// Of several definitions of one symbol, the one in `site_path`'s file, else
/// the one sharing the most leading path components with it.
fn nearest_definition<'a>(
    defs: &'a [ScipDefinition],
    site_path: &str,
) -> Option<&'a ScipDefinition> {
    if let Some(same) = defs.iter().find(|d| d.relative_path == site_path) {
        return Some(same);
    }
    let shared = |d: &ScipDefinition| {
        d.relative_path
            .split('/')
            .zip(site_path.split('/'))
            .take_while(|(a, b)| a == b)
            .count()
    };
    defs.iter().max_by_key(|d| shared(d))
}

/// rust-analyzer's SCIP output records no `is_implementation` relationships,
/// but its symbols name them: `m/impl#[Circle][Area]area().` is `Area::area`
/// implemented for `Circle`, and `m/Area#area().` is the trait's own method.
/// Link each defined trait method to the defined impl methods with the same
/// trait name and method name, so a call through `dyn Area` or `T: Area`
/// can fan out to the implementations. Traits are matched by their last path
/// segment (the impl descriptor carries no more), so two same-named traits
/// share implementations; callers lower confidence when there are several.
///
/// Methods of traits defined outside the index (`serde::Serializer`,
/// `Iterator`) are linked too when they are referenced: a call through
/// `S: Serializer` may run the workspace's own `impl Serializer for …`.
fn link_rust_trait_impls(
    definitions: &HashMap<String, ScipDefinition>,
    referenced: &HashMap<String, Vec<ScipDefinition>>,
    implementations: &mut HashMap<String, Vec<String>>,
    implemented_by: &mut HashMap<String, Vec<String>>,
) {
    let mut impls_by_key: HashMap<(String, String), Vec<&str>> = HashMap::new();
    let mut trait_methods: Vec<(&str, (String, String))> = Vec::new();
    for symbol in definitions.keys() {
        match rust_method_shape(symbol) {
            Some(RustMethod::TraitImpl { trait_name, method }) => impls_by_key
                .entry((trait_name, method))
                .or_default()
                .push(symbol),
            Some(RustMethod::TraitDecl { trait_name, method }) => {
                trait_methods.push((symbol, (trait_name, method)))
            }
            None => {}
        }
    }
    for symbol in referenced.keys() {
        // Standard-library traits (`Clone::clone`, `Default::default`,
        // `Iterator::next`) are called generically everywhere; fanning those
        // out to every workspace impl would rebuild the name-matched
        // `clone` / `is_empty` hotspots. Only third-party traits fan out.
        let package = symbol.split(' ').nth(2).unwrap_or("");
        if definitions.contains_key(symbol)
            || matches!(package, "core" | "alloc" | "std" | "proc_macro" | "test")
        {
            continue;
        }
        if let Some(RustMethod::TraitDecl { trait_name, method }) = rust_method_shape(symbol) {
            trait_methods.push((symbol, (trait_name, method)));
        }
    }
    for (trait_method, key) in trait_methods {
        if implementations.contains_key(trait_method) {
            continue;
        }
        let Some(impls) = impls_by_key.get(&key) else {
            continue;
        };
        let mut impls: Vec<String> = impls.iter().map(|s| s.to_string()).collect();
        impls.sort();
        for imp in &impls {
            implemented_by
                .entry(imp.clone())
                .or_default()
                .push(trait_method.to_string());
        }
        implementations.insert(trait_method.to_string(), impls);
    }
}

/// Method shapes in rust-analyzer SCIP symbols.
#[derive(Debug, PartialEq, Eq)]
pub enum RustMethod {
    /// `…/Trait#method().`: declared (with or without a default body) in a trait.
    TraitDecl { trait_name: String, method: String },
    /// `…/impl#[Type][Trait]method().`: a trait method implemented for a type.
    TraitImpl { trait_name: String, method: String },
}

/// Classify a rust-analyzer SCIP symbol as a trait method declaration or a
/// trait-impl method. Inherent methods (`impl#[Type]m().`), free functions,
/// and other schemes return `None`.
pub fn rust_method_shape(symbol: &str) -> Option<RustMethod> {
    if !symbol.starts_with("rust-analyzer ") {
        return None;
    }
    let descriptors = symbol.splitn(5, ' ').nth(4)?;
    let body = descriptors.strip_suffix("().")?;
    // Method name: trailing identifier (or backtick-escaped name).
    let (owner, method) = if let Some(stripped) = body.strip_suffix('`') {
        let open = stripped.rfind('`')?;
        (&body[..open], stripped[open + 1..].to_string())
    } else {
        let start = body
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(|i| i + 1)
            .unwrap_or(0);
        (&body[..start], body[start..].to_string())
    };
    if method.is_empty() {
        return None;
    }
    if let Some(groups) = owner.strip_suffix(']') {
        // `…impl#[Type][Trait]` (trait impl) or `…impl#[Type]` (inherent).
        let groups = format!("{groups}]");
        let parsed = bracket_groups(&groups)?;
        let (prefix, list) = parsed;
        if !prefix.ends_with("impl#") || list.len() != 2 {
            return None;
        }
        return Some(RustMethod::TraitImpl {
            trait_name: bare_type_name(&list[1]),
            method,
        });
    }
    let owner = owner.strip_suffix('#')?;
    let name_start = owner
        .rfind(['/', '#', '.', ']'])
        .map(|i| i + 1)
        .unwrap_or(0);
    let trait_name = owner[name_start..].trim_matches('`');
    if trait_name.is_empty() || trait_name == "impl" {
        return None;
    }
    Some(RustMethod::TraitDecl {
        trait_name: bare_type_name(trait_name),
        method,
    })
}

/// Split `prefix[a][b]` into `prefix` and `["a", "b"]`, honouring backtick
/// escapes inside the brackets (`[`Vec<T, A>`]`).
fn bracket_groups(s: &str) -> Option<(&str, Vec<String>)> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut groups = Vec::new();
    let mut i = chars.len();
    // Walk groups from the end.
    while i > 0 && chars[i - 1].1 == ']' {
        let close = i - 1;
        let mut j = close;
        let mut in_tick = false;
        loop {
            if j == 0 {
                return None;
            }
            j -= 1;
            match chars[j].1 {
                '`' => in_tick = !in_tick,
                '[' if !in_tick => break,
                _ => {}
            }
        }
        let inner: String = chars[j + 1..close].iter().map(|(_, c)| *c).collect();
        groups.push(inner);
        i = j;
    }
    groups.reverse();
    let prefix_end = chars.get(i).map(|(b, _)| *b).unwrap_or(s.len());
    Some((&s[..prefix_end], groups))
}

/// `Area`, `` `From<u32>` `` → `From`, `` `fmt::Display` `` → `Display`.
fn bare_type_name(raw: &str) -> String {
    let t = raw.trim_matches('`');
    let t = t.split('<').next().unwrap_or(t);
    t.rsplit("::").next().unwrap_or(t).trim().to_string()
}

fn document_entry(doc: Document) -> DocumentOccurrences {
    let mut occurrences = doc.occurrences;
    occurrences.sort_by(|a, b| {
        let ra = decode_occurrence_range(a);
        let rb = decode_occurrence_range(b);
        match (ra, rb) {
            (Some(a), Some(b)) => a
                .start_line
                .cmp(&b.start_line)
                .then(a.start_character.cmp(&b.start_character)),
            _ => a.symbol.cmp(&b.symbol),
        }
    });
    DocumentOccurrences {
        relative_path: doc.relative_path,
        occurrences,
    }
}

fn normalize_rel_path(path: &str) -> String {
    path.trim_start_matches("./").replace('\\', "/")
}

fn normalize_project_root(root: &str) -> PathBuf {
    let trimmed = root.strip_prefix("file://").unwrap_or(root);
    PathBuf::from(trimmed)
}

fn has_definition_role(symbol_roles: i32) -> bool {
    (symbol_roles & SymbolRole::Definition.value()) != 0
}

fn symbols_at_call_site(doc: &DocumentOccurrences, line: u32, character: u32) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(exact) = symbol_at_position(doc, line, character) {
        out.push(exact);
    }
    for sym in nearest_function_like_on_line_all(doc, line, character) {
        if !out.contains(&sym) {
            out.push(sym);
        }
    }
    out
}

fn symbol_at_position(doc: &DocumentOccurrences, line: u32, character: u32) -> Option<String> {
    let hits: Vec<&Occurrence> = doc
        .occurrences
        .iter()
        .filter(|occ| {
            decode_occurrence_range(occ).is_some_and(|range| range.contains(line, character))
        })
        .collect();
    if hits.is_empty() {
        return None;
    }
    // Prefer reference occurrences over module-spanning Definition ranges that
    // swallow whole files (common in rust-analyzer SCIP output).
    if let Some(occ) = hits
        .iter()
        .find(|occ| !has_definition_role(occ.symbol_roles))
    {
        return Some(occ.symbol.clone());
    }
    hits.iter()
        .min_by_key(|occ| {
            decode_occurrence_range(occ)
                .map(|r| {
                    (
                        r.end_line.saturating_sub(r.start_line),
                        r.end_character.saturating_sub(r.start_character),
                    )
                })
                .unwrap_or((u32::MAX, u32::MAX))
        })
        .map(|occ| occ.symbol.clone())
}

fn descriptor_after_last_backtick(symbol: &str) -> &str {
    match symbol.rfind('`') {
        Some(i) => &symbol[i + 1..],
        None => symbol,
    }
}

fn is_function_like_symbol(symbol: &str) -> bool {
    let d = descriptor_after_last_backtick(symbol);
    d.contains('(') || (d.contains('#') && !d.ends_with('/'))
}

fn is_package_like_symbol(symbol: &str) -> bool {
    let d = descriptor_after_last_backtick(symbol).trim_start_matches('/');
    d.is_empty() || (d.ends_with('/') && !d.contains('('))
}

fn nearest_function_like_on_line_all(
    doc: &DocumentOccurrences,
    line: u32,
    character: u32,
) -> Vec<String> {
    let mut hits: Vec<(u32, String)> = doc
        .occurrences
        .iter()
        .filter_map(|occ| {
            if !is_function_like_symbol(&occ.symbol) {
                return None;
            }
            if is_package_like_symbol(&occ.symbol) {
                return None;
            }
            let range = decode_occurrence_range(occ)?;
            if range.start_line != line {
                return None;
            }
            if range.end_character <= character && range.start_character < character {
                return None;
            }
            Some((range.start_character, occ.symbol.clone()))
        })
        .collect();
    hits.sort_by_key(|(col, _)| *col);
    hits.into_iter().map(|(_, sym)| sym).collect()
}

pub fn decode_occurrence_range(occ: &Occurrence) -> Option<DecodedRange> {
    if occ.has_single_line_range() {
        let r = occ.single_line_range();
        return Some(DecodedRange {
            start_line: r.line as u32,
            start_character: r.start_character as u32,
            end_line: r.line as u32,
            end_character: r.end_character as u32,
        });
    }
    if occ.has_multi_line_range() {
        let r = occ.multi_line_range();
        return Some(DecodedRange {
            start_line: r.start_line as u32,
            start_character: r.start_character as u32,
            end_line: r.end_line as u32,
            end_character: r.end_character as u32,
        });
    }
    decode_legacy_range(&occ.range)
}

/// Legacy packed `repeated int32` range encoding.
pub fn decode_legacy_range(range: &[i32]) -> Option<DecodedRange> {
    match range.len() {
        3 => {
            let start_line = *range.first()? as u32;
            let start_character = range.get(1).copied()? as u32;
            let end_character = range.get(2).copied()? as u32;
            Some(DecodedRange {
                start_line,
                start_character,
                end_line: start_line,
                end_character,
            })
        }
        4 => Some(DecodedRange {
            start_line: *range.first()? as u32,
            start_character: range.get(1).copied()? as u32,
            end_line: range.get(2).copied()? as u32,
            end_character: range.get(3).copied()? as u32,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scip::types::{Metadata, SymbolRole};

    fn minimal_index_bytes() -> Vec<u8> {
        let mut index = Index::new();
        index.metadata = protobuf::MessageField::some(Metadata {
            project_root: "/fixture".into(),
            ..Default::default()
        });
        let mut doc = Document::new();
        doc.language = "typescript".into();
        doc.relative_path = "src/index.ts".into();
        doc.occurrences.push(Occurrence {
            range: vec![1, 16, 22],
            symbol: "scip-typescript npm . `index`.callee().".into(),
            symbol_roles: SymbolRole::Definition.value(),
            ..Default::default()
        });
        doc.occurrences.push(Occurrence {
            range: vec![4, 9, 15],
            symbol: "scip-typescript npm . `index`.callee().".into(),
            symbol_roles: 0,
            ..Default::default()
        });
        index.documents.push(doc);
        index.write_to_bytes().unwrap()
    }

    fn index_with_documents(paths: &[&str]) -> ScipIndex {
        let mut index = Index::new();
        index.metadata = protobuf::MessageField::some(Metadata {
            project_root: "file:///caddy".into(),
            ..Default::default()
        });
        for p in paths {
            let mut doc = Document::new();
            doc.language = "go".into();
            doc.relative_path = (*p).into();
            index.documents.push(doc);
        }
        ScipIndex::from_bytes(&index.write_to_bytes().unwrap()).unwrap()
    }

    #[test]
    fn coverage_accepts_index_for_this_workspace() {
        let ws = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(ws.path().join("pkg")).unwrap();
        std::fs::write(ws.path().join("main.go"), "package main").unwrap();
        std::fs::write(ws.path().join("pkg/a.go"), "package pkg").unwrap();
        let index = index_with_documents(&["main.go", "pkg/a.go", "pkg/deleted.go"]);
        let cov = index.workspace_coverage(ws.path());
        assert_eq!((cov.total, cov.checked, cov.present), (3, 3, 2));
        assert!(!cov.looks_foreign());
    }

    #[test]
    fn coverage_rejects_index_from_another_checkout() {
        let ws = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(ws.path().join("pkg/kubelet")).unwrap();
        std::fs::write(ws.path().join("pkg/kubelet/kubelet.go"), "package kubelet").unwrap();
        let index = index_with_documents(&[
            "caddy.go",
            "modules/caddyhttp/server.go",
            "modules/caddyhttp/routes.go",
            "pkg/kubelet/kubelet.go",
        ]);
        let cov = index.workspace_coverage(ws.path());
        assert_eq!(cov.present, 1);
        assert!(cov.looks_foreign());
    }

    #[test]
    fn coverage_of_empty_index_is_foreign() {
        let ws = tempfile::TempDir::new().unwrap();
        let cov = index_with_documents(&[]).workspace_coverage(ws.path());
        assert_eq!(cov.ratio(), None);
        assert!(cov.looks_foreign());
    }

    #[test]
    fn coverage_samples_large_indexes() {
        let ws = tempfile::TempDir::new().unwrap();
        let names: Vec<String> = (0..5000).map(|i| format!("f{i:05}.go")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let cov = index_with_documents(&refs).workspace_coverage(ws.path());
        assert_eq!(cov.total, 5000);
        assert!(cov.checked <= COVERAGE_SAMPLE_LIMIT);
        assert!(cov.looks_foreign());
    }

    #[test]
    fn decode_legacy_three_element_range() {
        let range = decode_legacy_range(&[4, 9, 15]).unwrap();
        assert!(range.contains(4, 9));
        assert!(range.contains(4, 14));
        assert!(!range.contains(4, 15));
    }

    #[test]
    fn decode_legacy_four_element_range() {
        let range = decode_legacy_range(&[1, 0, 3, 5]).unwrap();
        assert!(range.contains(2, 2));
        assert!(!range.contains(3, 5));
    }

    #[test]
    fn rust_method_shapes() {
        let ra = |d: &str| format!("rust-analyzer cargo demo 0.1.0 {d}");
        let decl = |t: &str, m: &str| {
            Some(RustMethod::TraitDecl {
                trait_name: t.into(),
                method: m.into(),
            })
        };
        let imp = |t: &str, m: &str| {
            Some(RustMethod::TraitImpl {
                trait_name: t.into(),
                method: m.into(),
            })
        };
        assert_eq!(
            rust_method_shape(&ra("shapes/Area#area().")),
            decl("Area", "area")
        );
        assert_eq!(
            rust_method_shape(&ra("shapes/impl#[Circle][Area]area().")),
            imp("Area", "area")
        );
        assert_eq!(
            rust_method_shape(&ra("impl#[`Vec<T, A>`][`From<u32>`]from().")),
            imp("From", "from")
        );
        assert_eq!(rust_method_shape(&ra("shapes/impl#[Circle]new().")), None);
        assert_eq!(rust_method_shape(&ra("shapes/helper().")), None);
        assert_eq!(rust_method_shape(&ra("Point#x.")), None);
        assert_eq!(
            rust_method_shape("scip-go gomod x v1 `x`/Mux#Use()."),
            None,
            "only rust-analyzer symbols"
        );
    }

    #[test]
    fn repeated_rust_definition_pairs_with_the_nearest_one() {
        let def = |p: &str| ScipDefinition {
            relative_path: p.into(),
            line: 0,
            character: 0,
            symbol: "rust-analyzer cargo pkg 0.1.0 helper().".into(),
        };
        let defs = vec![def("tests/a.rs"), def("tests/b.rs"), def("src/bin/x.rs")];
        let pick = |site: &str| {
            nearest_definition(&defs, site)
                .unwrap()
                .relative_path
                .clone()
        };
        assert_eq!(pick("tests/b.rs"), "tests/b.rs");
        assert_eq!(pick("tests/a.rs"), "tests/a.rs");
        assert_eq!(pick("src/bin/x.rs"), "src/bin/x.rs");
    }

    #[test]
    fn rust_trait_methods_link_to_impls() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../graphengine-parsing/tests/fixtures/rust_scip_calls/index.scip"
        ))
        .unwrap();
        let index = ScipIndex::from_bytes(&bytes).unwrap();
        let sym = |d: &str| format!("rust-analyzer cargo rust-scip-calls 0.0.0 {d}");
        assert_eq!(
            index.implementations_of(&sym("shapes/Area#area().")),
            vec![
                sym("shapes/impl#[Circle][Area]area()."),
                sym("shapes/impl#[Square][Area]area()."),
            ]
        );
        assert!(index
            .implementations_of(&sym("shapes/impl#[Circle]is_empty()."))
            .is_empty());
    }

    #[test]
    fn parses_fixture_and_resolves_definition() {
        let bytes = minimal_index_bytes();
        let index = ScipIndex::from_bytes(&bytes).unwrap();
        let file = Path::new("/fixture/src/index.ts");
        let def = index.definition_at(file, 4, 10).expect("definition");
        assert_eq!(def.symbol, "scip-typescript npm . `index`.callee().");
        assert_eq!(def.line, 1);
        assert_eq!(def.character, 16);
    }

    /// Uses of an external trait's method fan out to the workspace's impls,
    /// except for standard-library traits.
    #[test]
    fn external_trait_method_uses_anchor_at_workspace_impls() {
        let occ = |line: i32, symbol: &str, def: bool| Occurrence {
            range: vec![line, 4, 10],
            symbol: symbol.into(),
            symbol_roles: if def {
                SymbolRole::Definition.value()
            } else {
                0
            },
            ..Default::default()
        };
        let ser = "rust-analyzer cargo serde 1.0.0 ser/Serializer#serialize_str().";
        let clone = "rust-analyzer cargo core https://github.com/rust-lang/rust/library/core clone/Clone#clone().";
        let ws = |d: &str| format!("rust-analyzer cargo app 0.1.0 {d}");
        let mut doc = Document::new();
        doc.relative_path = "src/lib.rs".into();
        doc.occurrences
            .push(occ(3, &ws("impl#[Json][Serializer]serialize_str()."), true));
        doc.occurrences
            .push(occ(9, &ws("impl#[Json][Clone]clone()."), true));
        doc.occurrences.push(occ(20, ser, false));
        doc.occurrences.push(occ(21, clone, false));
        let mut index = Index::new();
        index.metadata = protobuf::MessageField::some(Metadata {
            project_root: "/app".into(),
            ..Default::default()
        });
        index.documents.push(doc);
        let parsed = ScipIndex::from_index(index).unwrap();
        let pairs = parsed.reference_occurrences();
        let ser_use = pairs
            .iter()
            .find(|(r, _)| r.line == 20)
            .expect("serde trait use kept");
        assert_eq!(ser_use.1.symbol, ser);
        assert_eq!(ser_use.1.line, 3, "anchored at the workspace impl");
        assert_eq!(
            parsed.implementations_of(ser),
            vec![ws("impl#[Json][Serializer]serialize_str().")]
        );
        assert!(
            !pairs.iter().any(|(r, _)| r.line == 21),
            "std trait uses do not fan out"
        );
    }

    #[test]
    fn package_caret_on_selector_resolves_to_function() {
        let mut index = Index::new();
        index.metadata = protobuf::MessageField::some(Metadata {
            project_root: "/chi".into(),
            ..Default::default()
        });
        let mut doc = Document::new();
        doc.language = "go".into();
        doc.relative_path = "middleware/strip_test.go".into();
        doc.occurrences.push(Occurrence {
            range: vec![13, 6, 9],
            symbol: "scip-go gomod github.com/go-chi/chi/v5 v5.2.1 `github.com/go-chi/chi/v5`/"
                .into(),
            symbol_roles: 0,
            ..Default::default()
        });
        doc.occurrences.push(Occurrence {
            range: vec![13, 10, 19],
            symbol:
                "scip-go gomod github.com/go-chi/chi/v5 v5.2.1 `github.com/go-chi/chi/v5`/NewRouter()."
                    .into(),
            symbol_roles: 0,
            ..Default::default()
        });
        let mut def = Occurrence {
            range: vec![60, 5, 14],
            symbol:
                "scip-go gomod github.com/go-chi/chi/v5 v5.2.1 `github.com/go-chi/chi/v5`/NewRouter()."
                    .into(),
            ..Default::default()
        };
        def.symbol_roles = SymbolRole::Definition.value();
        let mut def_doc = Document::new();
        def_doc.relative_path = "chi.go".into();
        def_doc.occurrences.push(def);
        index.documents.push(doc);
        index.documents.push(def_doc);
        let parsed = ScipIndex::from_index(index).unwrap();
        let file = Path::new("/chi/middleware/strip_test.go");
        let hit = parsed
            .definition_at(file, 13, 6)
            .expect("package caret should upgrade to NewRouter");
        assert!(hit.symbol.ends_with("NewRouter()."));
    }

    #[test]
    fn type_conversion_caret_resolves_handlerfunc() {
        let mut index = Index::new();
        index.metadata = protobuf::MessageField::some(Metadata {
            project_root: "/chi".into(),
            ..Default::default()
        });
        let mut doc = Document::new();
        doc.relative_path = "middleware/get_head.go".into();
        doc.occurrences.push(Occurrence {
            range: vec![10, 8, 12],
            symbol: "scip-go gomod github.com/golang/go/src go1.20 `net/http`/".into(),
            symbol_roles: 0,
            ..Default::default()
        });
        doc.occurrences.push(Occurrence {
            range: vec![10, 13, 24],
            symbol: "scip-go gomod github.com/golang/go/src go1.20 `net/http`/HandlerFunc#".into(),
            symbol_roles: 0,
            ..Default::default()
        });
        let mut def = Occurrence {
            range: vec![2000, 0, 12],
            symbol: "scip-go gomod github.com/golang/go/src go1.20 `net/http`/HandlerFunc#".into(),
            ..Default::default()
        };
        def.symbol_roles = SymbolRole::Definition.value();
        let mut def_doc = Document::new();
        def_doc.relative_path = "src/net/http/server.go".into();
        def_doc.occurrences.push(def);
        index.documents.push(doc);
        index.documents.push(def_doc);
        let parsed = ScipIndex::from_index(index).unwrap();
        let hit = parsed
            .definition_at(Path::new("/chi/middleware/get_head.go"), 10, 8)
            .expect("http.HandlerFunc should resolve the type");
        assert!(hit.symbol.ends_with("HandlerFunc#"));
    }
}
