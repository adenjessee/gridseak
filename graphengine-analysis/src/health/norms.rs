//! Population (norms) database for percentile-based scoring.
//!
//! Schema and loader for the population SQLite database that stores per-metric
//! values from every analyzed project. Used to compute real percentile ranks
//! rather than hand-tuned score curves.

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::health_score::PopulationRow;

/// SQL to create the population table. Idempotent (IF NOT EXISTS).
pub const CREATE_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS population (
    id                    TEXT PRIMARY KEY,
    analyzed_at           TEXT NOT NULL,
    language              TEXT NOT NULL,
    node_count            INTEGER NOT NULL,
    func_count            INTEGER NOT NULL,
    cycle_ratio           REAL NOT NULL,
    avg_coupling          REAL,
    dead_ratio            REAL NOT NULL,
    hotspot_concentration REAL NOT NULL,
    max_depth             INTEGER NOT NULL,
    tangle_index          REAL NOT NULL,
    source                TEXT NOT NULL,
    repo_url              TEXT,
    stars                 INTEGER
);
"#;

/// Initialize a new population database at `path`, creating the table if needed.
pub fn init_population_db(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open population DB at {path}"))?;
    conn.execute_batch(CREATE_TABLE_SQL)
        .context("Failed to create population table")?;
    Ok(conn)
}

/// Insert a project's metrics into the population database.
#[allow(clippy::too_many_arguments)]
pub fn insert_population_row(
    conn: &Connection,
    id: &str,
    analyzed_at: &str,
    language: &str,
    node_count: usize,
    func_count: usize,
    cycle_ratio: f64,
    avg_coupling: Option<f64>,
    dead_ratio: f64,
    hotspot_concentration: f64,
    max_depth: usize,
    tangle_index: f64,
    source: &str,
    repo_url: Option<&str>,
    stars: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO population (id, analyzed_at, language, node_count, func_count,
         cycle_ratio, avg_coupling, dead_ratio, hotspot_concentration, max_depth, tangle_index,
         source, repo_url, stars)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        rusqlite::params![
            id,
            analyzed_at,
            language,
            node_count as i64,
            func_count as i64,
            cycle_ratio,
            avg_coupling,
            dead_ratio,
            hotspot_concentration,
            max_depth as i64,
            tangle_index,
            source,
            repo_url,
            stars,
        ],
    )?;
    Ok(())
}

/// Load all population rows from a norms database file.
pub fn load_population(path: &str) -> Result<Vec<(String, PopulationRow)>> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("Failed to open norms DB at {path}"))?;

    // Detect which columns exist (new columns may not be in older DBs)
    let column_names: Vec<String> = {
        let mut info = conn.prepare("PRAGMA table_info(population)")?;
        let names: Vec<String> = info
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        names
    };
    let has_extended = column_names.iter().any(|c| c == "avg_cyclomatic");

    let query = if has_extended {
        "SELECT id, cycle_ratio, avg_coupling, dead_ratio, hotspot_concentration, max_depth, tangle_index,
                avg_cyclomatic, avg_cohesion, avg_distance, temporal_coupling_score
         FROM population"
    } else {
        "SELECT id, cycle_ratio, avg_coupling, dead_ratio, hotspot_concentration, max_depth, tangle_index
         FROM population"
    };

    let mut stmt = conn.prepare(query)?;

    let rows = stmt.query_map([], move |row| {
        let id: String = row.get(0)?;
        let cycle_ratio: f64 = row.get(1)?;
        let avg_coupling: Option<f64> = row.get(2)?;
        let dead_ratio: f64 = row.get(3)?;
        let hotspot_concentration: f64 = row.get(4)?;
        let max_depth: i64 = row.get(5)?;
        let tangle_index: f64 = row.get(6)?;

        let (avg_cyclomatic, avg_cohesion, avg_distance, temporal_coupling_score) = if has_extended
        {
            (row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?)
        } else {
            (None, None, None, None)
        };

        Ok((
            id,
            PopulationRow {
                cycle_ratio,
                avg_coupling,
                dead_ratio,
                hotspot_concentration,
                max_depth: max_depth as usize,
                tangle_index,
                avg_cyclomatic,
                avg_cohesion,
                avg_distance,
                temporal_coupling_score,
            },
        ))
    })?;

    let mut population = Vec::new();
    for row in rows {
        population.push(row?);
    }

    Ok(population)
}

/// Derive the population version string from the most recent `analyzed_at` timestamp.
pub fn population_version(population: &[(String, PopulationRow)]) -> String {
    if population.is_empty() {
        return "empty".to_string();
    }
    format!("N={}", population.len())
}

/// Seed the population database from a health report JSON file.
/// Extracts the Layer A metrics and inserts them as a population row.
pub fn seed_from_health_report(
    conn: &Connection,
    report_json: &str,
    repo_id: &str,
    language: &str,
    source: &str,
    repo_url: Option<&str>,
    stars: Option<i64>,
) -> Result<()> {
    let report: serde_json::Value =
        serde_json::from_str(report_json).context("Failed to parse health report JSON")?;

    let metrics = report
        .get("metrics")
        .context("No metrics block in report")?;
    let summary = report
        .get("summary")
        .context("No summary block in report")?;

    let cycle_ratio = metrics
        .pointer("/cycles/ratio")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let avg_coupling = metrics
        .pointer("/coupling/avg_coupling")
        .and_then(|v| v.as_f64());
    let dead_ratio = metrics
        .pointer("/dead_code/ratio")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let hotspot_concentration = metrics
        .pointer("/hotspot_concentration/ratio")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let max_depth = metrics
        .pointer("/depth/max_call_depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let tangle_index = metrics
        .pointer("/tangle_index/ratio")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let node_count = summary
        .get("total_nodes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let func_count = summary
        .get("total_functions")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let analyzed_at = report
        .get("generated_at")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    insert_population_row(
        conn,
        repo_id,
        analyzed_at,
        language,
        node_count,
        func_count,
        cycle_ratio,
        avg_coupling,
        dead_ratio,
        hotspot_concentration,
        max_depth,
        tangle_index,
        source,
        repo_url,
        stars,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_load_population() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_pop.sqlite");
        let db_str = db_path.to_string_lossy().to_string();

        let conn = init_population_db(&db_str).unwrap();

        insert_population_row(
            &conn,
            "test-repo-1",
            "2026-02-25T00:00:00Z",
            "typescript",
            1000,
            300,
            0.005,
            Some(0.65),
            0.10,
            0.45,
            12,
            0.001,
            "calibration",
            Some("https://github.com/test/repo1"),
            Some(5000),
        )
        .unwrap();

        insert_population_row(
            &conn,
            "test-repo-2",
            "2026-02-25T00:00:00Z",
            "rust",
            500,
            100,
            0.0,
            Some(0.30),
            0.05,
            0.20,
            8,
            0.0,
            "calibration",
            None,
            None,
        )
        .unwrap();

        drop(conn);

        let pop = load_population(&db_str).unwrap();
        assert_eq!(pop.len(), 2);
    }

    #[test]
    fn seed_from_report_json() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_seed.sqlite");
        let db_str = db_path.to_string_lossy().to_string();

        let conn = init_population_db(&db_str).unwrap();

        let report = serde_json::json!({
            "generated_at": "2026-02-25T12:00:00Z",
            "metrics": {
                "cycles": { "count": 5, "total": 1000, "ratio": 0.005, "description": "" },
                "coupling": { "modules_measured": 10, "modules_above_070": 2, "modules_above_050": 4, "avg_coupling": 0.55, "description": "" },
                "hotspot_concentration": { "count": 10, "total": 200, "ratio": 0.35, "description": "" },
                "dead_code": { "count": 20, "total": 200, "ratio": 0.10, "description": "" },
                "depth": { "max_call_depth": 15, "description": "" },
                "tangle_index": { "count": 3, "total": 5000, "ratio": 0.0006, "description": "" }
            },
            "summary": { "total_nodes": 1000, "total_functions": 200 }
        });

        seed_from_health_report(
            &conn,
            &report.to_string(),
            "hono",
            "typescript",
            "calibration",
            Some("https://github.com/honojs/hono"),
            Some(20000),
        )
        .unwrap();

        drop(conn);

        let pop = load_population(&db_str).unwrap();
        assert_eq!(pop.len(), 1);
        assert_eq!(pop[0].0, "hono");
        assert!((pop[0].1.cycle_ratio - 0.005).abs() < 1e-6);
    }
}
