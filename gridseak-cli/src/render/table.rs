//! Width-aware terminal renderer for [`ScanReportView`].
//!
//! The renderer never produces ANSI colour codes — that is a separate
//! concern Stage 3 may bolt on once we know what the "default human"
//! palette should be. The output is pure UTF-8 text so the same bytes
//! work for terminal, piping into `less`, or capturing in a snapshot
//! test.

use std::io::{self, Write};

use crate::render::view::{ConfidenceNote, MetricRow, PriorityRow, ScanReportView};
use crate::render::width::Layout;

/// Render the hero report (the spec's "Example Output", §Target
/// First-Run UX) for `view` using the layout `layout`. Writes to
/// `out` and returns any I/O error verbatim.
pub fn render_hero(view: &ScanReportView, layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    write_header(view, layout, out)?;
    writeln!(out)?;
    write_priorities(&view.priorities, layout, out)?;
    writeln!(out)?;
    write_metrics(&view.metrics, layout, out)?;
    writeln!(out)?;
    write_next(view, layout, out)?;
    writeln!(out)?;
    writeln!(out, "Free vs hosted SaaS (signaling)")?;
    crate::render::tier_signaling::write_text(&view.tier_signal, out)?;
    Ok(())
}

/// Convenience: render to a `String` using the supplied layout.
/// Snapshot tests use this; the CLI driver writes straight to
/// stdout.
#[allow(dead_code)] // exercised by unit tests; reserved for snapshot harness.
pub fn render_hero_to_string(view: &ScanReportView, layout: Layout) -> String {
    let mut buf = Vec::with_capacity(2048);
    render_hero(view, layout, &mut buf).expect("write to Vec<u8> never fails");
    String::from_utf8(buf).expect("renderer emits only valid UTF-8")
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn write_header(view: &ScanReportView, _layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "GridSeak Structural Health Report")?;
    writeln!(out, "Repo: {}", view.repo_name)?;
    writeln!(out, "Scan: {}", view.scanned_at_display)?;
    if let Some(branch) = &view.branch {
        let dirty = if view.dirty.unwrap_or(false) {
            " (dirty)"
        } else {
            ""
        };
        writeln!(out, "Branch: {branch}{dirty}")?;
    }
    if let Some(commit) = &view.commit_short {
        writeln!(out, "Commit: {commit}")?;
    }
    let languages = if view.languages.is_empty() {
        "—".to_string()
    } else {
        view.languages.join(", ")
    };
    writeln!(out, "Languages: {languages}")?;
    let score_value = match view.score {
        Some(s) => format!("{s}/100"),
        None => "—".into(),
    };
    writeln!(out, "Score: {score_value} ({})", view.score_band.label())?;
    if !view.confidence_notes.is_empty() {
        writeln!(
            out,
            "Confidence: {}",
            format_confidence_notes(&view.confidence_notes)
        )?;
    }
    if !view.schema_caveats.is_empty() {
        writeln!(out, "Caveats: {}", view.schema_caveats.join(", "))?;
    }
    Ok(())
}

fn format_confidence_notes(notes: &[ConfidenceNote]) -> String {
    notes
        .iter()
        .map(|note| format!("{} for {}", note.level, note.metrics.join(", ")))
        .collect::<Vec<_>>()
        .join("; ")
}

// ---------------------------------------------------------------------------
// Top Priorities
// ---------------------------------------------------------------------------

fn write_priorities(rows: &[PriorityRow], layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "Top Priorities")?;
    if rows.is_empty() {
        writeln!(
            out,
            "(none — no findings ranked as priorities on this scan.)"
        )?;
        return Ok(());
    }
    match layout {
        Layout::Wide => write_priorities_wide(rows, out),
        Layout::Medium => write_priorities_medium(rows, out),
        Layout::Narrow => write_priorities_narrow(rows, out),
        Layout::Plain => write_priorities_plain(rows, out),
    }
}

fn write_priorities_wide(rows: &[PriorityRow], out: &mut dyn Write) -> io::Result<()> {
    // Column widths chosen to mirror the spec example; computed
    // against the rendered rows to keep alignment when finding labels
    // or targets are longer than the example.
    let header = ["Rank", "Severity", "Finding", "Target", "Evidence"];
    let col_widths = compute_priority_widths(rows, &header, 80, 40);
    let header_cells: Vec<String> = header.iter().map(|s| s.to_string()).collect();
    write_priority_row(out, &header_cells, &col_widths)?;
    for row in rows {
        let cells = vec![
            row.rank.to_string(),
            row.severity.clone(),
            row.finding.clone(),
            truncate(&row.target, col_widths[3]),
            truncate(&row.evidence, col_widths[4]),
        ];
        write_priority_row(out, &cells, &col_widths)?;
    }
    Ok(())
}

fn write_priorities_medium(rows: &[PriorityRow], out: &mut dyn Write) -> io::Result<()> {
    // Drop the Finding label column to save horizontal space; the
    // severity + target + evidence triplet still answers "what is
    // wrong and where".
    let header = ["Rank", "Severity", "Target", "Evidence"];
    let col_widths = compute_priority_widths_medium(rows, &header, 40, 30);
    let header_cells: Vec<String> = header.iter().map(|s| s.to_string()).collect();
    write_priority_row(out, &header_cells, &col_widths)?;
    for row in rows {
        let cells = vec![
            row.rank.to_string(),
            row.severity.clone(),
            truncate(&row.target, col_widths[2]),
            truncate(&row.evidence, col_widths[3]),
        ];
        write_priority_row(out, &cells, &col_widths)?;
    }
    Ok(())
}

fn write_priorities_narrow(rows: &[PriorityRow], out: &mut dyn Write) -> io::Result<()> {
    for (idx, row) in rows.iter().enumerate() {
        if idx > 0 {
            writeln!(out)?;
        }
        writeln!(out, "#{} {} — {}", row.rank, row.severity, row.finding)?;
        writeln!(out, "  Target:   {}", row.target)?;
        writeln!(out, "  Evidence: {}", row.evidence)?;
        if let Some(conf) = &row.confidence {
            writeln!(out, "  Confidence: {conf}")?;
        }
    }
    Ok(())
}

fn write_priorities_plain(rows: &[PriorityRow], out: &mut dyn Write) -> io::Result<()> {
    for row in rows {
        writeln!(
            out,
            "#{} [{}] {} :: {} :: {}",
            row.rank, row.severity, row.finding, row.target, row.evidence
        )?;
    }
    Ok(())
}

fn write_priority_row(out: &mut dyn Write, cells: &[String], widths: &[usize]) -> io::Result<()> {
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

fn compute_priority_widths(
    rows: &[PriorityRow],
    header: &[&str; 5],
    target_max: usize,
    evidence_max: usize,
) -> [usize; 5] {
    let mut widths = [4, 8, 20, 30, 30];
    for (idx, label) in header.iter().enumerate() {
        widths[idx] = widths[idx].max(label.len());
    }
    for row in rows {
        widths[0] = widths[0].max(row.rank.to_string().len());
        widths[1] = widths[1].max(row.severity.len());
        widths[2] = widths[2].max(row.finding.len());
        widths[3] = widths[3].max(row.target.len()).min(target_max);
        widths[4] = widths[4].max(row.evidence.len()).min(evidence_max);
    }
    widths
}

fn compute_priority_widths_medium(
    rows: &[PriorityRow],
    header: &[&str; 4],
    target_max: usize,
    evidence_max: usize,
) -> [usize; 4] {
    let mut widths = [4, 8, 30, 30];
    for (idx, label) in header.iter().enumerate() {
        widths[idx] = widths[idx].max(label.len());
    }
    for row in rows {
        widths[0] = widths[0].max(row.rank.to_string().len());
        widths[1] = widths[1].max(row.severity.len());
        widths[2] = widths[2].max(row.target.len()).min(target_max);
        widths[3] = widths[3].max(row.evidence.len()).min(evidence_max);
    }
    widths
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

fn write_metrics(rows: &[MetricRow], layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "Metrics")?;
    if rows.is_empty() {
        writeln!(out, "(no metrics persisted on this scan.)")?;
        return Ok(());
    }
    match layout {
        Layout::Wide | Layout::Medium => write_metrics_tabular(rows, out),
        Layout::Narrow => write_metrics_narrow(rows, out),
        Layout::Plain => write_metrics_plain(rows, out),
    }
}

fn write_metrics_tabular(rows: &[MetricRow], out: &mut dyn Write) -> io::Result<()> {
    let header = ["Metric", "Value", "Status"];
    let mut widths = [header[0].len(), header[1].len(), header[2].len()];
    for row in rows {
        widths[0] = widths[0].max(row.name.len());
        widths[1] = widths[1].max(row.value.len());
        widths[2] = widths[2].max(row.status.len());
    }
    write_metric_row(out, &header.map(str::to_string), &widths)?;
    for row in rows {
        write_metric_row(
            out,
            &[row.name.clone(), row.value.clone(), row.status.clone()],
            &widths,
        )?;
    }
    Ok(())
}

fn write_metrics_narrow(rows: &[MetricRow], out: &mut dyn Write) -> io::Result<()> {
    for row in rows {
        writeln!(out, "{}: {} — {}", row.name, row.value, row.status)?;
    }
    Ok(())
}

fn write_metrics_plain(rows: &[MetricRow], out: &mut dyn Write) -> io::Result<()> {
    for row in rows {
        writeln!(out, "{}: {} ({})", row.name, row.value, row.status)?;
    }
    Ok(())
}

fn write_metric_row(
    out: &mut dyn Write,
    cells: &[String; 3],
    widths: &[usize; 3],
) -> io::Result<()> {
    writeln!(
        out,
        "{:<n0$}  {:<n1$}  {:<n2$}",
        cells[0],
        cells[1],
        cells[2],
        n0 = widths[0],
        n1 = widths[1],
        n2 = widths[2],
    )
}

// ---------------------------------------------------------------------------
// Next commands
// ---------------------------------------------------------------------------

fn write_next(view: &ScanReportView, _layout: Layout, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "Next")?;
    for cmd in &view.next_commands {
        writeln!(out, "{cmd}")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn truncate(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_string();
    }
    if max_len <= 1 {
        return "…".into();
    }
    let mut taken: String = value.chars().take(max_len.saturating_sub(1)).collect();
    taken.push('…');
    taken
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::render::view::fixture_spec_example;

    fn sample_view() -> ScanReportView {
        fixture_spec_example()
    }

    #[test]
    fn wide_layout_contains_all_columns() {
        let s = render_hero_to_string(&sample_view(), Layout::Wide);
        assert!(s.contains("GridSeak Structural Health Report"));
        assert!(s.contains("Repo: my-service"));
        assert!(s.contains("Score: 62/100 (Moderate risk)"));
        assert!(
            s.contains("Confidence: high for blast radius, coupling, depth; medium for dead code")
                || s.contains("high for")
        );
        assert!(s.contains("Rank"));
        assert!(s.contains("Severity"));
        assert!(s.contains("Finding"));
        assert!(s.contains("Target"));
        assert!(s.contains("Evidence"));
        assert!(s.contains("Blast radius hotspot"));
        assert!(s.contains("Metric"));
        assert!(s.contains("Value"));
        assert!(s.contains("Status"));
        assert!(s.contains("Health score"));
        assert!(s.contains("Next"));
        assert!(s.contains("gridseak compare --previous"));
    }

    #[test]
    fn medium_layout_drops_finding_column() {
        let s = render_hero_to_string(&sample_view(), Layout::Medium);
        assert!(s.contains("Rank"));
        assert!(s.contains("Severity"));
        assert!(s.contains("Target"));
        assert!(s.contains("Evidence"));
        let header_line = s
            .lines()
            .find(|line| line.contains("Rank") && line.contains("Target"))
            .unwrap_or("");
        // Sanity: Finding shouldn't be a column header in medium.
        assert!(!header_line.contains("Finding"));
    }

    #[test]
    fn narrow_layout_uses_cards() {
        let s = render_hero_to_string(&sample_view(), Layout::Narrow);
        assert!(s.contains("#1 Critical — Blast radius hotspot"));
        assert!(s.contains("  Target:   auth/createSession"));
        assert!(s.contains("  Evidence: 41 downstream nodes"));
    }

    #[test]
    fn plain_layout_one_line_per_row() {
        let s = render_hero_to_string(&sample_view(), Layout::Plain);
        assert!(s.contains(
            "#1 [Critical] Blast radius hotspot :: auth/createSession :: 41 downstream nodes"
        ));
        assert!(s.contains("Cycles: 3 (Risk)"));
    }

    #[test]
    fn truncate_respects_max_length() {
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("hi", 5), "hi");
        assert_eq!(truncate("anything", 0), "…");
        assert_eq!(truncate("anything", 1), "…");
    }

    /// Stage 2 test gate: lock the spec's "Example Output"
    /// (CLI_SHADOW_MODE_DISTRIBUTION_SPEC.md, lines 146-178) to a
    /// byte-identical snapshot. Future stages MUST regenerate
    /// intentionally when they change the renderer; an accidental
    /// drift will fail this test.
    #[test]
    fn hero_report_wide_snapshot() {
        let rendered = render_hero_to_string(&sample_view(), Layout::Wide);
        insta::assert_snapshot!("hero_report_wide", rendered);
    }

    #[test]
    fn hero_report_medium_snapshot() {
        let rendered = render_hero_to_string(&sample_view(), Layout::Medium);
        insta::assert_snapshot!("hero_report_medium", rendered);
    }

    #[test]
    fn hero_report_narrow_snapshot() {
        let rendered = render_hero_to_string(&sample_view(), Layout::Narrow);
        insta::assert_snapshot!("hero_report_narrow", rendered);
    }

    #[test]
    fn hero_report_plain_snapshot() {
        let rendered = render_hero_to_string(&sample_view(), Layout::Plain);
        insta::assert_snapshot!("hero_report_plain", rendered);
    }
}
