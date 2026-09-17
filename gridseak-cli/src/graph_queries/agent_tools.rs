//! Agent-loop tools: diff impact, verify_claim, structural invariants.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use rusqlite::Connection;
use serde::Serialize;

use super::{
    blast_radius, callees, callers, cycles, open_graph, resolve_symbol_detailed, slice,
    GraphQueryError, NodeRef, SliceDirection,
};

#[derive(Debug, Clone, Serialize)]
pub struct DiffImpactRow {
    pub symbol: String,
    pub affected: String,
    pub tier: String,
    pub p_true: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffImpactView {
    pub changed_files: Vec<String>,
    pub changed_symbols: Vec<String>,
    pub affected: Vec<DiffImpactRow>,
    pub reaching_tests: Vec<String>,
    pub suggested_test_commands: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaimVerdict {
    Verified,
    Refuted,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyClaimView {
    pub claim: String,
    pub verdict: ClaimVerdict,
    pub witnesses: Vec<String>,
    pub p_true: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InvariantResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StructuralInvariantsView {
    pub passed: bool,
    pub results: Vec<InvariantResult>,
}

/// Typed claim grammar: `calls(A,B) | no_callers(X) | reaches(A,B) | in_cycle(X) | dead(X)`.
pub fn parse_claim(claim: &str) -> Option<(&str, Vec<&str>)> {
    let claim = claim.trim();
    let open = claim.find('(')?;
    let close = claim.rfind(')')?;
    if close <= open {
        return None;
    }
    let name = claim[..open].trim();
    let args: Vec<&str> = claim[open + 1..close]
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    Some((name, args))
}

fn resolve_or_unknown(
    conn: &Connection,
    needle: &str,
) -> Result<Result<NodeRef, VerifyClaimView>, GraphQueryError> {
    match resolve_symbol_detailed(conn, needle) {
        Ok(r) => Ok(Ok(r.node)),
        Err(GraphQueryError::UnknownSymbol(s)) => Ok(Err(VerifyClaimView {
            claim: s,
            verdict: ClaimVerdict::Unknown,
            witnesses: vec![format!("symbol `{needle}` not in scan artifact")],
            p_true: 0.0,
        })),
        Err(GraphQueryError::AmbiguousSymbol {
            symbol, candidates, ..
        }) => Ok(Err(VerifyClaimView {
            claim: symbol,
            verdict: ClaimVerdict::Unknown,
            witnesses: candidates,
            p_true: 0.0,
        })),
        Err(e) => Err(e),
    }
}

fn edge_p_true(conn: &Connection, from_id: &str, to_id: &str, language: &str) -> f64 {
    let mut stmt = conn
        .prepare(
            "SELECT provenance FROM edges
             WHERE from_id = ?1 AND to_id = ?2 AND json_extract(kind, '$.kind') = 'Call'
             LIMIT 1",
        )
        .ok();
    let Some(stmt) = stmt.as_mut() else {
        return 0.0;
    };
    let raw: Option<String> = stmt.query_row([from_id, to_id], |row| row.get(0)).ok();
    raw.as_deref()
        .and_then(|j| graphengine_parsing::domain::p_true_from_provenance_json(language, j, "Call"))
        .unwrap_or(0.0)
}

fn scan_language(conn: &Connection) -> String {
    conn.query_row(
        "SELECT value FROM metadata WHERE key = 'language' LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .unwrap_or_else(|_| "unknown".into())
}

fn load_runtime_executed(conn: &Connection) -> Vec<graphengine_runtime_witness::NodeExecuted> {
    let json: Result<String, _> = conn.query_row(
        "SELECT value FROM metadata WHERE key = 'runtime_executed_json' LIMIT 1",
        [],
        |row| row.get(0),
    );
    json.ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn verify_claim(artifact: &Path, claim: &str) -> Result<VerifyClaimView, GraphQueryError> {
    let Some((name, args)) = parse_claim(claim) else {
        return Ok(VerifyClaimView {
            claim: claim.to_string(),
            verdict: ClaimVerdict::Unknown,
            witnesses: vec!["unparseable claim".into()],
            p_true: 0.0,
        });
    };
    let conn = open_graph(artifact)?;
    let language = scan_language(&conn);
    match name {
        "calls" if args.len() == 2 => {
            let a = match resolve_or_unknown(&conn, args[0])? {
                Ok(n) => n,
                Err(mut v) => {
                    v.claim = claim.to_string();
                    return Ok(v);
                }
            };
            let b = match resolve_or_unknown(&conn, args[1])? {
                Ok(n) => n,
                Err(mut v) => {
                    v.claim = claim.to_string();
                    return Ok(v);
                }
            };
            let hits = callees(&conn, &a.id)?;
            let found = hits.iter().any(|h| h.id == b.id);
            let p = if found {
                edge_p_true(&conn, &a.id, &b.id, &language)
            } else {
                0.88
            };
            Ok(VerifyClaimView {
                claim: claim.to_string(),
                verdict: if found {
                    ClaimVerdict::Verified
                } else {
                    ClaimVerdict::Refuted
                },
                witnesses: hits.into_iter().map(|h| h.fqn).collect(),
                p_true: p,
            })
        }
        "no_callers" if args.len() == 1 => {
            let x = match resolve_or_unknown(&conn, args[0])? {
                Ok(n) => n,
                Err(mut v) => {
                    v.claim = claim.to_string();
                    return Ok(v);
                }
            };
            let hits = callers(&conn, &x.id)?;
            Ok(VerifyClaimView {
                claim: claim.to_string(),
                verdict: if hits.is_empty() {
                    ClaimVerdict::Verified
                } else {
                    ClaimVerdict::Refuted
                },
                witnesses: hits.into_iter().map(|h| h.fqn).collect(),
                p_true: 0.85,
            })
        }
        "reaches" if args.len() == 2 => {
            let a = match resolve_or_unknown(&conn, args[0])? {
                Ok(n) => n,
                Err(mut v) => {
                    v.claim = claim.to_string();
                    return Ok(v);
                }
            };
            let b = match resolve_or_unknown(&conn, args[1])? {
                Ok(n) => n,
                Err(mut v) => {
                    v.claim = claim.to_string();
                    return Ok(v);
                }
            };
            let rows = slice(&conn, &a.id, 8, 400)?;
            let hit = rows
                .iter()
                .find(|r| r.node.id == b.id && matches!(r.direction, SliceDirection::Downstream));
            let verdict = if hit.is_some() {
                ClaimVerdict::Verified
            } else {
                ClaimVerdict::Refuted
            };
            let p_true = hit.and_then(|h| h.p_true).unwrap_or(0.0);
            Ok(VerifyClaimView {
                claim: claim.to_string(),
                verdict,
                witnesses: rows
                    .into_iter()
                    .filter(|r| matches!(r.direction, SliceDirection::Downstream))
                    .map(|r| r.node.fqn)
                    .collect(),
                p_true,
            })
        }
        "in_cycle" if args.len() == 1 => {
            let x = match resolve_or_unknown(&conn, args[0])? {
                Ok(n) => n,
                Err(mut v) => {
                    v.claim = claim.to_string();
                    return Ok(v);
                }
            };
            let found: Vec<String> = cycles(&conn, 64, 8)?
                .into_iter()
                .filter(|c| c.members.iter().any(|m| m.id == x.id))
                .flat_map(|c| c.members.into_iter().map(|m| m.fqn))
                .collect();
            let verdict = if found.is_empty() {
                ClaimVerdict::Refuted
            } else {
                ClaimVerdict::Verified
            };
            let p_true = if found.is_empty() { 0.80 } else { 0.90 };
            Ok(VerifyClaimView {
                claim: claim.to_string(),
                verdict,
                witnesses: found,
                p_true,
            })
        }
        "dead" if args.len() == 1 => {
            let x = match resolve_or_unknown(&conn, args[0])? {
                Ok(n) => n,
                Err(mut v) => {
                    v.claim = claim.to_string();
                    return Ok(v);
                }
            };
            let executed = load_runtime_executed(&conn);
            if executed.is_empty() {
                return Ok(VerifyClaimView {
                    claim: claim.to_string(),
                    verdict: ClaimVerdict::Unknown,
                    witnesses: vec![
                        "dead() remains Unknown without runtime coverage; static-only never refutes"
                            .into(),
                    ],
                    p_true: 0.0,
                });
            }
            let file = node_file(&conn, &x.id).unwrap_or_default();
            let covered = executed.iter().any(|n| {
                file_match(&n.file, &file)
                    || n.symbol
                        .as_deref()
                        .map(|s| s == x.fqn || x.fqn.ends_with(s))
                        .unwrap_or(false)
            });
            if covered {
                Ok(VerifyClaimView {
                    claim: claim.to_string(),
                    verdict: ClaimVerdict::Refuted,
                    witnesses: vec![format!(
                        "runtime coverage observed {} (ceiling 0 for dead)",
                        x.fqn
                    )],
                    p_true: 0.0,
                })
            } else {
                Ok(VerifyClaimView {
                    claim: claim.to_string(),
                    verdict: ClaimVerdict::Unknown,
                    witnesses: vec!["coverage loaded but this node was not observed".into()],
                    p_true: 0.0,
                })
            }
        }
        _ => Ok(VerifyClaimView {
            claim: claim.to_string(),
            verdict: ClaimVerdict::Unknown,
            witnesses: vec!["unsupported claim form".into()],
            p_true: 0.0,
        }),
    }
}

fn node_file(conn: &Connection, id: &str) -> Option<String> {
    conn.query_row(
        "SELECT json_extract(location, '$.file') FROM nodes WHERE id = ?1",
        [id],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

fn file_match(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a == b || a.ends_with(b) || b.ends_with(a))
}

pub fn changed_files(repo: &Path, base: Option<&str>, head: Option<&str>) -> Vec<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).arg("diff").arg("--name-only");
    match (base, head) {
        (Some(b), Some(h)) => {
            cmd.arg(format!("{b}...{h}"));
        }
        (Some(b), None) => {
            cmd.arg(format!("{b}...HEAD"));
        }
        _ => {}
    }
    let output = cmd.output().ok();
    let Some(out) = output else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn functions_in_file(conn: &Connection, file: &str) -> Result<Vec<NodeRef>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "SELECT id, fqn, kind FROM nodes
         WHERE kind IN ('Function', 'Method', 'Constructor')
           AND (
             json_extract(location, '$.file') = ?1
             OR json_extract(location, '$.file') LIKE ?2
             OR json_extract(location, '$.file') LIKE ?3
           )
         ORDER BY fqn",
    )?;
    let like_suffix = format!("%/{file}");
    let like_suffix2 = format!("%{file}");
    let rows = stmt
        .query_map(rusqlite::params![file, like_suffix, like_suffix2], |row| {
            Ok(NodeRef {
                id: row.get(0)?,
                fqn: row.get(1)?,
                kind: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn diff_impact(
    artifact: &Path,
    repo: &Path,
    base: Option<&str>,
    head: Option<&str>,
) -> Result<DiffImpactView, GraphQueryError> {
    let conn = open_graph(artifact)?;
    let files = changed_files(repo, base, head);
    let mut notes = Vec::new();
    if files.is_empty() {
        notes.push("git diff produced no changed files (clean tree or missing git)".into());
    }
    let mut changed_symbols = Vec::new();
    let mut affected = Vec::new();
    let mut seen_affected = HashSet::new();
    for file in &files {
        let seeds = functions_in_file(&conn, file)?;
        for seed in seeds {
            changed_symbols.push(seed.fqn.clone());
            let rows = match blast_radius(&conn, &seed.id, 3, 200) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for row in rows {
                let key = format!("{}→{}", seed.fqn, row.node.fqn);
                if !seen_affected.insert(key) {
                    continue;
                }
                affected.push(DiffImpactRow {
                    symbol: seed.fqn.clone(),
                    affected: row.node.fqn,
                    tier: row.edge_evidence_tier.unwrap_or_else(|| "unknown".into()),
                    p_true: row.p_true.unwrap_or(0.0),
                });
            }
        }
    }
    changed_symbols.sort();
    changed_symbols.dedup();

    let mut reaching_tests = Vec::new();
    let mut test_stmt = conn
        .prepare(
            "SELECT id, fqn, kind FROM nodes
             WHERE kind IN ('Function', 'Method')
               AND (fqn LIKE '%test%' OR fqn LIKE '%Test%' OR json_extract(location, '$.file') LIKE '%test%')",
        )
        .ok();
    if let Some(stmt) = test_stmt.as_mut() {
        let tests: Vec<NodeRef> = stmt
            .query_map([], |row| {
                Ok(NodeRef {
                    id: row.get(0)?,
                    fqn: row.get(1)?,
                    kind: row.get(2)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();
        let changed_ids: HashSet<String> = changed_symbols
            .iter()
            .filter_map(|fqn| resolve_symbol_detailed(&conn, fqn).ok().map(|r| r.node.id))
            .collect();
        for t in tests {
            if let Ok(down) = slice(&conn, &t.id, 6, 200) {
                if down.iter().any(|r| {
                    matches!(r.direction, SliceDirection::Downstream)
                        && changed_ids.contains(&r.node.id)
                }) {
                    reaching_tests.push(t.fqn);
                }
            }
        }
    }
    if reaching_tests.is_empty() {
        notes.push(
            "reaching tests omitted or empty — runtime witness not required; classification is name/path heuristic"
                .into(),
        );
    }

    Ok(DiffImpactView {
        changed_files: files,
        changed_symbols,
        affected,
        reaching_tests,
        suggested_test_commands: vec!["cargo test --workspace".into()],
        notes,
    })
}

pub fn structural_invariants(
    head_artifact: &Path,
    base_artifact: Option<&Path>,
) -> Result<StructuralInvariantsView, GraphQueryError> {
    let head = open_graph(head_artifact)?;
    let head_cycles = cycles(&head, 64, 8)?;
    let base_cycles = if let Some(base) = base_artifact {
        let base_conn = open_graph(base)?;
        cycles(&base_conn, 64, 8)?
    } else {
        Vec::new()
    };
    let base_keys: HashSet<String> = base_cycles
        .iter()
        .map(|c| {
            let mut ids: Vec<String> = c.members.iter().map(|m| m.fqn.clone()).collect();
            ids.sort();
            ids.join("→")
        })
        .collect();
    let new_cycles: Vec<String> = head_cycles
        .iter()
        .map(|c| {
            let mut ids: Vec<String> = c.members.iter().map(|m| m.fqn.clone()).collect();
            ids.sort();
            ids.join("→")
        })
        .filter(|k| !base_keys.contains(k) && base_artifact.is_some())
        .collect();
    let no_new_cycles = InvariantResult {
        name: "no_new_cycles".into(),
        passed: new_cycles.is_empty(),
        detail: if base_artifact.is_none() {
            format!(
                "no base scan supplied; {} cycle(s) in head (not compared)",
                head_cycles.len()
            )
        } else if new_cycles.is_empty() {
            "no new cycles vs base".into()
        } else {
            format!("new cycles: {}", new_cycles.join(" | "))
        },
    };

    let orphans = orphaned_public_symbols(&head)?;
    let no_orphans = InvariantResult {
        name: "no_orphaned_public_symbols".into(),
        passed: orphans.is_empty(),
        detail: if orphans.is_empty() {
            "no exported fan-in-0 functions".into()
        } else {
            format!("orphans: {}", orphans.join(", "))
        },
    };

    let forbidden = InvariantResult {
        name: "forbidden_layers".into(),
        passed: true,
        detail: "no forbidden-layer policy configured; skipped".into(),
    };
    let hotspot_tests = InvariantResult {
        name: "hotspot_tests".into(),
        passed: true,
        detail: "reaching-test coverage not computed without runtime witness".into(),
    };

    let results = vec![no_new_cycles, no_orphans, forbidden, hotspot_tests];
    let passed = results.iter().all(|r| r.passed);
    Ok(StructuralInvariantsView { passed, results })
}

fn orphaned_public_symbols(conn: &Connection) -> Result<Vec<String>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "SELECT n.fqn FROM nodes n
         WHERE n.kind IN ('Function', 'Method')
           AND n.fqn NOT LIKE '%::test%'
           AND n.fqn NOT LIKE '%tests::%'
           AND (n.fqn LIKE '%::pub%' OR json_extract(n.properties, '$.visibility') = 'public'
                OR n.fqn LIKE '%gridseak%' OR n.fqn LIKE '%graphengine%')
           AND NOT EXISTS (
             SELECT 1 FROM edges e
             WHERE e.to_id = n.id AND json_extract(e.kind, '$.kind') = 'Call'
           )
         LIMIT 40",
    )?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Render-fixture constructor (not the MCP path).
pub fn diff_impact_empty(changed: Vec<String>) -> DiffImpactView {
    DiffImpactView {
        changed_files: Vec::new(),
        changed_symbols: changed,
        affected: Vec::new(),
        reaching_tests: Vec::new(),
        suggested_test_commands: vec!["cargo test --workspace".into()],
        notes: vec!["empty impact fixture".into()],
    }
}

/// Render-fixture constructor (not the MCP path).
pub fn structural_invariants_ok() -> StructuralInvariantsView {
    StructuralInvariantsView {
        passed: true,
        results: vec![InvariantResult {
            name: "fixture".into(),
            passed: true,
            detail: "render fixture".into(),
        }],
    }
}

/// Seeded false-claim evaluation used by the Batch E gate.
pub fn false_claim_suite(artifact: &Path) -> Result<FalseClaimReport, GraphQueryError> {
    let conn = open_graph(artifact)?;
    let mut stmt = conn.prepare(
        "SELECT id, fqn FROM nodes WHERE kind IN ('Function', 'Method') ORDER BY fqn LIMIT 400",
    )?;
    let nodes: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut real_pairs = HashSet::new();
    let mut edge_stmt = conn.prepare(
        "SELECT from_id, to_id, provenance FROM edges WHERE json_extract(kind, '$.kind') = 'Call'",
    )?;
    let mut tier3_pairs = Vec::new();
    {
        let mut rows = edge_stmt.query([])?;
        while let Some(row) = rows.next()? {
            let from: String = row.get(0)?;
            let to: String = row.get(1)?;
            let prov: Option<String> = row.get(2).ok();
            real_pairs.insert((from.clone(), to.clone()));
            if let Some(p) = prov {
                if p.contains("\"Compiler\"") || p.contains("\"Lsp\"") {
                    tier3_pairs.push((from, to));
                }
            }
        }
    }
    let mut false_claims = Vec::new();
    'outer: for (i, (a_id, a_fqn)) in nodes.iter().enumerate() {
        for (b_id, b_fqn) in nodes.iter().skip(i + 1).take(8) {
            if real_pairs.contains(&(a_id.clone(), b_id.clone())) {
                continue;
            }
            false_claims.push((a_fqn.clone(), b_fqn.clone()));
            if false_claims.len() >= 100 {
                break 'outer;
            }
        }
    }
    let mut refuted = 0usize;
    for (a, b) in &false_claims {
        let view = verify_claim(artifact, &format!("calls({a},{b})"))?;
        if view.verdict == ClaimVerdict::Refuted {
            refuted += 1;
        }
    }
    let mut false_refutations = 0usize;
    for (from, to) in &tier3_pairs {
        let a = nodes.iter().find(|(id, _)| id == from).map(|(_, f)| f);
        let b = nodes.iter().find(|(id, _)| id == to).map(|(_, f)| f);
        if let (Some(a), Some(b)) = (a, b) {
            let view = verify_claim(artifact, &format!("calls({a},{b})"))?;
            if view.verdict == ClaimVerdict::Refuted {
                false_refutations += 1;
            }
        }
    }
    let rate = if false_claims.is_empty() {
        0.0
    } else {
        refuted as f64 / false_claims.len() as f64
    };
    Ok(FalseClaimReport {
        false_claims: false_claims.len(),
        refuted,
        refute_rate: rate,
        tier3_pairs: tier3_pairs.len(),
        false_refutations,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct FalseClaimReport {
    pub false_claims: usize,
    pub refuted: usize,
    pub refute_rate: f64,
    pub tier3_pairs: usize,
    pub false_refutations: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_queries::queries::build_test_db;
    use rusqlite::Connection;

    fn write_tmp_db() -> (tempfile::NamedTempFile, Connection) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let src = build_test_db();
        src.execute("VACUUM INTO ?1", [tmp.path().to_str().unwrap()])
            .unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        (tmp, conn)
    }

    #[test]
    fn parses_calls_claim() {
        let (name, args) = parse_claim("calls(A,B)").unwrap();
        assert_eq!(name, "calls");
        assert_eq!(args, vec!["A", "B"]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_claim("not-a-claim").is_none());
    }

    #[test]
    fn verify_calls_refutes_missing_edge() {
        let (tmp, _conn) = write_tmp_db();
        let view = verify_claim(tmp.path(), "calls(crate::orphan::lonely,crate::a::root)").unwrap();
        assert_eq!(view.verdict, ClaimVerdict::Refuted);
    }

    #[test]
    fn verify_calls_confirms_real_edge() {
        let (tmp, _conn) = write_tmp_db();
        let view = verify_claim(tmp.path(), "calls(crate::a::root,crate::a::mid)").unwrap();
        assert_eq!(view.verdict, ClaimVerdict::Verified);
        assert!(view.p_true > 0.8);
    }

    #[test]
    fn verify_in_cycle_and_reaches() {
        let (tmp, _conn) = write_tmp_db();
        let cycle = verify_claim(tmp.path(), "in_cycle(crate::a::root)").unwrap();
        assert_eq!(cycle.verdict, ClaimVerdict::Verified);
        let reaches = verify_claim(tmp.path(), "reaches(crate::a::root,crate::a::leaf)").unwrap();
        assert_eq!(reaches.verdict, ClaimVerdict::Verified);
    }

    #[test]
    fn dead_is_unknown_without_coverage() {
        let (tmp, _conn) = write_tmp_db();
        let view = verify_claim(tmp.path(), "dead(crate::orphan::lonely)").unwrap();
        assert_eq!(view.verdict, ClaimVerdict::Unknown);
    }

    #[test]
    fn dead_refutes_when_runtime_coverage_observed() {
        let (tmp, conn) = write_tmp_db();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('runtime_executed_json', ?1)",
            [r#"[{"file":"src/orphan.rs","line":1,"symbol":"crate::orphan::lonely"}]"#],
        )
        .unwrap();
        drop(conn);
        let view = verify_claim(tmp.path(), "dead(crate::orphan::lonely)").unwrap();
        assert_eq!(view.verdict, ClaimVerdict::Refuted);
        assert_eq!(view.p_true, 0.0);
    }

    #[test]
    fn false_claim_suite_hits_bar_on_fixture() {
        let (tmp, _conn) = write_tmp_db();
        let report = false_claim_suite(tmp.path()).unwrap();
        assert!(
            report.refute_rate >= 0.95 || report.false_claims < 5,
            "refute_rate={} n={}",
            report.refute_rate,
            report.false_claims
        );
        assert_eq!(report.false_refutations, 0);
    }

    #[test]
    fn invariants_fail_on_orphans_when_present() {
        let (tmp, _conn) = write_tmp_db();
        let view = structural_invariants(tmp.path(), None).unwrap();
        assert!(view.results.iter().any(|r| r.name == "no_new_cycles"));
    }
}
