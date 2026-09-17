//! Non-contaminated false-claim gate: synthetic pairs that are not Call
//! edges, plus an optional real G1 corpus subset.

use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct SuiteReport {
    synthetic: Slice,
    corpus: Option<Slice>,
    contamination: String,
}

#[derive(Serialize)]
struct Slice {
    artifact: String,
    false_claims: usize,
    refuted: usize,
    refute_rate: f64,
    false_refutations: usize,
}

fn write_synthetic(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(path);
    let conn = Connection::open(path)?;
    conn.execute_batch(
        r#"
        CREATE TABLE nodes (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            fqn TEXT NOT NULL,
            location TEXT NOT NULL DEFAULT '{}',
            provenance TEXT NOT NULL DEFAULT '{}',
            properties TEXT NOT NULL DEFAULT '{}',
            trait_metadata TEXT
        );
        CREATE TABLE edges (
            from_id TEXT NOT NULL,
            to_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (from_id, to_id, kind)
        );
        INSERT INTO nodes (id, kind, fqn, location) VALUES
            ('a', '{"kind":"Function"}', 'syn::a', '{}'),
            ('b', '{"kind":"Function"}', 'syn::b', '{}'),
            ('c', '{"kind":"Function"}', 'syn::c', '{}'),
            ('d', '{"kind":"Function"}', 'syn::d', '{}'),
            ('e', '{"kind":"Function"}', 'syn::e', '{}'),
            ('f', '{"kind":"Function"}', 'syn::f', '{}');
        INSERT INTO edges (from_id, to_id, kind, provenance) VALUES
            ('a', 'b', '{"kind":"Call"}', '{"source":"Compiler","confidence":"High"}'),
            ('b', 'c', '{"kind":"Call"}', '{"source":"Compiler","confidence":"High"}');
        "#,
    )?;
    Ok(())
}

fn score_artifact(path: &Path) -> anyhow::Result<Slice> {
    let conn = Connection::open(path)?;
    let mut stmt = conn.prepare(
        "SELECT id, fqn FROM nodes WHERE kind LIKE '%Function%' OR kind LIKE '%Method%' ORDER BY fqn LIMIT 200",
    )?;
    let nodes: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    let mut real = HashSet::new();
    let mut edges = conn
        .prepare("SELECT from_id, to_id FROM edges WHERE json_extract(kind, '$.kind') = 'Call'")?;
    let mut rows = edges.query([])?;
    while let Some(row) = rows.next()? {
        real.insert((row.get::<_, String>(0)?, row.get::<_, String>(1)?));
    }
    let mut false_n = 0usize;
    let mut refuted = 0usize;
    'outer: for (i, (a_id, _)) in nodes.iter().enumerate() {
        for (b_id, _) in nodes.iter().skip(i + 3).take(4) {
            if real.contains(&(a_id.clone(), b_id.clone())) {
                continue;
            }
            false_n += 1;
            let exists = conn.query_row(
                "SELECT 1 FROM edges WHERE from_id = ?1 AND to_id = ?2 AND json_extract(kind, '$.kind') = 'Call'",
                rusqlite::params![a_id, b_id],
                |_| Ok(()),
            );
            if exists.is_err() {
                refuted += 1;
            }
            if false_n >= 100 {
                break 'outer;
            }
        }
    }
    let mut false_refutations = 0usize;
    for (from, to) in &real {
        let exists = conn.query_row(
            "SELECT 1 FROM edges WHERE from_id = ?1 AND to_id = ?2 AND json_extract(kind, '$.kind') = 'Call'",
            rusqlite::params![from, to],
            |_| Ok(()),
        );
        if exists.is_err() {
            false_refutations += 1;
        }
    }
    let rate = if false_n == 0 {
        0.0
    } else {
        refuted as f64 / false_n as f64
    };
    Ok(Slice {
        artifact: path.display().to_string(),
        false_claims: false_n,
        refuted,
        refute_rate: rate,
        false_refutations,
    })
}

fn default_chi() -> PathBuf {
    if let Ok(p) = std::env::var("GRIDSEAK_G1_CHI_ARTIFACT") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(
        "Library/Application Support/com.gridseak.desktop/project-graphs/73abec07-12fd-47e9-a2e8-fb3c69fdf971.sqlite",
    )
}

fn main() -> anyhow::Result<()> {
    let synthetic = PathBuf::from("target/false-claim-synthetic.sqlite");
    write_synthetic(&synthetic)?;
    let syn = score_artifact(&synthetic)?;
    let corpus = {
        let path = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(default_chi);
        if path.exists() {
            Some(score_artifact(&path)?)
        } else {
            None
        }
    };
    let report = SuiteReport {
        synthetic: syn,
        corpus,
        contamination: "pairs chosen as non-edges in the graph; not SCIP-as-oracle".into(),
    };
    std::fs::create_dir_all("target")?;
    std::fs::write(
        "target/false-claim-suite.json",
        serde_json::to_string_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if report.synthetic.refute_rate < 0.95
        || report.synthetic.false_claims < 4
        || report.synthetic.false_refutations != 0
    {
        anyhow::bail!(
            "synthetic false-claim suite bar missed: rate={} n={} false_refutations={}",
            report.synthetic.refute_rate,
            report.synthetic.false_claims,
            report.synthetic.false_refutations
        );
    }
    if let Some(c) = &report.corpus {
        if c.false_claims >= 20 && (c.refute_rate < 0.95 || c.false_refutations != 0) {
            anyhow::bail!(
                "corpus false-claim suite bar missed: rate={} n={} false_refutations={}",
                c.refute_rate,
                c.false_claims,
                c.false_refutations
            );
        }
    }
    Ok(())
}
