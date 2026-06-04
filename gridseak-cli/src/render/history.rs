//! View model + renderers for `gridseak scans list` and
//! `gridseak scan latest`.
//!
//! Each row carries the columns the spec calls for under
//! "History And Comparison" (CLI_SHADOW_MODE_DISTRIBUTION_SPEC.md
//! §History And Comparison):
//!
//! - scan date
//! - branch
//! - commit
//! - dirty state
//! - score
//! - critical / high counts
//! - cycles
//! - hotspots
//! - dead code
//! - coupling
//!
//! Per-row rendering is byte-stable across runs because everything
//! is built from the deterministic `MetricSnapshotDto` persisted
//! alongside each scan. We never re-derive numbers from a raw
//! `HealthReport` here — that would be slow (one file open per row)
//! and inconsistent (the snapshot is what the desktop reads, so the
//! CLI should not invent a different number for the same scan).

use std::io::{self, Write};

use gridseak_local_store::{MetricSnapshotDto, ScanRunDto};
use serde::Serialize;

use crate::render::view::ScoreBand;
use crate::render::width::Layout;

/// Pre-formatted history row. All fields are display strings; the
/// view is *only* a view.
#[derive(Debug, Clone, Serialize)]
pub struct ScanHistoryRow {
    pub scan_id: String,
    pub date: String,
    pub branch: String,
    pub commit_short: String,
    pub dirty: Option<bool>,
    pub score: Option<u32>,
    pub score_band: ScoreBand,
    pub critical: i64,
    pub high: i64,
    pub cycles: i64,
    pub hotspots: i64,
    pub dead_code: i64,
    pub coupling: Option<f64>,
    pub primary_language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanHistoryView {
    pub repo_name: String,
    pub scans: Vec<ScanHistoryRow>,
}

impl ScanHistoryRow {
    pub fn from_scan(scan: &ScanRunDto, metrics: Option<&MetricSnapshotDto>) -> Self {
        Self {
            scan_id: scan.id.clone(),
            date: shorten_date(&scan.started_at),
            branch: scan.git_branch.clone().unwrap_or_else(|| "—".into()),
            commit_short: scan
                .git_commit
                .as_deref()
                .map(short_commit)
                .unwrap_or("—")
                .to_string(),
            dirty: scan.git_dirty,
            score: metrics.and_then(|m| m.health_score.map(|s| s.round() as u32)),
            score_band: ScoreBand::from_score(
                metrics.and_then(|m| m.health_score.map(|s| s.round() as u32)),
            ),
            critical: metrics.map(|m| m.critical_count).unwrap_or(0),
            high: metrics.map(|m| m.high_count).unwrap_or(0),
            cycles: metrics.map(|m| m.cycle_count).unwrap_or(0),
            hotspots: metrics.map(|m| m.hotspot_count).unwrap_or(0),
            dead_code: metrics.map(|m| m.dead_code_count).unwrap_or(0),
            coupling: metrics.and_then(|m| m.avg_coupling),
            primary_language: scan.primary_language.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Renderers
// ---------------------------------------------------------------------------

/// Output format for history-style tables. Same shape as
/// [`crate::render::HeroFormat`] but specialised so history's
/// columns don't have to be folded into the hero schema.
#[derive(Debug, Clone)]
pub enum HistoryFormat {
    Table { layout: Layout },
    Markdown,
    Json,
    ForLlm,
}

pub fn render(
    format: &HistoryFormat,
    view: &ScanHistoryView,
    out: &mut dyn Write,
) -> io::Result<()> {
    match format {
        HistoryFormat::Table { layout } => render_table(view, *layout, out),
        HistoryFormat::Markdown => render_markdown(view, out),
        HistoryFormat::Json => render_json(view, out),
        HistoryFormat::ForLlm => render_llm(view, out),
    }
}

fn render_table(view: &ScanHistoryView, layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "Scan History — {}", view.repo_name)?;
    if view.scans.is_empty() {
        writeln!(out, "(no scans recorded yet — run `gridseak scan .`)")?;
        return Ok(());
    }
    match layout {
        Layout::Wide => render_table_wide(view, out),
        Layout::Medium => render_table_medium(view, out),
        Layout::Narrow => render_table_narrow(view, out),
        Layout::Plain => render_table_plain(view, out),
    }
}

fn render_table_wide(view: &ScanHistoryView, out: &mut dyn Write) -> io::Result<()> {
    let header = [
        "Date", "Branch", "Commit", "Score", "Critical", "High", "Cycles", "Hotspots", "Dead",
        "Coupling",
    ];
    let mut widths: Vec<usize> = header.iter().map(|c| c.len()).collect();
    let rows: Vec<Vec<String>> = view.scans.iter().map(row_cells_wide).collect();
    for row in &rows {
        for (i, c) in row.iter().enumerate() {
            widths[i] = widths[i].max(c.len());
        }
    }
    write_columns(
        out,
        &header.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        &widths,
    )?;
    for row in rows {
        write_columns(out, &row, &widths)?;
    }
    Ok(())
}

fn render_table_medium(view: &ScanHistoryView, out: &mut dyn Write) -> io::Result<()> {
    let header = [
        "Date", "Branch", "Score", "Critical", "Cycles", "Hotspots", "Dead",
    ];
    let mut widths: Vec<usize> = header.iter().map(|c| c.len()).collect();
    let rows: Vec<Vec<String>> = view.scans.iter().map(row_cells_medium).collect();
    for row in &rows {
        for (i, c) in row.iter().enumerate() {
            widths[i] = widths[i].max(c.len());
        }
    }
    write_columns(
        out,
        &header.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        &widths,
    )?;
    for row in rows {
        write_columns(out, &row, &widths)?;
    }
    Ok(())
}

fn render_table_narrow(view: &ScanHistoryView, out: &mut dyn Write) -> io::Result<()> {
    for (idx, scan) in view.scans.iter().enumerate() {
        if idx > 0 {
            writeln!(out)?;
        }
        let score = match scan.score {
            Some(s) => format!("{s} ({})", scan.score_band.label()),
            None => "—".into(),
        };
        writeln!(
            out,
            "[{date}] {branch} @ {commit}",
            date = scan.date,
            branch = scan.branch,
            commit = scan.commit_short
        )?;
        writeln!(out, "  score:    {score}")?;
        writeln!(out, "  critical: {} / high: {}", scan.critical, scan.high)?;
        writeln!(
            out,
            "  cycles: {} | hotspots: {} | dead: {} | coupling: {}",
            scan.cycles,
            scan.hotspots,
            scan.dead_code,
            scan.coupling
                .map(|c| format!("{c:.2}"))
                .unwrap_or_else(|| "—".into())
        )?;
    }
    Ok(())
}

fn render_table_plain(view: &ScanHistoryView, out: &mut dyn Write) -> io::Result<()> {
    for scan in &view.scans {
        writeln!(
            out,
            "{date} {branch} {commit} score={score} crit={crit} high={high} cycles={cyc} hot={hot} dead={dead} coupling={cpl}",
            date = scan.date,
            branch = scan.branch,
            commit = scan.commit_short,
            score = scan
                .score
                .map(|s| s.to_string())
                .unwrap_or_else(|| "—".into()),
            crit = scan.critical,
            high = scan.high,
            cyc = scan.cycles,
            hot = scan.hotspots,
            dead = scan.dead_code,
            cpl = scan
                .coupling
                .map(|c| format!("{c:.2}"))
                .unwrap_or_else(|| "—".into())
        )?;
    }
    Ok(())
}

fn row_cells_wide(row: &ScanHistoryRow) -> Vec<String> {
    vec![
        row.date.clone(),
        dirty_branch(&row.branch, row.dirty),
        row.commit_short.clone(),
        row.score
            .map(|s| s.to_string())
            .unwrap_or_else(|| "—".into()),
        row.critical.to_string(),
        row.high.to_string(),
        row.cycles.to_string(),
        row.hotspots.to_string(),
        row.dead_code.to_string(),
        row.coupling
            .map(|c| format!("{c:.2}"))
            .unwrap_or_else(|| "—".into()),
    ]
}

fn row_cells_medium(row: &ScanHistoryRow) -> Vec<String> {
    vec![
        row.date.clone(),
        dirty_branch(&row.branch, row.dirty),
        row.score
            .map(|s| s.to_string())
            .unwrap_or_else(|| "—".into()),
        row.critical.to_string(),
        row.cycles.to_string(),
        row.hotspots.to_string(),
        row.dead_code.to_string(),
    ]
}

fn write_columns(out: &mut dyn Write, cells: &[String], widths: &[usize]) -> io::Result<()> {
    let mut first = true;
    for (cell, width) in cells.iter().zip(widths.iter()) {
        if !first {
            write!(out, "  ")?;
        }
        write!(out, "{:<w$}", cell, w = *width)?;
        first = false;
    }
    writeln!(out)?;
    Ok(())
}

fn render_markdown(view: &ScanHistoryView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "# Scan History — {}", view.repo_name)?;
    writeln!(out)?;
    if view.scans.is_empty() {
        writeln!(out, "_No scans recorded yet — run `gridseak scan .`._")?;
        return Ok(());
    }
    writeln!(
        out,
        "| Date | Branch | Commit | Score | Critical | High | Cycles | Hotspots | Dead | Coupling |"
    )?;
    writeln!(
        out,
        "|------|--------|--------|------:|---------:|-----:|-------:|---------:|-----:|---------:|"
    )?;
    for row in &view.scans {
        writeln!(
            out,
            "| {date} | {branch} | `{commit}` | {score} | {crit} | {high} | {cyc} | {hot} | {dead} | {cpl} |",
            date = row.date,
            branch = dirty_branch(&row.branch, row.dirty),
            commit = row.commit_short,
            score = row
                .score
                .map(|s| s.to_string())
                .unwrap_or_else(|| "—".into()),
            crit = row.critical,
            high = row.high,
            cyc = row.cycles,
            hot = row.hotspots,
            dead = row.dead_code,
            cpl = row
                .coupling
                .map(|c| format!("{c:.2}"))
                .unwrap_or_else(|| "—".into())
        )?;
    }
    Ok(())
}

fn render_json(view: &ScanHistoryView, out: &mut dyn Write) -> io::Result<()> {
    let envelope = serde_json::json!({
        "schema": "gridseak.scan_history.v1",
        "view": view,
    });
    serde_json::to_writer_pretty(&mut *out, &envelope).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

fn render_llm(view: &ScanHistoryView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "[gridseak scan_history repo={}]", view.repo_name)?;
    writeln!(out, "## scans")?;
    if view.scans.is_empty() {
        writeln!(out, "(none)")?;
        return Ok(());
    }
    for row in &view.scans {
        writeln!(
            out,
            "{date} {branch} {commit} score={score} crit={crit} high={high} cycles={cyc} hot={hot} dead={dead} coupling={cpl} id={id}",
            date = row.date,
            branch = dirty_branch(&row.branch, row.dirty),
            commit = row.commit_short,
            score = row
                .score
                .map(|s| s.to_string())
                .unwrap_or_else(|| "—".into()),
            crit = row.critical,
            high = row.high,
            cyc = row.cycles,
            hot = row.hotspots,
            dead = row.dead_code,
            cpl = row
                .coupling
                .map(|c| format!("{c:.2}"))
                .unwrap_or_else(|| "—".into()),
            id = row.scan_id,
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn shorten_date(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|_| rfc3339.split('T').next().unwrap_or(rfc3339).to_string())
}

fn short_commit(commit: &str) -> &str {
    if commit.len() > 7 {
        &commit[..7]
    } else {
        commit
    }
}

fn dirty_branch(branch: &str, dirty: Option<bool>) -> String {
    if dirty.unwrap_or(false) {
        format!("{branch}*")
    } else {
        branch.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(date: &str, score: u32, crit: i64, cyc: i64) -> ScanHistoryRow {
        ScanHistoryRow {
            scan_id: format!("scan_{date}"),
            date: date.into(),
            branch: "main".into(),
            commit_short: "abcdef1".into(),
            dirty: Some(false),
            score: Some(score),
            score_band: ScoreBand::from_score(Some(score)),
            critical: crit,
            high: 0,
            cycles: cyc,
            hotspots: 0,
            dead_code: 0,
            coupling: Some(0.42),
            primary_language: Some("rust".into()),
        }
    }

    fn view() -> ScanHistoryView {
        ScanHistoryView {
            repo_name: "demo".into(),
            scans: vec![row("2026-05-17", 62, 2, 3), row("2026-05-10", 58, 3, 4)],
        }
    }

    #[test]
    fn table_wide_lists_all_columns() {
        let mut buf = Vec::new();
        render(
            &HistoryFormat::Table {
                layout: Layout::Wide,
            },
            &view(),
            &mut buf,
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("Date"));
        assert!(out.contains("Coupling"));
        assert!(out.contains("2026-05-17"));
        assert!(out.contains("0.42"));
    }

    #[test]
    fn empty_view_renders_helpful_hint() {
        let mut buf = Vec::new();
        render(
            &HistoryFormat::Table {
                layout: Layout::Wide,
            },
            &ScanHistoryView {
                repo_name: "demo".into(),
                scans: vec![],
            },
            &mut buf,
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("no scans recorded yet"));
    }

    #[test]
    fn snapshot_history_table_wide() {
        let mut buf = Vec::new();
        render(
            &HistoryFormat::Table {
                layout: Layout::Wide,
            },
            &view(),
            &mut buf,
        )
        .unwrap();
        insta::assert_snapshot!("history_table_wide", String::from_utf8(buf).unwrap());
    }

    #[test]
    fn snapshot_history_markdown() {
        let mut buf = Vec::new();
        render(&HistoryFormat::Markdown, &view(), &mut buf).unwrap();
        insta::assert_snapshot!("history_markdown", String::from_utf8(buf).unwrap());
    }

    #[test]
    fn json_envelope_is_stable() {
        let mut buf = Vec::new();
        render(&HistoryFormat::Json, &view(), &mut buf).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(json["schema"], "gridseak.scan_history.v1");
        assert_eq!(json["view"]["scans"][0]["scan_id"], "scan_2026-05-17");
    }
}
