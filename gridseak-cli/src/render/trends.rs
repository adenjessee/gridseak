//! `gridseak trends` view + renderers.
//!
//! Renders a time-series of one metric over a window of scans. The
//! TTY output uses simple block-character sparklines so the user
//! sees the shape of the trend at a glance; the Markdown output
//! uses a per-row arrow; JSON returns the raw points so downstream
//! consumers (CSV exporters, dashboards) can compute their own
//! charts.

use std::io::{self, Write};

use serde::Serialize;

use crate::render::width::Layout;

/// One data point on a trend series.
#[derive(Debug, Clone, Serialize)]
pub struct TrendPoint {
    pub scan_id: String,
    pub date: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendsView {
    pub repo_name: String,
    /// Display metric name (e.g. `"score"`, `"avg coupling"`).
    pub metric: String,
    /// Optional unit suffix (e.g. `"/100"`, `""`).
    pub unit: String,
    pub series: Vec<TrendPoint>,
}

#[derive(Debug, Clone)]
pub enum TrendsFormat {
    Table { layout: Layout },
    Markdown,
    Json,
    ForLlm,
}

pub fn render(format: &TrendsFormat, view: &TrendsView, out: &mut dyn Write) -> io::Result<()> {
    match format {
        TrendsFormat::Table { layout } => render_table(view, *layout, out),
        TrendsFormat::Markdown => render_markdown(view, out),
        TrendsFormat::Json => render_json(view, out),
        TrendsFormat::ForLlm => render_llm(view, out),
    }
}

fn render_table(view: &TrendsView, _layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "Trend — {} ({})", view.metric, view.repo_name)?;
    if view.series.is_empty() {
        writeln!(out, "(no data — run `gridseak scan .` a few times)")?;
        return Ok(());
    }
    let spark = sparkline(view.series.iter().map(|p| p.value));
    writeln!(out, "{spark}")?;
    writeln!(out)?;
    writeln!(out, "Date        Value")?;
    for point in &view.series {
        writeln!(
            out,
            "{date}  {value:.2}{unit}",
            date = point.date,
            value = point.value,
            unit = view.unit
        )?;
    }
    Ok(())
}

fn render_markdown(view: &TrendsView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "# Trend — {} ({})", view.metric, view.repo_name)?;
    writeln!(out)?;
    if view.series.is_empty() {
        writeln!(out, "_No data — run `gridseak scan .` a few times._")?;
        return Ok(());
    }
    let spark = sparkline(view.series.iter().map(|p| p.value));
    writeln!(out, "`{spark}`")?;
    writeln!(out)?;
    writeln!(out, "| Date | Value |")?;
    writeln!(out, "|------|------:|")?;
    for point in &view.series {
        writeln!(
            out,
            "| {date} | {value:.2}{unit} |",
            date = point.date,
            value = point.value,
            unit = view.unit
        )?;
    }
    Ok(())
}

fn render_json(view: &TrendsView, out: &mut dyn Write) -> io::Result<()> {
    let envelope = serde_json::json!({
        "schema": "gridseak.scan_trends.v1",
        "view": view,
    });
    serde_json::to_writer_pretty(&mut *out, &envelope).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

fn render_llm(view: &TrendsView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(
        out,
        "[gridseak scan_trends repo={} metric={} points={}]",
        view.repo_name,
        view.metric,
        view.series.len()
    )?;
    for point in &view.series {
        writeln!(
            out,
            "{date} value={value:.4} scan={id}",
            date = point.date,
            value = point.value,
            id = point.scan_id
        )?;
    }
    Ok(())
}

/// Render a Unicode-block sparkline from a series of values.
///
/// Returns an empty string when given an empty iterator. Uses the
/// 8-level block character set (`▁▂▃▄▅▆▇█`) which is widely
/// available in terminal fonts and avoids the more exotic Powerline
/// glyphs that don't render on stock Windows terminals.
pub fn sparkline(values: impl IntoIterator<Item = f64>) -> String {
    const TICKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let values: Vec<f64> = values.into_iter().collect();
    if values.is_empty() {
        return String::new();
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(f64::EPSILON);
    values
        .iter()
        .map(|v| {
            let scaled = ((v - min) / range) * (TICKS.len() as f64 - 1.0);
            TICKS[scaled.round().clamp(0.0, TICKS.len() as f64 - 1.0) as usize]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(date: &str, value: f64) -> TrendPoint {
        TrendPoint {
            scan_id: format!("scan_{date}"),
            date: date.into(),
            value,
        }
    }

    fn view() -> TrendsView {
        TrendsView {
            repo_name: "demo".into(),
            metric: "score".into(),
            unit: "/100".into(),
            series: vec![
                point("2026-05-10", 58.0),
                point("2026-05-14", 60.0),
                point("2026-05-17", 62.0),
            ],
        }
    }

    #[test]
    fn sparkline_handles_empty() {
        assert_eq!(sparkline(std::iter::empty::<f64>()), "");
    }

    #[test]
    fn sparkline_renders_full_range() {
        let s = sparkline([1.0, 8.0]);
        assert_eq!(s.chars().count(), 2);
        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars[0], '▁');
        assert_eq!(chars[1], '█');
    }

    #[test]
    fn snapshot_trends_table() {
        let mut buf = Vec::new();
        render(
            &TrendsFormat::Table {
                layout: Layout::Wide,
            },
            &view(),
            &mut buf,
        )
        .unwrap();
        insta::assert_snapshot!("trends_table_wide", String::from_utf8(buf).unwrap());
    }

    #[test]
    fn snapshot_trends_markdown() {
        let mut buf = Vec::new();
        render(&TrendsFormat::Markdown, &view(), &mut buf).unwrap();
        insta::assert_snapshot!("trends_markdown", String::from_utf8(buf).unwrap());
    }

    #[test]
    fn json_envelope_is_stable() {
        let mut buf = Vec::new();
        render(&TrendsFormat::Json, &view(), &mut buf).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(json["schema"], "gridseak.scan_trends.v1");
        assert_eq!(json["view"]["series"][2]["value"], 62.0);
    }
}
