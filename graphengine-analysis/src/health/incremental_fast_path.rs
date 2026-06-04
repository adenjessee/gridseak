//! S2 incremental analysis fast path (v1 zero-delta only; S2-β segmented sync).

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

use graphengine_parsing::infrastructure::storage::parse_meta_store::{
    compute_graph_fingerprint, read_incremental_scan_stats, upsert_parse_meta,
    IncrementalScanStats, INCREMENTAL_SCAN_STATS_KEY,
};

use super::report::{HealthReport, CAVEAT_INCREMENTAL_ANALYSIS_REUSED_V1};

pub const ANALYSIS_REPORT_CACHE_KEY: &str = "analysis_report_cache";
pub const GRAPH_FINGERPRINT_KEY: &str = "graph_fingerprint";
pub const ANALYSIS_STATUS_KEY: &str = "analysis_status";

pub struct FastPathOutcome {
    pub report: HealthReport,
    pub tier: FastPathTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastPathTier {
    ZeroDelta,
}

/// Attempt to produce a report from the persistent analysis cache (zero-delta only).
pub fn try_fast_path(
    conn: &Connection,
    db_path: &str,
    incremental_enabled: bool,
) -> Result<Option<FastPathOutcome>> {
    if !incremental_enabled {
        return Ok(None);
    }

    let Some(stats) = read_incremental_scan_stats(conn)? else {
        return Ok(None);
    };
    if stats.plan_disabled {
        return Ok(None);
    }

    let cached_report = read_cached_report(conn)?;
    if cached_report.is_none() {
        return Ok(None);
    }
    let cached_report = cached_report.unwrap();

    if stats.is_zero_delta() {
        return Ok(Some(FastPathOutcome {
            report: refresh_cached_report(cached_report, db_path, false),
            tier: FastPathTier::ZeroDelta,
        }));
    }

    Ok(None)
}

/// Persist a full analysis report for future warm rescans.
pub fn write_analysis_cache(
    conn: &Connection,
    report: &HealthReport,
    stats: &IncrementalScanStats,
) -> Result<()> {
    let fingerprint = compute_graph_fingerprint(conn, stats)?;
    let json = serde_json::to_string(report).context("serialize analysis_report_cache")?;
    upsert_parse_meta(conn, ANALYSIS_REPORT_CACHE_KEY, &json)?;
    upsert_parse_meta(conn, GRAPH_FINGERPRINT_KEY, &fingerprint)?;
    // Keep stats key fresh — orchestrator writes it, but rewriting here
    // guarantees analyzer + parser agree on the fingerprint inputs.
    let stats_json = serde_json::to_string(stats).context("serialize incremental_scan_stats")?;
    upsert_parse_meta(conn, INCREMENTAL_SCAN_STATS_KEY, &stats_json)?;
    Ok(())
}

fn read_cached_report(conn: &Connection) -> Result<Option<HealthReport>> {
    let Some(raw) = read_parse_meta_value(conn, ANALYSIS_REPORT_CACHE_KEY)? else {
        return Ok(None);
    };
    let report: HealthReport =
        serde_json::from_str(&raw).context("deserialize analysis_report_cache")?;
    Ok(Some(report))
}

fn read_parse_meta_value(conn: &Connection, key: &str) -> Result<Option<String>> {
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='parse_meta'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        return Ok(None);
    }
    conn.query_row(
        "SELECT value FROM parse_meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .with_context(|| format!("read parse_meta `{key}`"))
}

fn refresh_cached_report(
    mut report: HealthReport,
    db_path: &str,
    add_caveat: bool,
) -> HealthReport {
    report.db_path = db_path.to_string();
    report.generated_at = Utc::now().to_rfc3339();
    report.analysis_duration_ms = 0;
    if add_caveat
        && !report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_INCREMENTAL_ANALYSIS_REUSED_V1)
    {
        report
            .integrity_status
            .schema_caveats
            .push(CAVEAT_INCREMENTAL_ANALYSIS_REUSED_V1.to_string());
    }
    report
}
