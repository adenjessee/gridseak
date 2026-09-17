//! Single-file offline HTML report.

use std::io::{self, Write};

use super::report_card::render_report_card;
use crate::graph_queries::agent_tools::{
    DiffImpactView, StructuralInvariantsView, VerifyClaimView,
};

pub fn render_html(
    impact: &DiffImpactView,
    claims: &[VerifyClaimView],
    invariants: &StructuralInvariantsView,
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut card = Vec::new();
    render_report_card(impact, claims, invariants, &mut card)?;
    let body = html_escape(&String::from_utf8_lossy(&card));
    writeln!(out, "<!DOCTYPE html><html><head><meta charset=\"utf-8\">")?;
    writeln!(out, "<title>GridSeak report</title>")?;
    writeln!(
        out,
        "<style>body{{font-family:ui-monospace,monospace;margin:2rem;background:#111;color:#eee}} pre{{white-space:pre-wrap}}</style>"
    )?;
    writeln!(out, "</head><body><pre>{body}</pre></body></html>")?;
    Ok(())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
