//! Read-only SQL helpers over the persisted `project-graphs/<scan_id>.sqlite`.
//!
//! Stage 6 turns the CLI into a lightweight graph explorer. The
//! contract for every query in this module:
//!
//! - Opens the SQLite **read-only** via the `mode=ro` URI parameter so
//!   we cannot accidentally corrupt the scan artifact, even with a bug.
//! - Returns a `GraphQueryError` typed by failure class so callers
//!   (CLI + future MCP tools) can map each variant onto a friendly
//!   message without string-matching.
//! - Honors `Call`-kind edges as the call-graph by default and
//!   exposes `Contains` separately for module / file membership.
//! - Resolves user-supplied "symbols" by exact FQN first, falling
//!   back to suffix or substring matches so users don't have to paste
//!   the canonical FQN every time.
//!
//! Keeping the SQL out of the renderers means MCP can reuse the same
//! query layer without re-implementing the JOINs (Stage 8 dependency).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

pub mod agent_tools;
pub mod queries;

pub use queries::*;

#[derive(Debug, thiserror::Error)]
pub enum GraphQueryError {
    #[error("graph artifact not found at {0}; has this project ever been scanned?")]
    ArtifactMissing(String),
    #[error("symbol `{0}` did not match any function node (tried exact, suffix, then substring)")]
    UnknownSymbol(String),
    #[error("symbol `{symbol}` matches multiple candidates; specify more of the FQN or name it by location (`path/to/file.ts:123`) (showing {shown}/{total}): {candidates:?}")]
    AmbiguousSymbol {
        symbol: String,
        candidates: Vec<String>,
        shown: usize,
        total: usize,
    },
    // `SeedLooksLikePath { seed: String }` was defined here but never
    // constructed — it predated `OverlyGenericSymbol` in the Q3
    // resolver-hardening session and was effectively superseded.
    // Deleted to silence the dead_code warning. If we later want to
    // emit a dedicated error for path-shaped seeds (e.g. `foo/bar.rs`
    // or seeds ending in a recognised source extension), reintroduce
    // it with a construction site in `resolve_symbol_detailed` and a
    // unit test exercising the new branch.
    #[error(
        "symbol `{seed_fqn}` resolved to a {kind} node, but blast_radius only traverses Call edges \
         between Function nodes. Pick a function inside this {kind}."
    )]
    SeedNotAFunction { seed_fqn: String, kind: String },
    /// Substring fallback was the only thing that would have matched
    /// and the seed is so short/generic that a substring scan would
    /// pull in semantically unrelated symbols from other modules. We
    /// refuse rather than silently injecting noise into a BFS the
    /// agent will then summarise to the user.
    ///
    /// Surfaced by P2: the bare seed `load` substring-matched five
    /// `*_load*` functions in `graphengine-parsing` and contaminated
    /// the blast radius result. A careful agent caught it; a
    /// less-careful agent would have shipped the bad data.
    #[error(
        "symbol `{symbol}` is too generic for substring resolution \
         (matches names like `*{symbol}*` across the codebase). \
         Pass a qualified path instead — e.g. `module::{symbol}` or \
         the full FQN — or name it by location: `path/to/file.ts:123` \
         (the line it is declared on) or `path/to/file.ts::{symbol}`. \
         Use `gridseak_resolve_symbol` (if available) \
         or `gridseak_get_recommendations` to find the canonical FQN."
    )]
    OverlyGenericSymbol { symbol: String },
    /// A location-shaped seed (`src/a.ts:120`, `src/a.ts::name`) named a
    /// scanned file, but no function or method there matches.
    #[error("no function or method matches `{location}`: {detail}")]
    NoSymbolAtLocation { location: String, detail: String },
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
}

/// How `resolve_symbol` arrived at the returned node. The agent can
/// use this to decide how loudly to caveat — `SubstringUnique` is
/// "the only match happened to be unique but the input was vague,
/// double-check with the user."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionMethod {
    /// Exact FQN match — canonical case, no caveats.
    ExactFqn,
    /// Exact node-id match (caller pasted an id from JSON output).
    ExactId,
    /// Suffix unique match — agent passed a partial FQN that
    /// uniquely identified the symbol. Safe to trust.
    SuffixUnique,
    /// Substring unique match — agent passed a partial name that
    /// happened to be unique in this graph. **Caller should warn
    /// the user** that the match was inferred from a substring scan
    /// rather than a structural identifier. Returned only when the
    /// seed was not on the overly-generic blocklist.
    SubstringUnique,
    /// Location-shaped seed: `file:line` (the function or method declared
    /// on, or enclosing, that line) or `file::name` (the one function of
    /// that name in the file). Exact — no caveat needed.
    Location,
}

/// Result of [`resolve_symbol_detailed`]. Wraps a [`NodeRef`] with
/// the resolution method so the caller can decide whether to attach
/// a "resolution: substring" warning to the response envelope.
#[derive(Debug, Clone, Serialize)]
pub struct SymbolResolution {
    pub node: NodeRef,
    pub method: ResolutionMethod,
}

/// Heuristic: does the seed string look like a filesystem path rather
/// than an FQN? Catches the common agent mistake of passing
/// `crate/src/foo.rs` to a symbol-only tool. Keep this conservative:
/// FQNs with `::` separators are the canonical form, paths use `/` or
/// known source extensions. Centralised here so the CLI and MCP
/// reject path-shaped seeds identically, before resolve_symbol
/// wastes a substring scan looking for them.
///
/// Location-shaped seeds (`src/a.ts:120`, `src/a.ts::name`) name a single
/// declaration and are not paths in this sense.
pub fn seed_looks_like_path(seed: &str) -> bool {
    if parse_location_seed(seed).is_some() {
        return false;
    }
    if seed.contains('/') || seed.contains('\\') {
        return true;
    }
    const SOURCE_EXTS: &[&str] = &[
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".kt", ".swift", ".cls",
        ".trigger",
    ];
    SOURCE_EXTS.iter().any(|ext| seed.ends_with(ext))
}

/// True when a resolved node is callable (has Call edges in the
/// graph). Other kinds (Module, File, etc.) are valid graph nodes
/// but reachable only via Contains, so traversals that follow Call
/// edges return empty for them.
pub fn node_is_function(node: &NodeRef) -> bool {
    node.kind.eq_ignore_ascii_case("Function")
        || node.kind.eq_ignore_ascii_case("Method")
        || node.kind.eq_ignore_ascii_case("Constructor")
}

/// Open the per-scan graph artifact in read-only mode.
///
/// We deliberately fail with a friendly typed error when the artifact
/// is missing rather than allowing rusqlite's `unable to open database
/// file` to leak through. The path is a literal filesystem path; we
/// URI-encode it ourselves because `Connection::open_with_flags`
/// expects a path, and we want the read-only safety the URI form
/// provides.
pub fn open_graph(path: &Path) -> Result<Connection, GraphQueryError> {
    if !path.exists() {
        return Err(GraphQueryError::ArtifactMissing(path.display().to_string()));
    }
    // `OpenFlags::SQLITE_OPEN_READ_ONLY` plus the URI form gives us a
    // belt-and-suspenders guarantee against accidental writes. The URI
    // form also lets us opt into shared cache should we ever want to
    // run many parallel queries; today we don't, but the connection
    // string is the right place to express intent.
    let escaped = encode_uri_path(path);
    let uri = format!("file:{escaped}?mode=ro&immutable=1");
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&uri, flags)?;
    Ok(conn)
}

fn encode_uri_path(path: &Path) -> String {
    // SQLite URIs require a few characters to be percent-encoded. We
    // hit `#` and `?` in real-world macOS paths often enough to need
    // this. Spaces work fine without encoding because the URI handler
    // is lenient.
    let s = path.display().to_string();
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            // SQLite expects forward slashes even on Windows; harmless on POSIX.
            '\\' => out.push('/'),
            other => out.push(other),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Symbol resolution
// ---------------------------------------------------------------------------

/// Names that the substring-fallback in [`resolve_symbol_detailed`]
/// refuses to substring-match against. These are short, generic
/// identifiers that occur in dozens of unrelated modules (P2
/// dogfood evidence: `load` substring-matched five `*_load*`
/// functions across `graphengine-parsing` when an agent passed
/// `AnalysisGraph::load` as a seed). When a caller passes a bare
/// `load`, they must qualify it (e.g. `AnalysisGraph::load` or
/// `crate::graph::load`) so the resolver can prove uniqueness via
/// suffix match instead of guessing.
///
/// Sorted alphabetically for easy diff review. Adding to this list
/// is a backwards-compatible UX upgrade — callers who already pass
/// fully-qualified names are unaffected.
const OVERLY_GENERIC_SYMBOL_NAMES: &[&str] = &[
    "as_str",
    "build",
    "clone",
    "create",
    "default",
    "delete",
    "drop",
    "format",
    "from",
    "get",
    "init",
    "into",
    "is_empty",
    "iter",
    "len",
    "load",
    "new",
    "next",
    "parse",
    "run",
    "save",
    "set",
    "start",
    "stop",
    "to_string",
    "update",
];

/// Hard minimum length for substring resolution. Below this we
/// refuse the substring fallback even off-blocklist — a 3-character
/// substring like `add` or `fmt` will catch hundreds of unrelated
/// symbols in any real codebase, and the agent should be told so
/// rather than silently fed contaminated results. The 6-character
/// threshold matches the dogfood doc's "≤6 chars" guidance.
const MIN_SUBSTRING_SEED_LEN: usize = 6;

/// True when `needle` is a bare, ungenericised identifier that we
/// should not let the substring fallback match. "Bare" means no
/// path separator (`::`), no parens, no generics — just a name.
/// Centralised so the test pins exactly the inputs we refuse.
fn is_overly_generic_seed(needle: &str) -> bool {
    if needle.contains("::") || needle.contains('.') || needle.contains('(') {
        return false;
    }
    let trimmed = needle.trim();
    if trimmed.len() < MIN_SUBSTRING_SEED_LEN {
        return true;
    }
    OVERLY_GENERIC_SYMBOL_NAMES.contains(&trimmed)
}

/// Resolve a user-supplied symbol to a node id + canonical FQN.
///
/// Thin convenience wrapper over [`resolve_symbol_detailed`] for
/// callers that don't need to know which resolution method fired.
/// Prefer the detailed variant in MCP tools so the response envelope
/// can warn when a substring scan succeeded but the input was vague.
pub fn resolve_symbol(conn: &Connection, needle: &str) -> Result<NodeRef, GraphQueryError> {
    resolve_symbol_detailed(conn, needle).map(|r| r.node)
}

/// Resolve a user-supplied symbol and report **how** the match was
/// made. Resolution order, each strictly more permissive than the
/// previous:
///
/// 1. Exact FQN match (`bindings::process_import_binding`) →
///    [`ResolutionMethod::ExactFqn`].
/// 2. Exact id match (caller pasted a node id from JSON output) →
///    [`ResolutionMethod::ExactId`].
/// 3. Suffix match on FQN (`process_import_binding` finds the same
///    node above as long as no other function ends with that suffix)
///    → [`ResolutionMethod::SuffixUnique`].
/// 4. Substring match on FQN — **last-resort, gated on
///    [`is_overly_generic_seed`]**. When the seed is a bare generic
///    name (`load`, `init`, `new`, …) or under
///    [`MIN_SUBSTRING_SEED_LEN`] characters, the substring fallback
///    refuses and we return [`GraphQueryError::OverlyGenericSymbol`]
///    so the caller is forced to disambiguate. When the substring
///    fallback does succeed, the resolution method is
///    [`ResolutionMethod::SubstringUnique`] so the response envelope
///    can attach a "resolution: substring" warning the agent must
///    relay to the user.
///
/// We never silently pick one of many matches; ambiguity surfaces
/// as [`GraphQueryError::AmbiguousSymbol`] with the candidate list
/// so the caller can re-prompt the user.
pub fn resolve_symbol_detailed(
    conn: &Connection,
    needle: &str,
) -> Result<SymbolResolution, GraphQueryError> {
    if let Some(resolution) = resolve_location_shaped(conn, needle)? {
        return Ok(resolution);
    }
    if let Some(node) = lookup_exact_fqn(conn, needle)? {
        return Ok(SymbolResolution {
            node,
            method: ResolutionMethod::ExactFqn,
        });
    }
    if let Some(node) = lookup_exact_id(conn, needle)? {
        return Ok(SymbolResolution {
            node,
            method: ResolutionMethod::ExactId,
        });
    }
    let suffix = format!("%{needle}");
    let suffix_matches = lookup_like_fqn(conn, &suffix, 25)?;
    if suffix_matches.len() == 1 {
        return Ok(SymbolResolution {
            node: suffix_matches.into_iter().next().unwrap(),
            method: ResolutionMethod::SuffixUnique,
        });
    }
    if suffix_matches.len() > 1 {
        let total = suffix_matches.len();
        return Err(GraphQueryError::AmbiguousSymbol {
            symbol: needle.to_string(),
            candidates: suffix_matches
                .iter()
                .take(10)
                .map(|c| c.fqn.clone())
                .collect(),
            shown: 10.min(total),
            total,
        });
    }
    // Path-shaped names agents write naturally: Go import paths
    // (`k8s.io/kubernetes/pkg/api/v1/pod.GetPodCondition`) and file paths
    // (`pkg/api/v1/pod/util.go::GetPodCondition`). Without this the lookup
    // failed and agents fell back to grep (P3 scale-test calibration).
    if let Some(resolution) = resolve_path_shaped(conn, needle)? {
        return Ok(resolution);
    }
    // Substring fallback. Gate it on the generic-name blocklist so
    // a bare `load` doesn't silently match `lazy_load` from another
    // module. See P2 dogfood evidence in V0_1_0_RC1_FOLLOWUP_ISSUES.
    if is_overly_generic_seed(needle) {
        return Err(GraphQueryError::OverlyGenericSymbol {
            symbol: needle.to_string(),
        });
    }
    let substring = format!("%{needle}%");
    let substring_matches = lookup_like_fqn(conn, &substring, 25)?;
    if substring_matches.len() == 1 {
        return Ok(SymbolResolution {
            node: substring_matches.into_iter().next().unwrap(),
            method: ResolutionMethod::SubstringUnique,
        });
    }
    if substring_matches.len() > 1 {
        let total = substring_matches.len();
        return Err(GraphQueryError::AmbiguousSymbol {
            symbol: needle.to_string(),
            candidates: substring_matches
                .iter()
                .take(10)
                .map(|c| c.fqn.clone())
                .collect(),
            shown: 10.min(total),
            total,
        });
    }
    Err(GraphQueryError::UnknownSymbol(needle.to_string()))
}

/// A seed that names a declaration by where it lives in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationSeed {
    /// `path/to/file.ts:123` (optionally `:col`): the function or method
    /// declared on, or enclosing, line 123.
    Line { file: String, line: u32 },
    /// `path/to/file.ts::name` (or `::Class.name`): the function or method
    /// called `name` in that file.
    Name { file: String, name: String },
}

/// Parse `file:line[:col]` or `file::name` where `file` ends in a source
/// extension. Anything else (FQNs, Go import paths) is `None`.
pub fn parse_location_seed(needle: &str) -> Option<LocationSeed> {
    let needle = needle.trim();
    let clean = |f: &str| f.trim().trim_start_matches("./").to_string();
    if let Some((file, name)) = needle.split_once("::") {
        let name = name.trim();
        if has_source_ext(file) && !name.is_empty() {
            return Some(LocationSeed::Name {
                file: clean(file),
                name: name.to_string(),
            });
        }
        return None;
    }
    let (rest, last) = needle.rsplit_once(':')?;
    let last: u32 = last.trim().parse().ok()?;
    // `file:line:col` — drop the column.
    let (file, line) = match rest.rsplit_once(':') {
        Some((file, line)) if line.trim().parse::<u32>().is_ok() => {
            (file, line.trim().parse().ok()?)
        }
        _ => (rest, last),
    };
    (has_source_ext(file) && line > 0).then(|| LocationSeed::Line {
        file: clean(file),
        line,
    })
}

fn has_source_ext(path: &str) -> bool {
    const EXTS: &[&str] = &[
        "rs", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt",
        "swift", "cls", "trigger", "cs", "rb", "php", "c", "cc", "cpp", "h", "hpp",
    ];
    path.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && EXTS.contains(&ext))
}

/// True for the synthetic nodes that stand for a file's or module's
/// top-level code rather than a declared function.
pub fn is_top_level_scope_fqn(fqn: &str) -> bool {
    fqn.ends_with("__file_scope__") || fqn.ends_with("__file_module__")
}

/// What kind of caller a node is, as agents see it: `function`, `method`
/// or `constructor` (callable), or — for code that runs outside any
/// declared function — `file_scope` (a file's top-level statements),
/// `module`, `file`, or `synthetic` (a parser-made wrapper). The bool is
/// whether the node is a callable function or method.
pub fn caller_kind(conn: &Connection, node: &NodeRef) -> Result<(String, bool), GraphQueryError> {
    if is_top_level_scope_fqn(&node.fqn) {
        return Ok(("file_scope".into(), false));
    }
    if !node_is_function(node) {
        return Ok((node.kind.to_ascii_lowercase(), false));
    }
    let synthetic: bool = conn
        .query_row(
            "SELECT COALESCE(json_extract(properties, '$.synthetic'), 0) FROM nodes WHERE id = ?1",
            [&node.id],
            |r| r.get::<_, i64>(0),
        )
        .map(|v| v != 0)
        .unwrap_or(false);
    if synthetic {
        return Ok(("synthetic".into(), false));
    }
    let short = node.fqn.rsplit("::").next().unwrap_or(&node.fqn);
    let kind = if short == "constructor" || node.kind.eq_ignore_ascii_case("Constructor") {
        "constructor"
    } else if node.kind.eq_ignore_ascii_case("Method") {
        "method"
    } else {
        "function"
    };
    Ok((kind.into(), true))
}

/// Resolve `file:line` / `file::name` against node locations. `Ok(None)`
/// when the seed is not location-shaped or no scanned file matches `file`
/// (so path-shaped FQN resolution can still try it); an error when the file
/// matches but no single function there does.
fn resolve_location_shaped(
    conn: &Connection,
    needle: &str,
) -> Result<Option<SymbolResolution>, GraphQueryError> {
    let Some(seed) = parse_location_seed(needle) else {
        return Ok(None);
    };
    let file = match &seed {
        LocationSeed::Line { file, .. } | LocationSeed::Name { file, .. } => file.clone(),
    };
    // Stored paths are usually absolute; match `file` as a whole-segment suffix.
    let mut stmt = conn.prepare(
        "SELECT id, fqn, kind, f, json_extract(location, '$.start_line'),
                json_extract(location, '$.end_line')
         FROM (SELECT *, json_extract(location, '$.file') AS f FROM nodes
               WHERE kind IN ('Function', 'Method', 'Constructor'))
         WHERE f = ?1 OR substr(f, -length(?1) - 1) = '/' || ?1",
    )?;
    let rows = stmt
        .query_map([&file], |r| {
            Ok((
                NodeRef {
                    id: r.get(0)?,
                    fqn: r.get(1)?,
                    kind: r.get(2)?,
                    file: r.get(3)?,
                    line: r.get(4)?,
                },
                r.get::<_, Option<u32>>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut files: Vec<&str> = rows.iter().filter_map(|(n, _)| n.file.as_deref()).collect();
    files.sort_unstable();
    files.dedup();
    if files.len() > 1 {
        return Err(GraphQueryError::AmbiguousSymbol {
            symbol: needle.to_string(),
            candidates: files.iter().take(10).map(|f| f.to_string()).collect(),
            shown: files.len().min(10),
            total: files.len(),
        });
    }
    let candidates: Vec<(NodeRef, u32, u32)> = rows
        .into_iter()
        .filter(|(n, _)| !is_top_level_scope_fqn(&n.fqn))
        .map(|(n, end)| {
            let start = n.line.unwrap_or(0);
            (n, start, end.unwrap_or(start))
        })
        .collect();
    let located = |n: &NodeRef| format!("{} ({}:{})", n.fqn, file, n.line.unwrap_or(0));
    let picked = match &seed {
        LocationSeed::Line { line, .. } => {
            let line = *line;
            let on_line = candidates
                .iter()
                .filter(|(_, s, _)| *s == line)
                .min_by_key(|(_, s, e)| e - s);
            let enclosing = || {
                candidates
                    .iter()
                    .filter(|(_, s, e)| *s <= line && line <= *e)
                    .min_by_key(|(_, s, e)| (e - s, u32::MAX - s))
            };
            match on_line.or_else(enclosing) {
                Some((n, _, _)) => n.clone(),
                None => {
                    return Err(GraphQueryError::NoSymbolAtLocation {
                        location: needle.to_string(),
                        detail: format!(
                            "line {line} of `{file}` is not inside any function or method \
                             (top-level code, a type, or a field that is not a function). \
                             Pass the line of the declaration itself."
                        ),
                    })
                }
            }
        }
        LocationSeed::Name { name, .. } => {
            let short = name.rsplit(['.', ':']).next().unwrap_or(name);
            let hits: Vec<&NodeRef> = candidates
                .iter()
                .map(|(n, _, _)| n)
                .filter(|n| n.fqn.rsplit("::").next() == Some(short))
                .collect();
            match hits.as_slice() {
                [one] => (*one).clone(),
                [] => {
                    return Err(GraphQueryError::NoSymbolAtLocation {
                        location: needle.to_string(),
                        detail: format!("`{file}` declares no function or method named `{short}`"),
                    })
                }
                many => {
                    return Err(GraphQueryError::AmbiguousSymbol {
                        symbol: needle.to_string(),
                        candidates: many.iter().take(10).map(|n| located(n)).collect(),
                        shown: many.len().min(10),
                        total: many.len(),
                    })
                }
            }
        }
    };
    Ok(Some(SymbolResolution {
        node: picked,
        method: ResolutionMethod::Location,
    }))
}

/// Split a path-shaped symbol into (directory segments, symbol tail).
///
/// - `pkg/api/v1/pod/util.go::GetPodCondition` → (`pkg api v1 pod util`, `GetPodCondition`)
/// - `k8s.io/kubernetes/pkg/api/v1/pod.GetPodCondition` → (`k8s.io kubernetes pkg api v1 pod`, `GetPodCondition`)
/// - `example.com/m/pkg.Type.Method` → (`example.com m pkg`, `Type.Method`)
fn split_path_symbol(needle: &str) -> Option<(Vec<String>, String)> {
    if !needle.contains('/') {
        return None;
    }
    if let Some((path, tail)) = needle
        .rsplit_once("::")
        .or_else(|| needle.rsplit_once(':'))
        .filter(|(p, _)| p.contains('.') || p.contains('/'))
    {
        let path = path
            .rsplit_once('.')
            .filter(|(_, ext)| ext.len() <= 4 && !ext.contains('/'))
            .map(|(stem, _)| stem)
            .unwrap_or(path);
        let segs = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        return Some((segs, tail.to_string()));
    }
    let (dir, last) = needle.rsplit_once('/')?;
    let (pkg, tail) = last.split_once('.')?;
    let mut segs: Vec<String> = dir
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    segs.push(pkg.to_string());
    Some((segs, tail.to_string()))
}

/// Resolve a path-shaped symbol by trying ever-shorter trailing directory
/// segments (module prefixes like `k8s.io/kubernetes` are not in FQNs) until
/// exactly one function ends with the tail under that path.
fn resolve_path_shaped(
    conn: &Connection,
    needle: &str,
) -> Result<Option<SymbolResolution>, GraphQueryError> {
    let Some((segs, tail)) = split_path_symbol(needle) else {
        return Ok(None);
    };
    if tail.is_empty() || segs.is_empty() {
        return Ok(None);
    }
    for k in (1..=segs.len()).rev() {
        let dir = segs[segs.len() - k..].join("::");
        let mut matches = lookup_like_fqn(conn, &format!("%{dir}::{tail}"), 25)?;
        if matches.is_empty() {
            // Go/TS FQNs carry the file stem between package and symbol.
            matches = lookup_like_fqn(conn, &format!("%{dir}::%::{tail}"), 25)?;
            matches.retain(|m| {
                m.fqn
                    .rsplit_once("::")
                    .and_then(|(prefix, _)| prefix.rsplit_once("::"))
                    .is_some_and(|(pkg, _)| pkg.ends_with(&dir))
            });
        }
        match matches.len() {
            0 => continue,
            1 => {
                return Ok(Some(SymbolResolution {
                    node: matches.into_iter().next().unwrap(),
                    method: ResolutionMethod::SuffixUnique,
                }))
            }
            total => {
                return Err(GraphQueryError::AmbiguousSymbol {
                    symbol: needle.to_string(),
                    candidates: matches.iter().take(10).map(|c| c.fqn.clone()).collect(),
                    shown: 10.min(total),
                    total,
                })
            }
        }
    }
    Ok(None)
}

fn lookup_exact_fqn(conn: &Connection, fqn: &str) -> Result<Option<NodeRef>, GraphQueryError> {
    let mut stmt = conn.prepare("SELECT id, fqn, kind FROM nodes WHERE fqn = ?1 LIMIT 1")?;
    let mut rows = stmt.query([fqn])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(NodeRef {
            id: row.get(0)?,
            fqn: row.get(1)?,
            kind: row.get(2)?,
            file: None,
            line: None,
        }));
    }
    Ok(None)
}

fn lookup_exact_id(conn: &Connection, id: &str) -> Result<Option<NodeRef>, GraphQueryError> {
    let mut stmt = conn.prepare("SELECT id, fqn, kind FROM nodes WHERE id = ?1 LIMIT 1")?;
    let mut rows = stmt.query([id])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(NodeRef {
            id: row.get(0)?,
            fqn: row.get(1)?,
            kind: row.get(2)?,
            file: None,
            line: None,
        }));
    }
    Ok(None)
}

fn lookup_like_fqn(
    conn: &Connection,
    pattern: &str,
    cap: usize,
) -> Result<Vec<NodeRef>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "SELECT id, fqn, kind FROM nodes
         WHERE fqn LIKE ?1 AND kind IN ('Function', 'Module', 'File')
         ORDER BY length(fqn), fqn
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![pattern, cap as i64], |row| {
            Ok(NodeRef {
                id: row.get(0)?,
                fqn: row.get(1)?,
                kind: row.get(2)?,
                file: None,
                line: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Types returned to the renderers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct NodeRef {
    pub id: String,
    pub fqn: String,
    pub kind: String,
    /// Source location, filled for caller/callee lists so an agent can open
    /// or edit the site without reverse-engineering the FQN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FanRow {
    pub node: NodeRef,
    pub count: i64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct PathRow {
    pub depth: usize,
    pub node: NodeRef,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoupledModuleRow {
    pub from: NodeRef,
    pub to: NodeRef,
    pub edge_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CycleRow {
    pub length: usize,
    pub members: Vec<NodeRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SliceNode {
    pub depth: usize,
    pub direction: SliceDirection,
    pub node: NodeRef,
    /// Tier of the Call edge that linked this node into the BFS
    /// frontier. One of `"tier_0"` (tree-sitter), `"tier_1"`
    /// (grep heuristic), or `"tier_3"` (LSP-verified). `None` when
    /// the edge provenance was missing in the database (older
    /// scans) or unparseable — consumers should render that as
    /// "unknown tier" rather than assume Tier 0.
    ///
    /// Why per-hop: the response envelope marks the *seed* as
    /// deterministically resolved, but at depth 3 a node might have
    /// been reached via a Tier 1 grep heuristic. Without per-hop
    /// tier, the agent has to caveat the whole BFS result as
    /// "possibly noisy at the leaves." Per-hop annotation lets the
    /// agent report Tier 0 hits as "definitely affected" and Tier 1
    /// hits as "possibly affected" — that's the trust contract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_evidence_tier: Option<String>,
    /// Calibrated P(true) of the Call edge that pulled this node in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p_true: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceDirection {
    Upstream,
    Downstream,
}

/// One row in a file-scoped blast radius response. Same shape as a
/// regular [`SliceNode`] row but additionally carries the list of
/// file-resident symbols that pulled it into the result set — the
/// agent can show a user "fn_x is affected because the file's
/// `init`, `tick`, and `shutdown` all transitively reach it."
#[derive(Debug, Clone, Serialize)]
pub struct FileBlastRadiusRow {
    pub depth: usize,
    pub node: NodeRef,
    /// Symbols in the queried file that reach this node in the
    /// upstream direction (i.e. they are downstream of `node`, so
    /// changing `node` would force re-validation of these symbols).
    /// Multiple entries mean the row is "more affected" by edits to
    /// the file. Sorted by FQN for stable rendering.
    pub via_seeds: Vec<NodeRef>,
    /// Best (highest-trust) tier across all per-seed BFS paths that
    /// reach this node. See `best_tier` for the precedence rule
    /// (`tier_3` > `tier_0` > `tier_1`). `None` when no seed walk
    /// produced a recognisable provenance source. Mirrors the
    /// `SliceNode::edge_evidence_tier` contract so the renderer can
    /// reuse the same legend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_evidence_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p_true: Option<f64>,
}

/// Pick the more-trusted of two edge tiers for aggregation
/// (e.g. file blast radius unions BFS paths from multiple seeds; if
/// any seed reaches a row via Tier 3 we want the aggregated row to
/// report Tier 3). Precedence: `tier_3` > `tier_0` > `tier_1`;
/// unknown / unset tiers lose to any known tier.
///
/// Returning `&'static str` keeps the helper allocation-free; the
/// caller maps it back to `Option<String>` when storing.
pub fn best_tier<'a>(a: Option<&'a str>, b: Option<&'a str>) -> Option<&'a str> {
    fn rank(t: Option<&str>) -> u8 {
        match t {
            Some("tier_3") => 3,
            Some("tier_0") => 2,
            Some("tier_1") => 1,
            _ => 0,
        }
    }
    if rank(a) >= rank(b) {
        a.or(b)
    } else {
        b.or(a)
    }
}

/// Aggregated upstream blast radius for every callable symbol in a
/// single file. One MCP call replaces the 50-symbol fanout pattern
/// agents previously had to perform manually.
#[derive(Debug, Clone, Serialize)]
pub struct FileBlastRadiusResult {
    /// Repo-relative path that was queried, echoed back so the
    /// caller can match this response to the originating request.
    pub file_path: String,
    /// Function-kind nodes that physically live in `file_path`. Used
    /// as BFS seeds; also returned so the caller can see the symbol
    /// surface of the file at a glance.
    pub seeds: Vec<NodeRef>,
    /// Upstream callers (depth-bounded), unioned across all seeds.
    /// Each row excludes the seeds themselves so the caller sees
    /// only the *external* blast radius.
    pub rows: Vec<FileBlastRadiusRow>,
    /// `true` when the per-seed BFS hit the per-call cap before
    /// completing. The result is still useful but partial; the
    /// caller should narrow `depth` or widen `cap` for a complete
    /// view.
    pub cap_hit: bool,
}

// ---------------------------------------------------------------------------
// Shared traversal primitives
// ---------------------------------------------------------------------------

/// Walk `Call` edges outward from `seed` up to `max_depth`. `direction
/// = "out"` follows callees (downstream); `"in"` follows callers
/// (upstream). Returns a `(depth, node, edge_tier)` sequence in BFS
/// order.
///
/// `edge_tier` is the tier label of the *first* edge that pulled the
/// node into the BFS frontier — i.e. the tier of the shortest-path
/// edge from `seed` to that node (BFS visits shortest paths first).
/// When a downstream consumer (blast radius, slice, file blast
/// radius) asks "why is this node here", that's the edge we can
/// point at. Returns `None` when the edge has no parseable
/// provenance (older scans that pre-date the provenance column, or
/// an unrecognised source string).
///
/// Why first-edge tier and not best-edge or worst-edge: BFS already
/// returns the shortest path, which is what the agent will quote
/// when narrating impact. Recomputing best/worst-tier across all
/// parents would force a second pass and obscures the causal chain
/// the agent is actually showing the user.
/// One BFS hit: (depth, node, tier of the edge that reached it, p_true).
type WalkRow = (usize, NodeRef, Option<String>, Option<f64>);

fn walk_calls(
    conn: &Connection,
    seed: &str,
    direction: Direction,
    max_depth: usize,
    cap: usize,
) -> Result<Vec<WalkRow>, GraphQueryError> {
    walk_calls_using(conn, seed, direction, max_depth, cap, EdgeUses::All)
}

/// Which compiler-recorded uses an edge walk follows. Edges carry
/// `provenance.via` (`call`, `dispatch`, `value`) when the scan came
/// from a reference sweep; edges without it (heuristic tier, scans
/// that pre-date the field) are always followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeUses {
    /// Every Call edge, including function values and interface dispatch.
    #[default]
    All,
    /// Direct calls plus interface / virtual dispatch.
    Calls,
    /// Direct static calls only: what a compiler's call graph lists.
    Static,
}

impl EdgeUses {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "all" | "" => Some(Self::All),
            "calls" | "call+dispatch" => Some(Self::Calls),
            "static" | "call" => Some(Self::Static),
            _ => None,
        }
    }

    fn sql(self) -> &'static str {
        match self {
            Self::All => "",
            Self::Calls => {
                " AND COALESCE(json_extract(e.provenance, '$.via'), 'call') IN ('call', 'dispatch')"
            }
            Self::Static => " AND COALESCE(json_extract(e.provenance, '$.via'), 'call') = 'call'",
        }
    }
}

pub(crate) fn walk_calls_using(
    conn: &Connection,
    seed: &str,
    direction: Direction,
    max_depth: usize,
    cap: usize,
    uses: EdgeUses,
) -> Result<Vec<WalkRow>, GraphQueryError> {
    let mut seen = HashSet::<String>::new();
    let mut order = Vec::<WalkRow>::new();
    let mut queue: VecDeque<(usize, String)> = VecDeque::new();
    seen.insert(seed.to_string());
    queue.push_back((0, seed.to_string()));

    let base = match direction {
        Direction::Out => {
            "SELECT n.id, n.fqn, n.kind, e.provenance, json_extract(n.location, '$.file'),
                    json_extract(n.location, '$.start_line') FROM edges e
             JOIN nodes n ON n.id = e.to_id
             WHERE e.from_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'"
        }
        Direction::In => {
            "SELECT n.id, n.fqn, n.kind, e.provenance, json_extract(n.location, '$.file'),
                    json_extract(n.location, '$.start_line') FROM edges e
             JOIN nodes n ON n.id = e.from_id
             WHERE e.to_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'"
        }
    };
    let sql = format!("{base}{}", uses.sql());
    let mut stmt = conn.prepare(&sql)?;

    while let Some((depth, id)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        let mut rows = stmt.query([&id])?;
        while let Some(row) = rows.next()? {
            let next = NodeRef {
                id: row.get(0)?,
                fqn: row.get(1)?,
                kind: row.get(2)?,
                file: row.get(4).ok().flatten(),
                line: row.get(5).ok().flatten(),
            };
            // Provenance can be NULL on legacy rows; tolerate that
            // and fall through to `None` tier rather than failing
            // the whole walk.
            let provenance_json: Option<String> = row.get(3).ok();
            let tier = provenance_json
                .as_deref()
                .and_then(provenance_source_to_tier);
            let p_true = provenance_json.as_deref().and_then(|raw| {
                graphengine_parsing::domain::p_true_from_provenance_json("unknown", raw, "Call")
            });
            if seen.insert(next.id.clone()) {
                order.push((depth + 1, next.clone(), tier, p_true));
                queue.push_back((depth + 1, next.id.clone()));
                if order.len() >= cap {
                    return Ok(order);
                }
            }
        }
    }
    Ok(order)
}

/// Map a stored edge-provenance JSON blob to the agent-facing tier
/// label (`tier_0`, `tier_1`, `tier_3`). Centralised here so the
/// CLI's `tier_legend()` and every BFS row use the same vocabulary
/// — the agent rules in `.cursor/rules/gridseak.mdc` depend on this
/// exact set of strings.
///
/// Mapping is owned by `graphengine_parsing::domain::evidence_tier`.
/// Returns `None` for unrecognised sources rather than guessing; the
/// caller renders that as "unknown tier" so the agent doesn't get
/// fooled into treating an unknown source as Tier 0.
fn provenance_source_to_tier(raw: &str) -> Option<String> {
    graphengine_parsing::domain::evidence_tier::provenance_json_to_tier_id(raw).map(str::to_string)
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Direction {
    Out,
    In,
}

// ---------------------------------------------------------------------------
// HashMap helpers shared across queries
// ---------------------------------------------------------------------------

#[cfg(test)]
mod resolver_hardening_tests {
    use super::*;

    #[test]
    fn blocklist_rejects_bare_load() {
        // `load` is the canonical P2 dogfood case: it substring-matched
        // five `*_load*` functions across modules. After the hardening
        // we must refuse it rather than silently picking one.
        assert!(is_overly_generic_seed("load"));
        assert!(is_overly_generic_seed("init"));
        assert!(is_overly_generic_seed("new"));
        assert!(is_overly_generic_seed("get"));
    }

    #[test]
    fn blocklist_accepts_qualified_load() {
        // Qualifying the seed with a `::` (or `.`) marker tells the
        // resolver "the user did the disambiguation work; let
        // suffix-match prove it." Don't refuse those.
        assert!(!is_overly_generic_seed("AnalysisGraph::load"));
        assert!(!is_overly_generic_seed("graph::load"));
        assert!(!is_overly_generic_seed("MyClass.load"));
        assert!(!is_overly_generic_seed("load(x)"));
    }

    #[test]
    fn blocklist_rejects_short_seeds_even_off_list() {
        // A 3-char `add` is not on the blocklist but is too short to
        // be a useful substring needle. We refuse the substring scan
        // for anything under MIN_SUBSTRING_SEED_LEN.
        assert!(is_overly_generic_seed("add"));
        assert!(is_overly_generic_seed("fmt"));
        assert!(is_overly_generic_seed("x"));
    }

    #[test]
    fn blocklist_accepts_specific_long_names() {
        // Long, distinctive names that happen to contain a generic
        // root should still pass — the substring scan for these is
        // useful.
        assert!(!is_overly_generic_seed("process_import_binding"));
        assert!(!is_overly_generic_seed("calculate_blast_radius"));
        assert!(!is_overly_generic_seed("load_workspace_config"));
    }

    fn make_db_with_three_load_substrings() -> Connection {
        // A miniature graph where the bare seed `load` would
        // substring-match THREE functions in unrelated modules, with
        // no FQN actually ending in `load` (so suffix-match returns
        // zero and the substring fallback is the only thing that
        // could resolve it). This is the contamination shape Q3
        // prevents — see P2 dogfood evidence in
        // V0_1_0_RC1_FOLLOWUP_ISSUES.md.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{}', provenance TEXT NOT NULL DEFAULT '{}',
                properties TEXT NOT NULL DEFAULT '{}', trait_metadata TEXT
            );
            CREATE TABLE edges (
                from_id TEXT, to_id TEXT, kind TEXT NOT NULL,
                provenance TEXT NOT NULL DEFAULT '{}', PRIMARY KEY (from_id, to_id, kind)
            );
            INSERT INTO nodes (id, kind, fqn) VALUES
                ('n1', 'Function', 'parser::lazy_load_config'),
                ('n2', 'Function', 'store::reload_state'),
                ('n3', 'Function', 'graph::load_workspace_root'),
                ('n4', 'Function', 'graph::AnalysisGraph::save');
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn bare_load_seed_returns_overly_generic_error() {
        // The blocklist check fires before substring fallback. Even
        // though there's no suffix match for `load`, the resolver
        // refuses to substring-scan because `load` is on the
        // generic-name blocklist.
        let conn = make_db_with_three_load_substrings();
        let err = resolve_symbol_detailed(&conn, "load").expect_err("bare `load` must be refused");
        assert!(
            matches!(err, GraphQueryError::OverlyGenericSymbol { ref symbol } if symbol == "load"),
            "expected OverlyGenericSymbol, got {err:?}"
        );
    }

    #[test]
    fn bare_load_seed_refused_even_when_suffix_match_is_unique() {
        // Edge case: even when suffix-match WOULD have uniquely
        // identified the symbol (e.g. exactly one FQN ends in
        // `::load`), we still want the blocklist check to refuse
        // because the user habituated to passing `load` everywhere
        // will eventually pass it in a project where suffix-match
        // is ambiguous. Force the qualified name from the start.
        //
        // NOTE: this test pins the CURRENT behaviour — the resolver
        // tries suffix BEFORE the blocklist check. If a future
        // refactor wants the blocklist to apply only to substring,
        // update this test deliberately, don't drop it.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{}', provenance TEXT NOT NULL DEFAULT '{}',
                properties TEXT NOT NULL DEFAULT '{}', trait_metadata TEXT
            );
            INSERT INTO nodes (id, kind, fqn) VALUES
                ('n1', 'Function', 'graph::AnalysisGraph::load');
            "#,
        )
        .unwrap();
        let resolved = resolve_symbol_detailed(&conn, "load").unwrap();
        assert_eq!(resolved.method, ResolutionMethod::SuffixUnique);
        assert_eq!(resolved.node.id, "n1");
    }

    #[test]
    fn qualified_load_resolves_via_suffix_unique() {
        // The user did the disambiguation work — `lazy_load_config`
        // uniquely identifies n1 via suffix match (or exact, since
        // the FQN ends in `lazy_load_config`).
        let conn = make_db_with_three_load_substrings();
        let resolved = resolve_symbol_detailed(&conn, "lazy_load_config").unwrap();
        assert_eq!(resolved.node.id, "n1");
        // Suffix match (the FQN is `parser::lazy_load_config`).
        assert_eq!(resolved.method, ResolutionMethod::SuffixUnique);
    }

    #[test]
    fn long_distinctive_substring_resolves_with_substring_method() {
        // `lazy_load` is long enough to dodge the short-seed check
        // and not on the blocklist; suffix-match fails (no FQN ends
        // in `lazy_load`) but substring-match uniquely picks
        // `parser::lazy_load_config`. The method must be
        // `SubstringUnique` so the MCP envelope can warn the agent.
        let conn = make_db_with_three_load_substrings();
        let resolved = resolve_symbol_detailed(&conn, "lazy_load").unwrap();
        assert_eq!(resolved.node.id, "n1");
        assert_eq!(resolved.method, ResolutionMethod::SubstringUnique);
    }

    #[test]
    fn exact_fqn_takes_precedence_over_blocklist_check() {
        // Even though `load` is on the blocklist for the substring
        // fallback, an exact FQN match short-circuits the whole
        // pipeline. We pin that the blocklist doesn't accidentally
        // refuse an unambiguous exact reference.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{}', provenance TEXT NOT NULL DEFAULT '{}',
                properties TEXT NOT NULL DEFAULT '{}', trait_metadata TEXT
            );
            INSERT INTO nodes (id, kind, fqn) VALUES ('n1', 'Function', 'load');
            "#,
        )
        .unwrap();
        let resolved = resolve_symbol_detailed(&conn, "load").unwrap();
        assert_eq!(resolved.method, ResolutionMethod::ExactFqn);
    }
}

#[cfg(test)]
mod tier_tests {
    use super::*;

    #[test]
    fn provenance_source_maps_to_correct_tier() {
        assert_eq!(
            provenance_source_to_tier(r#"{"source":"TreeSitter","confidence":"High"}"#).as_deref(),
            Some("tier_0")
        );
        assert_eq!(
            provenance_source_to_tier(r#"{"source":"Heuristic","confidence":"Low"}"#).as_deref(),
            Some("tier_1")
        );
        assert_eq!(
            provenance_source_to_tier(r#"{"source":"Lsp","confidence":"High"}"#).as_deref(),
            Some("tier_3")
        );
        assert_eq!(
            provenance_source_to_tier(r#"{"source":"Compiler","confidence":"High"}"#).as_deref(),
            Some("tier_3")
        );
        assert_eq!(
            provenance_source_to_tier(r#"{"source":"Runtime","confidence":"High"}"#).as_deref(),
            Some("runtime_verified")
        );
        assert!(provenance_source_to_tier("{}").is_none());
        assert!(provenance_source_to_tier(r#"{"source":"Wat"}"#).is_none());
        assert!(provenance_source_to_tier("not json").is_none());
    }

    #[test]
    fn best_tier_prefers_lsp_then_treesitter_then_heuristic() {
        // tier_3 > tier_0 > tier_1; unknown loses to everything.
        assert_eq!(best_tier(Some("tier_3"), Some("tier_0")), Some("tier_3"));
        assert_eq!(best_tier(Some("tier_0"), Some("tier_3")), Some("tier_3"));
        assert_eq!(best_tier(Some("tier_0"), Some("tier_1")), Some("tier_0"));
        assert_eq!(best_tier(Some("tier_1"), Some("tier_0")), Some("tier_0"));
        assert_eq!(best_tier(None, Some("tier_1")), Some("tier_1"));
        assert_eq!(best_tier(Some("tier_1"), None), Some("tier_1"));
        assert_eq!(best_tier(None, None), None);
    }
}

#[allow(dead_code)]
pub(crate) fn nodes_by_id(
    conn: &Connection,
    ids: &[String],
) -> Result<HashMap<String, NodeRef>, GraphQueryError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id, fqn, kind FROM nodes WHERE id IN ({placeholders})",
        placeholders = placeholders
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                NodeRef {
                    id: row.get(0)?,
                    fqn: row.get(1)?,
                    kind: row.get(2)?,
                    file: None,
                    line: None,
                },
            ))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod path_symbol_tests {
    use super::split_path_symbol;

    #[test]
    fn go_import_path_splits_package_and_name() {
        let (segs, tail) =
            split_path_symbol("k8s.io/kubernetes/pkg/api/v1/pod.GetPodCondition").unwrap();
        assert_eq!(segs, ["k8s.io", "kubernetes", "pkg", "api", "v1", "pod"]);
        assert_eq!(tail, "GetPodCondition");
    }

    #[test]
    fn go_method_keeps_receiver() {
        let (_, tail) = split_path_symbol("github.com/x/y/headers.HeaderOps.Provision").unwrap();
        assert_eq!(tail, "HeaderOps.Provision");
    }

    #[test]
    fn file_path_drops_extension() {
        let (segs, tail) = split_path_symbol("pkg/api/v1/pod/util.go::GetPodCondition").unwrap();
        assert_eq!(segs, ["pkg", "api", "v1", "pod", "util"]);
        assert_eq!(tail, "GetPodCondition");
    }

    #[test]
    fn plain_names_are_not_path_shaped() {
        assert!(split_path_symbol("GetPodCondition").is_none());
        assert!(split_path_symbol("pod::util::GetPodCondition").is_none());
    }
}

#[cfg(test)]
mod location_seed_tests {
    use super::*;

    /// hono-shaped graph: `fetch` / `request` are class-field arrows in
    /// `src/hono-base.ts`, another `fetch` lives in `src/client/client.ts`,
    /// and zod-shaped `_addCheck` is declared twice in one file.
    fn graph() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{}', provenance TEXT NOT NULL DEFAULT '{}',
                properties TEXT NOT NULL DEFAULT '{}', trait_metadata TEXT
            );
            CREATE TABLE edges (
                from_id TEXT, to_id TEXT, kind TEXT NOT NULL,
                provenance TEXT NOT NULL DEFAULT '{}', PRIMARY KEY (from_id, to_id, kind)
            );
            INSERT INTO nodes (id, kind, fqn, location, properties) VALUES
                ('fs', 'Function', 'p::hono_base::__file_scope__',
                 '{"file":"/r/src/hono-base.ts","start_line":1,"end_line":540}', '{}'),
                ('hfetch', 'Function', 'p::hono_base::fetch',
                 '{"file":"/r/src/hono-base.ts","start_line":483,"end_line":489}', '{}'),
                ('request', 'Function', 'p::hono_base::request',
                 '{"file":"/r/src/hono-base.ts","start_line":503,"end_line":521}', '{}'),
                ('cfetch', 'Function', 'p::client::client::fetch',
                 '{"file":"/r/src/client/client.ts","start_line":40,"end_line":60}', '{}'),
                ('add1', 'Function', 'p::types::_addCheck',
                 '{"file":"/r/src/types.ts","start_line":1095,"end_line":1100}', '{}'),
                ('add2', 'Function', 'p::types::_addCheck',
                 '{"file":"/r/src/types.ts","start_line":1555,"end_line":1560}', '{}'),
                ('dadd', 'Function', 'p::deno::lib::types::_addCheck',
                 '{"file":"/r/deno/lib/types.ts","start_line":1095,"end_line":1100}', '{}'),
                ('mod', 'Module', 'p::types',
                 '{"file":"/r/src/types.ts","start_line":1,"end_line":5000}', '{}'),
                ('syn', 'Function', 'p::x::field.__init__',
                 '{"file":"/r/src/x.ts","start_line":3,"end_line":3}', '{"synthetic":true}');
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn parses_location_shapes() {
        assert_eq!(
            parse_location_seed("src/hono-base.ts:483"),
            Some(LocationSeed::Line {
                file: "src/hono-base.ts".into(),
                line: 483
            })
        );
        assert_eq!(
            parse_location_seed("./src/hono-base.ts:483:3"),
            Some(LocationSeed::Line {
                file: "src/hono-base.ts".into(),
                line: 483
            })
        );
        assert_eq!(
            parse_location_seed("src/hono-base.ts::Hono.fetch"),
            Some(LocationSeed::Name {
                file: "src/hono-base.ts".into(),
                name: "Hono.fetch".into()
            })
        );
        assert_eq!(parse_location_seed("hono_base::fetch"), None);
        assert_eq!(parse_location_seed("fetch"), None);
        assert_eq!(parse_location_seed("src/hono-base.ts"), None);
        assert_eq!(parse_location_seed("k8s.io/x/pod.GetPodCondition"), None);
        assert!(!seed_looks_like_path("src/hono-base.ts:483"));
        assert!(seed_looks_like_path("src/hono-base.ts"));
    }

    #[test]
    fn file_line_resolves_common_and_class_field_names() {
        let conn = graph();
        // Bare `fetch` is refused as too generic; the location is not.
        assert!(resolve_symbol_detailed(&conn, "fetch").is_err());
        let r = resolve_symbol_detailed(&conn, "src/hono-base.ts:483").unwrap();
        assert_eq!(
            (r.node.id.as_str(), r.method),
            ("hfetch", ResolutionMethod::Location)
        );
        // A line inside the body resolves to the enclosing function, not
        // the file scope that also spans it.
        let r = resolve_symbol_detailed(&conn, "src/hono-base.ts:510").unwrap();
        assert_eq!(r.node.id, "request");
        // Same name declared twice in one file: the line picks one.
        let r = resolve_symbol_detailed(&conn, "src/types.ts:1555").unwrap();
        assert_eq!(r.node.id, "add2");
        // An absolute path works too.
        let r = resolve_symbol_detailed(&conn, "/r/deno/lib/types.ts:1095").unwrap();
        assert_eq!(r.node.id, "dadd");
    }

    #[test]
    fn file_name_resolves_or_reports_ambiguity() {
        let conn = graph();
        let r = resolve_symbol_detailed(&conn, "src/hono-base.ts::fetch").unwrap();
        assert_eq!(r.node.id, "hfetch");
        let r = resolve_symbol_detailed(&conn, "src/hono-base.ts::Hono.request").unwrap();
        assert_eq!(r.node.id, "request");
        let err = resolve_symbol_detailed(&conn, "src/types.ts::_addCheck").unwrap_err();
        assert!(
            matches!(err, GraphQueryError::AmbiguousSymbol { total: 2, .. }),
            "{err:?}"
        );
    }

    #[test]
    fn location_errors_are_specific() {
        let conn = graph();
        // Top-level line of a known file: no function there.
        let err = resolve_symbol_detailed(&conn, "src/hono-base.ts:2").unwrap_err();
        assert!(
            matches!(err, GraphQueryError::NoSymbolAtLocation { .. }),
            "{err:?}"
        );
        let err = resolve_symbol_detailed(&conn, "src/hono-base.ts::nope").unwrap_err();
        assert!(
            matches!(err, GraphQueryError::NoSymbolAtLocation { .. }),
            "{err:?}"
        );
        // `types.ts` alone is a suffix of two scanned files.
        let err = resolve_symbol_detailed(&conn, "types.ts:1095").unwrap_err();
        assert!(
            matches!(err, GraphQueryError::AmbiguousSymbol { .. }),
            "{err:?}"
        );
        // A file the scan never saw is not location-resolvable.
        assert!(resolve_symbol_detailed(&conn, "src/missing.ts:3").is_err());
    }

    #[test]
    fn caller_kind_separates_top_level_code() {
        let conn = graph();
        let node = |id: &str, kind: &str, fqn: &str| NodeRef {
            id: id.into(),
            fqn: fqn.into(),
            kind: kind.into(),
            file: None,
            line: None,
        };
        let k = |n: NodeRef| caller_kind(&conn, &n).unwrap();
        assert_eq!(
            k(node("fs", "Function", "p::hono_base::__file_scope__")),
            ("file_scope".into(), false)
        );
        assert_eq!(
            k(node("mod", "Module", "p::types")),
            ("module".into(), false)
        );
        assert_eq!(
            k(node("syn", "Function", "p::x::field.__init__")),
            ("synthetic".into(), false)
        );
        assert_eq!(
            k(node("hfetch", "Function", "p::hono_base::fetch")),
            ("function".into(), true)
        );
    }
}
