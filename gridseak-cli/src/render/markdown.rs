//! GitHub-friendly Markdown renderer for the hero scan report.
//!
//! Output is intentionally bog-standard CommonMark + GitHub-flavored
//! tables so a `gridseak scan . --format markdown` paste lands cleanly
//! in GitHub issues, PR descriptions, and Slack with code-block
//! tables. No HTML, no fenced code blocks around the tables, no
//! collapsible details: the agent or human reading this should be able
//! to copy/paste without post-processing.

use std::io::{self, Write};

use crate::render::view::{ConfidenceNote, MetricRow, PriorityRow, ScanReportView};

/// Render `view` as GitHub-flavored Markdown to `out`.
pub fn render_hero(view: &ScanReportView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "# GridSeak Structural Health Report")?;
    writeln!(out)?;
    write_overview(view, out)?;
    writeln!(out)?;
    write_priorities(&view.priorities, out)?;
    writeln!(out)?;
    write_metrics(&view.metrics, out)?;
    writeln!(out)?;
    write_next(view, out)?;
    writeln!(out)?;
    crate::render::tier_signaling::write_markdown(&view.tier_signal, out)?;
    Ok(())
}

#[allow(dead_code)] // exercised by unit tests; reserved for snapshot harness.
pub fn render_hero_to_string(view: &ScanReportView) -> String {
    let mut buf = Vec::with_capacity(2048);
    render_hero(view, &mut buf).expect("Vec<u8> writes never fail");
    String::from_utf8(buf).expect("markdown renderer emits valid UTF-8")
}

fn write_overview(view: &ScanReportView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "- **Repo:** {}", view.repo_name)?;
    writeln!(out, "- **Scan:** {}", view.scanned_at_display)?;
    if let Some(branch) = &view.branch {
        let dirty = if view.dirty.unwrap_or(false) {
            " (dirty)"
        } else {
            ""
        };
        writeln!(out, "- **Branch:** {branch}{dirty}")?;
    }
    if let Some(commit) = &view.commit_short {
        writeln!(out, "- **Commit:** `{commit}`")?;
    }
    let langs = if view.languages.is_empty() {
        "—".to_string()
    } else {
        view.languages.join(", ")
    };
    writeln!(out, "- **Languages:** {langs}")?;
    let score = match view.score {
        Some(s) => format!("{s}/100"),
        None => "—".into(),
    };
    writeln!(out, "- **Score:** {score} ({})", view.score_band.label())?;
    if !view.confidence_notes.is_empty() {
        writeln!(
            out,
            "- **Confidence:** {}",
            format_confidence(&view.confidence_notes)
        )?;
    }
    if !view.schema_caveats.is_empty() {
        writeln!(out, "- **Caveats:** {}", view.schema_caveats.join(", "))?;
    }
    Ok(())
}

fn format_confidence(notes: &[ConfidenceNote]) -> String {
    notes
        .iter()
        .map(|note| format!("{} for {}", note.level, note.metrics.join(", ")))
        .collect::<Vec<_>>()
        .join("; ")
}

fn write_priorities(rows: &[PriorityRow], out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "## Top Priorities")?;
    writeln!(out)?;
    if rows.is_empty() {
        writeln!(out, "_No priorities ranked on this scan._")?;
        return Ok(());
    }
    writeln!(out, "| Rank | Severity | Finding | Target | Evidence |")?;
    writeln!(out, "|-----:|----------|---------|--------|----------|")?;
    for row in rows {
        writeln!(
            out,
            "| {} | {} | {} | `{}` | {} |",
            row.rank,
            md_escape(&row.severity),
            md_escape(&row.finding),
            md_escape(&row.target),
            md_escape(&row.evidence),
        )?;
    }
    Ok(())
}

fn write_metrics(rows: &[MetricRow], out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "## Metrics")?;
    writeln!(out)?;
    if rows.is_empty() {
        writeln!(out, "_No metrics persisted on this scan._")?;
        return Ok(());
    }
    writeln!(out, "| Metric | Value | Status |")?;
    writeln!(out, "|--------|-------|--------|")?;
    for row in rows {
        writeln!(
            out,
            "| {} | {} | {} |",
            md_escape(&row.name),
            md_escape(&row.value),
            md_escape(&row.status),
        )?;
    }
    Ok(())
}

fn write_next(view: &ScanReportView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "## Next")?;
    writeln!(out)?;
    for cmd in &view.next_commands {
        writeln!(out, "- `{cmd}`")?;
    }
    Ok(())
}

use super::util::md_escape;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::view::{
        fixture_spec_example, ConfidenceNote, MetricRow, PriorityRow, ScoreBand,
    };

    fn view() -> ScanReportView {
        ScanReportView {
            repo_name: "demo".into(),
            root_path: "/tmp/demo".into(),
            scan_id: "scan_test".into(),
            scanned_at_display: "2026-05-17 11:12".into(),
            branch: Some("main".into()),
            commit_short: Some("abcdef1".into()),
            dirty: Some(true),
            languages: vec!["typescript".into()],
            score: Some(62),
            score_band: ScoreBand::Moderate,
            confidence_notes: vec![ConfidenceNote {
                level: "high".into(),
                metrics: vec!["coupling".into()],
            }],
            priorities: vec![PriorityRow {
                rank: 1,
                severity: "Critical".into(),
                finding_id: "finding_a".into(),
                finding: "Blast radius hotspot".into(),
                target: "auth/login".into(),
                evidence: "12 downstream nodes".into(),
                confidence: Some("high".into()),
            }],
            metrics: vec![MetricRow {
                name: "Cycles".into(),
                value: "3".into(),
                status: "Risk".into(),
                confidence: None,
            }],
            next_commands: vec!["gridseak explain finding_a".into()],
            schema_caveats: Vec::new(),
            tier_signal: crate::render::tier_signaling::TierSignal::default_v0(),
        }
    }

    #[test]
    fn renders_github_table() {
        let s = render_hero_to_string(&view());
        assert!(s.starts_with("# GridSeak Structural Health Report"));
        assert!(s.contains("| Rank | Severity | Finding | Target | Evidence |"));
        assert!(s.contains(
            "| 1 | Critical | Blast radius hotspot | `auth/login` | 12 downstream nodes |"
        ));
        assert!(s.contains("| Cycles | 3 | Risk |"));
        assert!(s.contains("**Branch:** main (dirty)"));
    }

    #[test]
    fn escapes_pipes_in_cells() {
        let mut v = view();
        v.priorities[0].target = "a|b".into();
        let s = render_hero_to_string(&v);
        assert!(s.contains("`a\\|b`"));
    }

    /// Locks the Markdown rendering for the same fixture the table
    /// renderer uses (Stage 2 spec example). Failures here mean
    /// either a renderer drift or an intentional change that needs
    /// `cargo insta review`.
    #[test]
    fn hero_markdown_snapshot() {
        let rendered = render_hero_to_string(&fixture_spec_example());
        insta::assert_snapshot!("hero_markdown", rendered);
    }
}
