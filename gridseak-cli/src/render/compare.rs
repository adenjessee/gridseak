//! `gridseak compare [PATH]` view + renderers.
//!
//! Output mirrors the spec's example (lines 226-234):
//!
//! ```text
//! Scan History
//! Date        Branch  Commit   Score  Critical  High  Cycles  Hotspots  Dead  Coupling
//! 2026-05-17  main    9f3c2a1  62     2         7     3       7         18    0.42
//! 2026-05-10  main    7ad182f  58     3         9     4       8         22    0.47
//! Delta                         +4     -1        -2    -1      -1        -4    -0.05
//! ```
//!
//! Two scans (from/to) become three rows: the older "from", the
//! newer "to", and a third "Delta" row carrying signed differences
//! per column. Rendering is the same width-aware ladder used by
//! every other table renderer in this module.

use std::io::{self, Write};

use serde::Serialize;

use crate::render::history::ScanHistoryRow;
use crate::render::width::Layout;

#[derive(Debug, Clone, Serialize)]
pub struct ScanCompareView {
    pub repo_name: String,
    pub from: ScanHistoryRow,
    pub to: ScanHistoryRow,
    pub delta: ScanDelta,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanDelta {
    pub score: Option<i32>,
    pub critical: i64,
    pub high: i64,
    pub cycles: i64,
    pub hotspots: i64,
    pub dead_code: i64,
    pub coupling: Option<f64>,
}

impl ScanCompareView {
    pub fn build(repo_name: String, from: ScanHistoryRow, to: ScanHistoryRow) -> Self {
        let delta = ScanDelta {
            score: match (from.score, to.score) {
                (Some(a), Some(b)) => Some(b as i32 - a as i32),
                _ => None,
            },
            critical: to.critical - from.critical,
            high: to.high - from.high,
            cycles: to.cycles - from.cycles,
            hotspots: to.hotspots - from.hotspots,
            dead_code: to.dead_code - from.dead_code,
            coupling: match (from.coupling, to.coupling) {
                (Some(a), Some(b)) => Some(b - a),
                _ => None,
            },
        };
        Self {
            repo_name,
            from,
            to,
            delta,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CompareFormat {
    Table { layout: Layout },
    Markdown,
    Json,
    ForLlm,
}

pub fn render(
    format: &CompareFormat,
    view: &ScanCompareView,
    out: &mut dyn Write,
) -> io::Result<()> {
    match format {
        CompareFormat::Table { layout } => render_table(view, *layout, out),
        CompareFormat::Markdown => render_markdown(view, out),
        CompareFormat::Json => render_json(view, out),
        CompareFormat::ForLlm => render_llm(view, out),
    }
}

fn render_table(view: &ScanCompareView, layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "Scan Compare — {}", view.repo_name)?;
    match layout {
        Layout::Wide => render_table_wide(view, out),
        Layout::Medium | Layout::Narrow => render_table_compact(view, out),
        Layout::Plain => render_table_plain(view, out),
    }
}

fn render_table_wide(view: &ScanCompareView, out: &mut dyn Write) -> io::Result<()> {
    let header = [
        "Date", "Branch", "Commit", "Score", "Critical", "High", "Cycles", "Hotspots", "Dead",
        "Coupling",
    ];

    let from = row_cells(&view.from);
    let to = row_cells(&view.to);
    let delta = delta_cells(&view.delta);

    let mut widths: Vec<usize> = header.iter().map(|c| c.len()).collect();
    for row in [&from, &to, &delta] {
        for (i, c) in row.iter().enumerate() {
            widths[i] = widths[i].max(c.len());
        }
    }

    write_columns(
        out,
        &header.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        &widths,
    )?;
    write_columns(out, &from, &widths)?;
    write_columns(out, &to, &widths)?;
    write_columns(out, &delta, &widths)?;
    Ok(())
}

fn render_table_compact(view: &ScanCompareView, out: &mut dyn Write) -> io::Result<()> {
    let header = [
        "Metric",
        view.from.date.as_str(),
        view.to.date.as_str(),
        "Delta",
    ];
    let widths = [10usize, header[1].len().max(8), header[2].len().max(8), 8];

    write_columns(
        out,
        &header.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        &widths,
    )?;
    write_columns(
        out,
        &[
            "score".into(),
            cell_score(view.from.score),
            cell_score(view.to.score),
            signed_score(view.delta.score),
        ],
        &widths,
    )?;
    write_columns(
        out,
        &[
            "critical".into(),
            view.from.critical.to_string(),
            view.to.critical.to_string(),
            signed_i64(view.delta.critical),
        ],
        &widths,
    )?;
    write_columns(
        out,
        &[
            "high".into(),
            view.from.high.to_string(),
            view.to.high.to_string(),
            signed_i64(view.delta.high),
        ],
        &widths,
    )?;
    write_columns(
        out,
        &[
            "cycles".into(),
            view.from.cycles.to_string(),
            view.to.cycles.to_string(),
            signed_i64(view.delta.cycles),
        ],
        &widths,
    )?;
    write_columns(
        out,
        &[
            "hotspots".into(),
            view.from.hotspots.to_string(),
            view.to.hotspots.to_string(),
            signed_i64(view.delta.hotspots),
        ],
        &widths,
    )?;
    write_columns(
        out,
        &[
            "dead".into(),
            view.from.dead_code.to_string(),
            view.to.dead_code.to_string(),
            signed_i64(view.delta.dead_code),
        ],
        &widths,
    )?;
    write_columns(
        out,
        &[
            "coupling".into(),
            cell_coupling(view.from.coupling),
            cell_coupling(view.to.coupling),
            signed_coupling(view.delta.coupling),
        ],
        &widths,
    )?;
    Ok(())
}

fn render_table_plain(view: &ScanCompareView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "from: {}", flatten_row(&view.from))?;
    writeln!(out, "to:   {}", flatten_row(&view.to))?;
    writeln!(out, "delta: {}", flatten_delta(&view.delta))?;
    Ok(())
}

fn render_markdown(view: &ScanCompareView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "# Scan Compare — {}", view.repo_name)?;
    writeln!(out)?;
    writeln!(
        out,
        "| Metric | {} | {} | Delta |",
        view.from.date, view.to.date
    )?;
    writeln!(out, "|--------|----:|----:|------:|")?;
    writeln!(
        out,
        "| score | {} | {} | {} |",
        cell_score(view.from.score),
        cell_score(view.to.score),
        signed_score(view.delta.score)
    )?;
    writeln!(
        out,
        "| critical | {} | {} | {} |",
        view.from.critical,
        view.to.critical,
        signed_i64(view.delta.critical)
    )?;
    writeln!(
        out,
        "| high | {} | {} | {} |",
        view.from.high,
        view.to.high,
        signed_i64(view.delta.high)
    )?;
    writeln!(
        out,
        "| cycles | {} | {} | {} |",
        view.from.cycles,
        view.to.cycles,
        signed_i64(view.delta.cycles)
    )?;
    writeln!(
        out,
        "| hotspots | {} | {} | {} |",
        view.from.hotspots,
        view.to.hotspots,
        signed_i64(view.delta.hotspots)
    )?;
    writeln!(
        out,
        "| dead | {} | {} | {} |",
        view.from.dead_code,
        view.to.dead_code,
        signed_i64(view.delta.dead_code)
    )?;
    writeln!(
        out,
        "| coupling | {} | {} | {} |",
        cell_coupling(view.from.coupling),
        cell_coupling(view.to.coupling),
        signed_coupling(view.delta.coupling)
    )?;
    Ok(())
}

fn render_json(view: &ScanCompareView, out: &mut dyn Write) -> io::Result<()> {
    let envelope = serde_json::json!({
        "schema": "gridseak.scan_compare.v1",
        "view": view,
    });
    serde_json::to_writer_pretty(&mut *out, &envelope).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

fn render_llm(view: &ScanCompareView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(
        out,
        "[gridseak scan_compare repo={} from={} to={}]",
        view.repo_name, view.from.scan_id, view.to.scan_id
    )?;
    writeln!(out, "from: {}", flatten_row(&view.from))?;
    writeln!(out, "to: {}", flatten_row(&view.to))?;
    writeln!(out, "delta: {}", flatten_delta(&view.delta))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn row_cells(row: &ScanHistoryRow) -> Vec<String> {
    let branch = if row.dirty.unwrap_or(false) {
        format!("{}*", row.branch)
    } else {
        row.branch.clone()
    };
    vec![
        row.date.clone(),
        branch,
        row.commit_short.clone(),
        cell_score(row.score),
        row.critical.to_string(),
        row.high.to_string(),
        row.cycles.to_string(),
        row.hotspots.to_string(),
        row.dead_code.to_string(),
        cell_coupling(row.coupling),
    ]
}

fn delta_cells(delta: &ScanDelta) -> Vec<String> {
    vec![
        "Delta".into(),
        String::new(),
        String::new(),
        signed_score(delta.score),
        signed_i64(delta.critical),
        signed_i64(delta.high),
        signed_i64(delta.cycles),
        signed_i64(delta.hotspots),
        signed_i64(delta.dead_code),
        signed_coupling(delta.coupling),
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

fn cell_score(score: Option<u32>) -> String {
    score.map(|s| s.to_string()).unwrap_or_else(|| "—".into())
}

fn cell_coupling(coupling: Option<f64>) -> String {
    coupling
        .map(|c| format!("{c:.2}"))
        .unwrap_or_else(|| "—".into())
}

fn signed_score(delta: Option<i32>) -> String {
    delta
        .map(|d| {
            if d > 0 {
                format!("+{d}")
            } else {
                d.to_string()
            }
        })
        .unwrap_or_else(|| "—".into())
}

fn signed_i64(d: i64) -> String {
    if d > 0 {
        format!("+{d}")
    } else {
        d.to_string()
    }
}

fn signed_coupling(d: Option<f64>) -> String {
    d.map(|d| {
        if d > 0.0 {
            format!("+{d:.2}")
        } else {
            format!("{d:.2}")
        }
    })
    .unwrap_or_else(|| "—".into())
}

fn flatten_row(row: &ScanHistoryRow) -> String {
    format!(
        "{date} {branch} {commit} score={score} crit={crit} high={high} cycles={cyc} hot={hot} dead={dead} coupling={cpl}",
        date = row.date,
        branch = row.branch,
        commit = row.commit_short,
        score = cell_score(row.score),
        crit = row.critical,
        high = row.high,
        cyc = row.cycles,
        hot = row.hotspots,
        dead = row.dead_code,
        cpl = cell_coupling(row.coupling),
    )
}

fn flatten_delta(d: &ScanDelta) -> String {
    format!(
        "score={score} crit={crit} high={high} cycles={cyc} hot={hot} dead={dead} coupling={cpl}",
        score = signed_score(d.score),
        crit = signed_i64(d.critical),
        high = signed_i64(d.high),
        cyc = signed_i64(d.cycles),
        hot = signed_i64(d.hotspots),
        dead = signed_i64(d.dead_code),
        cpl = signed_coupling(d.coupling),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::view::ScoreBand;

    fn history_row(date: &str, score: u32, critical: i64, cycles: i64) -> ScanHistoryRow {
        ScanHistoryRow {
            scan_id: format!("scan_{date}"),
            date: date.into(),
            branch: "main".into(),
            commit_short: date[..7].to_string(),
            dirty: Some(false),
            score: Some(score),
            score_band: ScoreBand::from_score(Some(score)),
            critical,
            high: 7,
            cycles,
            hotspots: 7,
            dead_code: 18,
            coupling: Some(0.42),
            primary_language: Some("rust".into()),
        }
    }

    fn view() -> ScanCompareView {
        ScanCompareView::build(
            "demo".into(),
            history_row("2026-05-10", 58, 3, 4),
            history_row("2026-05-17", 62, 2, 3),
        )
    }

    #[test]
    fn delta_signs_match_spec_example() {
        let v = view();
        assert_eq!(v.delta.score, Some(4));
        assert_eq!(v.delta.critical, -1);
        assert_eq!(v.delta.cycles, -1);
        assert_eq!(v.delta.dead_code, 0);
    }

    #[test]
    fn snapshot_compare_table_wide() {
        let mut buf = Vec::new();
        render(
            &CompareFormat::Table {
                layout: Layout::Wide,
            },
            &view(),
            &mut buf,
        )
        .unwrap();
        insta::assert_snapshot!("compare_table_wide", String::from_utf8(buf).unwrap());
    }

    #[test]
    fn snapshot_compare_markdown() {
        let mut buf = Vec::new();
        render(&CompareFormat::Markdown, &view(), &mut buf).unwrap();
        insta::assert_snapshot!("compare_markdown", String::from_utf8(buf).unwrap());
    }

    #[test]
    fn json_envelope_is_stable() {
        let mut buf = Vec::new();
        render(&CompareFormat::Json, &view(), &mut buf).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(json["schema"], "gridseak.scan_compare.v1");
        assert_eq!(json["view"]["delta"]["score"], 4);
    }
}
