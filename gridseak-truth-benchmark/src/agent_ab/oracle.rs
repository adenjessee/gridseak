//! Read-only queries over a GridSeak scan artifact (same SQL as verify_claim).

use rusqlite::Connection;
use std::path::Path;

pub struct ArtifactOracle {
    conn: Connection,
    path: std::path::PathBuf,
}

#[derive(Debug, Clone)]
pub struct ClaimView {
    pub verdict: String,
    pub witnesses: Vec<String>,
}

impl ArtifactOracle {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    pub fn file_of(&self, id: &str) -> Option<String> {
        let loc: String = self
            .conn
            .query_row("SELECT location FROM nodes WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .ok()?;
        let v: serde_json::Value = serde_json::from_str(&loc).ok()?;
        v.get("file")
            .and_then(|f| f.as_str())
            .map(|s| s.replace('\\', "/"))
    }

    pub fn scan_id(&self) -> Option<String> {
        self.conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'scan_id' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .ok()
            .or_else(|| {
                self.path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
            })
    }

    pub fn resolve(&self, needle: &str) -> Option<(String, String)> {
        let exact: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT id, fqn FROM nodes WHERE fqn = ?1 LIMIT 1",
                [needle],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        if exact.is_some() {
            return exact;
        }
        let like = format!("%{needle}");
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, fqn FROM nodes
                 WHERE (kind LIKE '%Function%' OR kind LIKE '%Method%')
                   AND fqn LIKE ?1
                 ORDER BY length(fqn) ASC LIMIT 8",
            )
            .ok()?;
        let rows: Vec<(String, String)> = stmt
            .query_map([like], |r| Ok((r.get(0)?, r.get(1)?)))
            .ok()?
            .filter_map(|r| r.ok())
            .collect();
        let prefer = |fqn: &str| !fqn.contains("::deno::") && !fqn.contains("/_examples/");
        if rows.len() == 1 {
            return Some(rows.into_iter().next()?);
        }
        rows.into_iter().find(|(_, fqn)| {
            prefer(fqn) && (fqn.ends_with(needle) || fqn.ends_with(&format!("::{needle}")))
        })
    }

    pub fn callers(&self, id: &str, compiler_only: bool) -> anyhow::Result<Vec<String>> {
        let sql = if compiler_only {
            "SELECT n.fqn, e.provenance FROM edges e
             JOIN nodes n ON n.id = e.from_id
             WHERE e.to_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'"
        } else {
            "SELECT n.fqn, e.provenance FROM edges e
             JOIN nodes n ON n.id = e.from_id
             WHERE e.to_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows.flatten() {
            if compiler_only && !row.1.contains("\"Compiler\"") && !row.1.contains("\"Lsp\"") {
                continue;
            }
            out.push(row.0);
        }
        Ok(out)
    }

    pub fn callers_treesitter_only(&self, id: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.fqn, e.provenance FROM edges e
             JOIN nodes n ON n.id = e.from_id
             WHERE e.to_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        Ok(rows
            .flatten()
            .filter(|(_, p)| p.contains("TreeSitter") || p.contains("Heuristic"))
            .map(|(f, _)| f)
            .collect())
    }

    pub fn callees(&self, id: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.fqn FROM edges e
             JOIN nodes n ON n.id = e.to_id
             WHERE e.from_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'",
        )?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        Ok(rows.flatten().collect())
    }

    pub fn no_callers(&self, needle: &str, treesitter_only: bool) -> Option<ClaimView> {
        let (id, fqn) = self.resolve(needle)?;
        let hits = if treesitter_only {
            self.callers_treesitter_only(&id).ok()?
        } else {
            self.callers(&id, false).ok()?
        };
        Some(ClaimView {
            verdict: if hits.is_empty() {
                "verified".into()
            } else {
                "refuted".into()
            },
            witnesses: if hits.is_empty() { vec![fqn] } else { hits },
        })
    }

    pub fn calls(&self, a: &str, b: &str) -> Option<ClaimView> {
        let (aid, _) = self.resolve(a)?;
        let (bid, _) = self.resolve(b)?;
        let hits = self.callees(&aid).ok()?;
        let found = hits.iter().any(|h| {
            self.resolve(b)
                .map(|(_, f)| f == *h || h.ends_with(b))
                .unwrap_or(false)
                || {
                    let Ok(more) = self.callees(&aid) else {
                        return false;
                    };
                    more.iter().any(|x| {
                        self.conn
                            .query_row("SELECT id FROM nodes WHERE fqn = ?1", [x.as_str()], |r| {
                                r.get::<_, String>(0)
                            })
                            .ok()
                            .as_deref()
                            == Some(bid.as_str())
                    })
                }
        });
        let found = found
            || self
                .conn
                .query_row(
                    "SELECT 1 FROM edges WHERE from_id = ?1 AND to_id = ?2 AND json_extract(kind, '$.kind') = 'Call'",
                    rusqlite::params![aid, bid],
                    |_| Ok(()),
                )
                .is_ok();
        Some(ClaimView {
            verdict: if found {
                "verified".into()
            } else {
                "refuted".into()
            },
            witnesses: hits,
        })
    }

    pub fn in_cycle(&self, needle: &str) -> Option<ClaimView> {
        let (id, fqn) = self.resolve(needle)?;
        // Bounded DFS for a cycle through `id`.
        let mut stmt = self
            .conn
            .prepare("SELECT from_id, to_id FROM edges WHERE json_extract(kind, '$.kind') = 'Call'")
            .ok()?;
        let edges: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .ok()?
            .flatten()
            .collect();
        let mut adj: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (a, b) in edges {
            adj.entry(a).or_default().push(b);
        }
        let mut found = false;
        let mut stack = vec![(id.clone(), vec![id.clone()])];
        let mut steps = 0usize;
        while let Some((node, path)) = stack.pop() {
            steps += 1;
            if steps > 8000 || path.len() > 6 {
                continue;
            }
            for nxt in adj.get(&node).into_iter().flatten() {
                if nxt == &id && path.len() >= 2 {
                    found = true;
                    break;
                }
                if !path.contains(nxt) {
                    let mut p = path.clone();
                    p.push(nxt.clone());
                    stack.push((nxt.clone(), p));
                }
            }
            if found {
                break;
            }
        }
        Some(ClaimView {
            verdict: if found {
                "verified".into()
            } else {
                "refuted".into()
            },
            witnesses: vec![fqn],
        })
    }
}

pub fn parse_calls_args(claim: &str) -> Option<(String, String)> {
    let claim = claim.trim();
    let open = claim.find('(')?;
    let close = claim.rfind(')')?;
    let name = claim[..open].trim();
    let args: Vec<&str> = claim[open + 1..close]
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    match (name, args.as_slice()) {
        ("calls", [a, b]) => Some(((*a).into(), (*b).into())),
        _ => None,
    }
}
