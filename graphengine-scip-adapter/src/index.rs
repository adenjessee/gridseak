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

/// Parsed SCIP index with occurrence lookup tables.
#[derive(Debug, Clone)]
pub struct ScipIndex {
    project_root: PathBuf,
    by_relative_path: HashMap<String, DocumentOccurrences>,
    definitions_by_symbol: HashMap<String, ScipDefinition>,
    implementations_by_symbol: HashMap<String, Vec<String>>,
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
        let mut definitions_by_symbol = HashMap::new();
        let mut implementations_by_symbol: HashMap<String, Vec<String>> = HashMap::new();
        let mut references_by_symbol: HashMap<String, Vec<ScipDefinition>> = HashMap::new();
        for doc in index.documents {
            for info in &doc.symbols {
                for rel in &info.relationships {
                    if rel.is_implementation {
                        implementations_by_symbol
                            .entry(rel.symbol.clone())
                            .or_default()
                            .push(info.symbol.clone());
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
                    definitions_by_symbol.insert(occ.symbol.clone(), loc);
                } else {
                    references_by_symbol
                        .entry(occ.symbol.clone())
                        .or_default()
                        .push(loc);
                }
            }
            by_relative_path.insert(normalize_rel_path(&entry.relative_path), entry);
        }
        Ok(Self {
            project_root,
            by_relative_path,
            definitions_by_symbol,
            implementations_by_symbol,
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

    /// Symbols that implement `symbol` (SCIP `is_implementation` relationships).
    /// Callers split confidence 1/n across the may-call set.
    pub fn implementations_of(&self, symbol: &str) -> Vec<String> {
        self.implementations_by_symbol
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
    fn parses_fixture_and_resolves_definition() {
        let bytes = minimal_index_bytes();
        let index = ScipIndex::from_bytes(&bytes).unwrap();
        let file = Path::new("/fixture/src/index.ts");
        let def = index.definition_at(file, 4, 10).expect("definition");
        assert_eq!(def.symbol, "scip-typescript npm . `index`.callee().");
        assert_eq!(def.line, 1);
        assert_eq!(def.character, 16);
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
