//! PR-level deletion safety: compare a base and a head scan of the same repo.
//!
//! A function removed between base and head is *broken* when a caller the
//! compiler proved at base still exists at head and still names it, while no
//! function of that name exists anywhere at head. Callers that were rewritten
//! or deleted in the same change are not flagged — the per-edit gate cannot
//! see that, which is why this check exists.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

#[derive(Args, Debug, Clone)]
pub struct PrCheckArgs {
    /// Graph sqlite of the base (pre-change) scan.
    #[arg(long)]
    pub base: PathBuf,
    /// Graph sqlite of the head (post-change) scan.
    #[arg(long)]
    pub head: PathBuf,
    /// Repo root the base scan was taken from.
    #[arg(long)]
    pub base_root: PathBuf,
    /// Repo root the head scan was taken from (caller text is read here).
    #[arg(long)]
    pub head_root: PathBuf,
    /// `json` (default) or `markdown` (PR comment).
    #[arg(long, default_value = "json")]
    pub format: String,
}

#[derive(Debug, Clone)]
struct FnNode {
    id: String,
    /// (repo-relative file, local FQN + declaration header). The header keeps
    /// same-named functions and methods in one file apart: node FQNs omit the
    /// receiver/class, so `URLParam` and `Context.URLParam` would collide.
    key: (String, String),
    /// Local FQN for display.
    name: String,
    /// Normalized declaration line; joins the key only for duplicated names.
    header: String,
    /// Only functions can be "removed"; other kinds appear as callers.
    is_function: bool,
    short: String,
    file_abs: String,
    start_line: u32,
    end_line: u32,
}

#[derive(Debug, Serialize)]
pub struct SurvivingCaller {
    pub caller: String,
    pub file: String,
    pub line: u32,
    /// `x.name` rather than a bare `name`: in dynamic languages an
    /// unresolved attribute may just be an untyped receiver.
    pub attribute: bool,
}

#[derive(Debug, Serialize)]
pub struct RemovedSymbol {
    pub symbol: String,
    pub file: String,
    /// `broken`: no definition left; `moved`: same name defined elsewhere at head.
    pub status: String,
    pub moved_to: Vec<String>,
    pub surviving_callers: Vec<SurvivingCaller>,
}

#[derive(Debug, Serialize)]
pub struct PrCheckReport {
    pub verdict: String,
    pub removed_functions: usize,
    pub removed_with_compiler_callers: usize,
    pub callers_updated_in_change: usize,
    /// Share of base Call edges proven by the compiler tier. With no
    /// compiler tier the check cannot prove anything, so it says so.
    pub base_compiler_call_ratio: f64,
    pub findings: Vec<RemovedSymbol>,
}

pub fn run(args: PrCheckArgs) -> Result<i32> {
    let report = check(&args.base, &args.head, &args.base_root, &args.head_root)?;
    if args.format == "markdown" {
        println!("{}", render_markdown(&report));
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }
    Ok(if report.verdict == "broken" { 1 } else { 0 })
}

pub fn check(
    base: &Path,
    head: &Path,
    base_root: &Path,
    head_root: &Path,
) -> Result<PrCheckReport> {
    let base_conn = open(base)?;
    let head_conn = open(head)?;
    let mut base_fns = load_functions(&base_conn, base_root)?;
    let mut head_fns = load_functions(&head_conn, head_root)?;
    disambiguate(&mut base_fns, &mut head_fns);

    let head_by_key: HashMap<&(String, String), &FnNode> =
        head_fns.iter().map(|n| (&n.key, n)).collect();
    let mut head_by_short: HashMap<&str, Vec<&FnNode>> = HashMap::new();
    for n in &head_fns {
        head_by_short.entry(n.short.as_str()).or_default().push(n);
    }
    let base_by_id: HashMap<&str, &FnNode> = base_fns.iter().map(|n| (n.id.as_str(), n)).collect();
    let head_by_id: HashMap<&str, &FnNode> = head_fns.iter().map(|n| (n.id.as_str(), n)).collect();
    let removed: Vec<&FnNode> = base_fns
        .iter()
        .filter(|n| {
            n.is_function && n.short != "__file_scope__" && !head_by_key.contains_key(&n.key)
        })
        .collect();
    let removed_ids: HashSet<&str> = removed.iter().map(|n| n.id.as_str()).collect();

    let mut text_cache: HashMap<String, Vec<String>> = HashMap::new();
    let indexes = Indexes::load(base_root, head_root);
    let mut findings = Vec::new();
    let mut with_callers = 0;
    let mut updated = 0;
    for sym in &removed {
        let callers = compiler_callers(&base_conn, &sym.id)?;
        let callers: Vec<&FnNode> = callers
            .iter()
            .filter(|id| !removed_ids.contains(id.as_str()))
            .filter_map(|id| base_by_id.get(id.as_str()).copied())
            .collect();
        if callers.is_empty() {
            continue;
        }
        with_callers += 1;
        let base_moniker = indexes.base_moniker(sym);
        let mut surviving = Vec::new();
        for caller in callers {
            let Some(head_caller) = head_by_key.get(&caller.key) else {
                continue;
            };
            // Without a head index, fall back to: the compiler links the caller
            // to a same-named function at head, so it was repointed. With an
            // index, each mention is resolved on its own below (a same-named
            // method elsewhere must not hide a dangling plain call).
            if !indexes.has_head()
                && compiler_calls_short(&head_conn, &head_caller.id, &sym.short, &head_by_id)?
            {
                updated += 1;
                continue;
            }
            let mentions = mentions(&mut text_cache, head_caller, &sym.short);
            let mut dangling: Vec<Mention> = mentions
                .into_iter()
                .filter(|m| {
                    indexes.is_dangling(&head_caller.key.0, m.line, m.col, base_moniker.as_deref())
                })
                .collect();
            // Report a bare use first: it is unambiguous.
            dangling.sort_by_key(|m| m.attribute);
            match dangling.first() {
                Some(m) => surviving.push(SurvivingCaller {
                    caller: caller.name.clone(),
                    file: caller.key.0.clone(),
                    line: m.line,
                    attribute: m.attribute,
                }),
                None => updated += 1,
            }
        }
        if surviving.is_empty() {
            continue;
        }
        let moved_to: Vec<String> = head_by_short
            .get(sym.short.as_str())
            .map(|v| {
                v.iter()
                    .map(|n| format!("{}::{}", n.key.0, n.name))
                    .collect()
            })
            .unwrap_or_default();
        findings.push(RemovedSymbol {
            symbol: sym.name.clone(),
            file: sym.key.0.clone(),
            // A bare name the compiler cannot resolve is broken outright. When
            // every dangling use is `x.name` (or there is no head index) and the
            // name still exists elsewhere, it may have moved: ask for review.
            status: if moved_to.is_empty()
                || (indexes.has_head()
                    && surviving
                        .iter()
                        .any(|c| !c.attribute || statically_typed(&c.file)))
            {
                "broken"
            } else {
                "moved"
            }
            .into(),
            moved_to,
            surviving_callers: surviving,
        });
    }
    let compiler_ratio = compiler_call_ratio(&base_conn)?;
    let verdict = if findings.iter().any(|f| f.status == "broken") {
        "broken"
    } else if compiler_ratio == 0.0 && !removed.is_empty() {
        "unverified"
    } else if findings.is_empty() {
        "safe"
    } else {
        "review"
    };
    Ok(PrCheckReport {
        verdict: verdict.into(),
        removed_functions: removed.len(),
        removed_with_compiler_callers: with_callers,
        callers_updated_in_change: updated,
        base_compiler_call_ratio: compiler_ratio,
        findings,
    })
}

fn open(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open graph {}", path.display()))
}

fn load_functions(conn: &Connection, root: &Path) -> Result<Vec<FnNode>> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut stmt = conn.prepare(
        // Callers can be any declaration (Go top-level code is credited to its
        // file module); only functions are candidates for removal.
        "SELECT id, fqn, location, kind FROM nodes WHERE kind NOT IN ('Project', 'Folder', 'File', 'Crate')",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let owners = load_owners(conn)?;
    let mut out = Vec::new();
    let mut text_cache: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let (id, fqn, loc, kind) = row?;
        let v: serde_json::Value = serde_json::from_str(&loc).unwrap_or_default();
        let file_abs = v["file"].as_str().unwrap_or("").to_string();
        let abs = Path::new(&file_abs);
        let abs = abs.canonicalize().unwrap_or_else(|_| abs.to_path_buf());
        let rel = abs
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| file_abs.clone());
        // FQNs lead with the scanned folder's name; drop it so two checkouts compare.
        let local = fqn
            .split_once("::")
            .map(|(_, rest)| rest)
            .unwrap_or(&fqn)
            .to_string();
        // Last path segment, past any `Receiver.` / `Class.` qualifier.
        let last = local.rsplit("::").next().unwrap_or(&local);
        let short = last.rsplit('.').next().unwrap_or(last).to_string();
        // `__file_scope__` nodes stand for module top-level code: they are
        // real callers (`const x = helper()` at file level) and never removed
        // on their own, so keep them.
        let start = v["start_line"].as_u64().unwrap_or(0) as u32;
        let end = v["end_line"].as_u64().unwrap_or(0) as u32;
        // Node FQNs omit the class, so `A._parse` and `B._parse` share one;
        // the tightest enclosing class/struct range tells them apart.
        let local = match owners.get(&file_abs).and_then(|o| {
            o.iter()
                .filter(|(s0, e0, _)| *s0 <= start && end <= *e0)
                .min_by_key(|(s0, e0, _)| e0 - s0)
        }) {
            Some((_, _, owner)) => match local.rsplit_once("::") {
                Some((prefix, name)) => format!("{prefix}::{owner}.{name}"),
                None => format!("{owner}.{local}"),
            },
            None => local,
        };
        let start_line = start;
        let lines = text_cache.entry(file_abs.clone()).or_insert_with(|| {
            std::fs::read_to_string(&file_abs)
                .map(|s| s.lines().map(str::to_string).collect())
                .unwrap_or_default()
        });
        let header = lines
            .get(start_line.saturating_sub(1) as usize)
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        out.push(FnNode {
            id,
            is_function: matches!(kind.as_str(), "Function" | "Method" | "Constructor"),
            key: (rel, local.clone()),
            header,
            name: local,
            short,
            file_abs,
            start_line,
            end_line: end,
        });
    }
    Ok(out)
}

/// Names that occur more than once in a file on either side (Go function vs.
/// method of the same name, overloads) get their declaration line appended,
/// so a signature edit to a unique function is never mistaken for a removal.
fn disambiguate(base: &mut [FnNode], head: &mut [FnNode]) {
    let mut counts: HashMap<(String, String), (usize, usize)> = HashMap::new();
    for n in base.iter() {
        counts.entry(n.key.clone()).or_default().0 += 1;
    }
    for n in head.iter() {
        counts.entry(n.key.clone()).or_default().1 += 1;
    }
    for n in base.iter_mut().chain(head.iter_mut()) {
        if counts.get(&n.key).is_some_and(|(b, h)| *b > 1 || *h > 1) {
            n.key.1 = format!("{}\u{1f}{}", n.key.1, n.header);
        }
    }
}

/// `file → [(start_line, end_line, short name)]` for class-like nodes.
type Owners = HashMap<String, Vec<(u32, u32, String)>>;

fn load_owners(conn: &Connection) -> Result<Owners> {
    let mut stmt = conn.prepare(
        "SELECT fqn, location FROM nodes WHERE kind IN ('Struct', 'Class', 'Interface', 'Trait', 'Enum')",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out: Owners = HashMap::new();
    for row in rows {
        let (fqn, loc) = row?;
        let v: serde_json::Value = serde_json::from_str(&loc).unwrap_or_default();
        let file = v["file"].as_str().unwrap_or("").to_string();
        let start = v["start_line"].as_u64().unwrap_or(0) as u32;
        let end = v["end_line"].as_u64().unwrap_or(0) as u32;
        if end > start {
            let short = fqn.rsplit("::").next().unwrap_or(&fqn).to_string();
            out.entry(file).or_default().push((start, end, short));
        }
    }
    Ok(out)
}

fn compiler_call_ratio(conn: &Connection) -> Result<f64> {
    let (compiler, total): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(json_extract(provenance, '$.source') = 'Compiler'), 0), COUNT(*)
           FROM edges WHERE json_extract(kind, '$.kind') = 'Call'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(if total == 0 {
        0.0
    } else {
        compiler as f64 / total as f64
    })
}

fn compiler_callers(conn: &Connection, target: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT from_id FROM edges WHERE to_id = ?1
           AND json_extract(kind, '$.kind') = 'Call'
           AND json_extract(provenance, '$.source') = 'Compiler'",
    )?;
    let ids = stmt
        .query_map([target], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

fn compiler_calls_short(
    conn: &Connection,
    caller: &str,
    short: &str,
    by_id: &HashMap<&str, &FnNode>,
) -> Result<bool> {
    let mut stmt = conn.prepare_cached(
        "SELECT to_id FROM edges WHERE from_id = ?1
           AND json_extract(kind, '$.kind') = 'Call'
           AND json_extract(provenance, '$.source') = 'Compiler'",
    )?;
    let ids = stmt
        .query_map([caller], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids
        .iter()
        .any(|id| by_id.get(id.as_str()).is_some_and(|n| n.short == short)))
}

/// Every (1-based line, 0-based col) inside `node` where code — not comments,
/// strings or docstrings — names `ident`.
struct Mention {
    line: u32,
    col: u32,
    attribute: bool,
}

fn mentions(cache: &mut HashMap<String, Vec<String>>, node: &FnNode, ident: &str) -> Vec<Mention> {
    let lines = cache.entry(node.file_abs.clone()).or_insert_with(|| {
        std::fs::read_to_string(&node.file_abs)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    });
    let mut out = Vec::new();
    let mut in_block: Option<&'static str> = None;
    for line_no in node.start_line.max(1)..=node.end_line {
        let Some(text) = lines.get(line_no as usize - 1) else {
            break;
        };
        let code = code_only(text, &mut in_block);
        let chars: Vec<char> = code.chars().collect();
        for col in word_columns(&code, ident) {
            let attribute = chars[..col as usize]
                .iter()
                .rev()
                .find(|c| !c.is_whitespace())
                .is_some_and(|c| *c == '.');
            out.push(Mention {
                line: line_no,
                col,
                attribute,
            });
        }
    }
    out
}

/// SCIP indexes for both sides, when the scans produced them.
struct Indexes {
    base: Option<(PathBuf, graphengine_scip_adapter::ScipIndex)>,
    head: Option<(PathBuf, graphengine_scip_adapter::ScipIndex)>,
}

impl Indexes {
    fn load(base_root: &Path, head_root: &Path) -> Self {
        let load = |root: &Path| {
            let pattern = root.join(".gridseak/scip");
            let dir = std::fs::read_dir(&pattern).ok()?;
            for entry in dir.flatten() {
                let idx = entry.path().join("index.scip");
                if let Ok(index) = graphengine_scip_adapter::ScipIndex::from_file(&idx) {
                    return Some((root.to_path_buf(), index));
                }
            }
            None
        };
        Self {
            base: load(base_root),
            head: load(head_root),
        }
    }

    fn has_head(&self) -> bool {
        self.head.is_some()
    }

    /// The removed function's SCIP moniker, read from its base definition.
    fn base_moniker(&self, sym: &FnNode) -> Option<String> {
        let (_, index) = self.base.as_ref()?;
        let first = sym.start_line.saturating_sub(1);
        index
            .occurrences_in(&sym.key.0)
            .into_iter()
            .filter(|(_, roles, r)| {
                roles & 1 != 0 && r.start_line >= first && r.start_line <= first + 3
            })
            .map(|(symbol, _, _)| symbol)
            .find(|symbol| names_function(symbol, &sym.short))
    }

    /// A mention is dangling when the head index resolves nothing at it, or
    /// still resolves it to the removed function's moniker. Without a head
    /// index every code mention counts.
    fn is_dangling(&self, file: &str, line: u32, col: u32, removed: Option<&str>) -> bool {
        let Some((_, index)) = self.head.as_ref() else {
            return true;
        };
        let occurrences = index.occurrences_in(file);
        let hit = occurrences
            .iter()
            .find(|(_, _, r)| r.start_line + 1 == line && r.start_character == col);
        match hit {
            None => true,
            // A resolved local always has a definition in its own document;
            // indexers bind names they cannot resolve (e.g. a dead import)
            // to a definition-less local.
            Some((symbol, _, _)) if symbol.starts_with("local ") => !occurrences
                .iter()
                .any(|(s, roles, _)| s == symbol && roles & 1 != 0),
            Some((symbol, _, _)) => {
                removed == Some(symbol.as_str()) && index.definition_of(symbol).is_none()
            }
        }
    }
}

/// Files whose compiler resolves every member access: an unresolved `x.name`
/// there is a real break, not an untyped receiver.
fn statically_typed(file: &str) -> bool {
    [".ts", ".tsx", ".go", ".rs", ".java", ".cs", ".kt", ".scala"]
        .iter()
        .any(|ext| file.ends_with(ext))
}

fn names_function(symbol: &str, short: &str) -> bool {
    [
        format!("{short}()."),
        format!("{short}."),
        format!("`{short}`()."),
    ]
    .iter()
    .any(|suffix| {
        symbol.ends_with(suffix.as_str())
            && symbol[..symbol.len() - suffix.len()]
                .chars()
                .next_back()
                .is_some_and(|c| matches!(c, '/' | '#' | '.' | ' '))
    })
}

/// The code on one line with comments and string/docstring contents replaced
/// by spaces, so columns still line up with the source.
/// `in_block` carries an open `"""`, `'''` or `/*` across lines.
fn code_only(line: &str, in_block: &mut Option<&'static str>) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let starts = |i: usize, pat: &str| {
        pat.chars()
            .enumerate()
            .all(|(k, p)| chars.get(i + k) == Some(&p))
    };
    let mut i = 0;
    while i < chars.len() {
        if let Some(close) = *in_block {
            if starts(i, close) {
                let n = close.chars().count();
                out.extend(std::iter::repeat_n(' ', n));
                i += n;
                *in_block = None;
            } else {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        let c = chars[i];
        if c == '#' || starts(i, "//") {
            out.extend(std::iter::repeat_n(' ', chars.len() - i));
            break;
        }
        if starts(i, "/*") {
            *in_block = Some("*/");
            out.extend([' ', ' ']);
            i += 2;
            continue;
        }
        if starts(i, "\"\"\"") || starts(i, "'''") {
            *in_block = Some(if c == '"' { "\"\"\"" } else { "'''" });
            out.extend([' ', ' ', ' ']);
            i += 3;
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            // Python f-strings: `{expr}` inside is code, like a JS template.
            let fstring = c != '`' && is_fstring_prefix(&chars[..i]);
            out.push(' ');
            i += 1;
            let mut escaped = false;
            while i < chars.len() {
                let d = chars[i];
                // Template literal `${expr}` / f-string `{expr}`: code.
                let template = c == '`' && !escaped && d == '$' && chars.get(i + 1) == Some(&'{');
                let fexpr = fstring && d == '{' && chars.get(i + 1) != Some(&'{');
                if fstring && d == '{' && chars.get(i + 1) == Some(&'{') {
                    out.extend([' ', ' ']);
                    i += 2;
                    continue;
                }
                if template || fexpr {
                    let skip = if template { 2 } else { 1 };
                    out.extend(std::iter::repeat_n(' ', skip));
                    i += skip;
                    let mut depth = 1;
                    while i < chars.len() {
                        match chars[i] {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                        if depth == 0 {
                            out.push(' ');
                            i += 1;
                            break;
                        }
                        out.push(chars[i]);
                        i += 1;
                    }
                    continue;
                }
                out.push(' ');
                i += 1;
                if escaped {
                    escaped = false;
                } else if d == '\\' {
                    escaped = true;
                } else if d == c {
                    break;
                }
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out.into_iter().collect()
}

/// Whether the string prefix right before a quote (`f`, `rf`, `Fr`, …) makes
/// it a Python f-string. The prefix must not be the tail of an identifier.
fn is_fstring_prefix(before: &[char]) -> bool {
    let prefix: Vec<char> = before
        .iter()
        .rev()
        .take_while(|c| c.is_alphabetic())
        .copied()
        .collect();
    prefix.len() <= 2
        && prefix.iter().any(|c| c.eq_ignore_ascii_case(&'f'))
        && prefix.iter().all(|c| "fFrR".contains(*c))
}

/// Char columns where `word` appears as a whole identifier.
fn word_columns(hay: &str, word: &str) -> Vec<u32> {
    let chars: Vec<char> = hay.chars().collect();
    let w: Vec<char> = word.chars().collect();
    let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut cols = Vec::new();
    if w.is_empty() || chars.len() < w.len() {
        return cols;
    }
    for i in 0..=chars.len() - w.len() {
        if chars[i..i + w.len()] == w[..]
            && !(i > 0 && is_ident(chars[i - 1]))
            && !chars.get(i + w.len()).is_some_and(|c| is_ident(*c))
        {
            cols.push(i as u32);
        }
    }
    cols
}

pub fn render_markdown(r: &PrCheckReport) -> String {
    let mut s = String::new();
    let headline = match r.verdict.as_str() {
        "broken" => "🔴 **Deleted code is still called**",
        "review" => "🟡 **Moved code — check the callers' imports**",
        "unverified" => "⚪ **Not verified — no compiler index for this scan**",
        _ => "🟢 **Deletions are safe**",
    };
    s.push_str(&format!("### GridSeak deletion check: {headline}\n\n"));
    s.push_str(&format!(
        "{} functions removed · {} had compiler-proven callers · {} callers were updated in this change\n\n",
        r.removed_functions, r.removed_with_compiler_callers, r.callers_updated_in_change
    ));
    for f in &r.findings {
        s.push_str(&format!(
            "- `{}` ({}) — **{}**\n",
            f.symbol, f.file, f.status
        ));
        for c in &f.surviving_callers {
            s.push_str(&format!(
                "  - still named by `{}` at {}:{}\n",
                c.caller, c.file, c.line
            ));
        }
        for m in &f.moved_to {
            s.push_str(&format!("  - same name now defined at `{m}`\n"));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mentions_in(line: &str, block: &mut Option<&'static str>) -> Vec<u32> {
        word_columns(&code_only(line, block), "helper")
    }

    #[test]
    fn word_match_respects_identifier_boundaries() {
        assert_eq!(word_columns("return helper(x)", "helper"), vec![7]);
        assert!(word_columns("return my_helper(x)", "helper").is_empty());
        assert!(word_columns("return helpers(x)", "helper").is_empty());
    }

    #[test]
    fn comments_strings_and_docstrings_do_not_count_as_mentions() {
        let mut b = None;
        assert!(mentions_in("    # helper() was removed", &mut b).is_empty());
        assert!(mentions_in("  x = 1  // helper()", &mut b).is_empty());
        assert!(mentions_in("  log('helper failed')", &mut b).is_empty());
        assert_eq!(mentions_in("  y = helper(\"a\")", &mut b), vec![6]);
        assert!(mentions_in("    \"\"\"Create a", &mut b).is_empty());
        assert!(mentions_in("    calls helper twice", &mut b).is_empty());
        assert!(mentions_in("    \"\"\"", &mut b).is_empty());
        assert_eq!(mentions_in("    return helper()", &mut b), vec![11]);
        assert_eq!(mentions_in("  t = `^${helper(a)}$`;", &mut b), vec![10]);
        assert!(mentions_in("  t = `helper ${x}`;", &mut b).is_empty());
        assert_eq!(mentions_in("  s = f\"{helper(a)}\"", &mut b), vec![9]);
        assert!(mentions_in("  s = f\"{{helper}} {x}\"", &mut b).is_empty());
        assert!(mentions_in("  s = \"{helper(a)}\"", &mut b).is_empty());
    }

    #[test]
    fn moniker_suffix_names_the_function() {
        assert!(names_function(
            "scip-python python pkg 1 `pkg.mod`/helper().",
            "helper"
        ));
        assert!(names_function("scip-go gomod m v `m`/Mux#Use().", "Use"));
        assert!(!names_function(
            "scip-python python pkg 1 `pkg.mod`/my_helper().",
            "helper"
        ));
    }
}
