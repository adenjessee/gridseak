//! Free vs hosted-SaaS signaling block.
//!
//! Rendered as a small footer in every scan report. Its job is to set
//! honest expectations: what the on-machine binary does (free, MIT /
//! Apache-2.0, no telemetry), and what a future hosted SaaS layer
//! might add (multi-repo sync, team RBAC, audit log retention) — all
//! of which lives outside this repo when it exists at all.
//!
//! No enforcement happens here. No tier flag is read. The lists are
//! deliberately committed strings so a single edit moves every render
//! surface together (terminal table, markdown export, LLM bundle).

use std::io::{self, Write};

use serde::Serialize;

/// What the CLI surfaces about its own free / hosted posture.
///
/// `feedback_prompt` is intentionally short and mentions the
/// `gridseak feedback` command so the reader has a one-shot answer
/// to "how do I tell you what's missing?".
#[derive(Debug, Clone, Serialize)]
pub struct TierSignal {
    pub free: Vec<String>,
    pub future_hosted: Vec<String>,
    pub feedback_prompt: String,
}

impl TierSignal {
    /// Canonical v0.1 tier signal. The lists are deliberately small
    /// so the signaling block doesn't crowd the actual scan output.
    pub fn default_v0() -> Self {
        Self {
            free: vec![
                "Local scans (CLI + MCP server)".into(),
                "Scan history on this machine".into(),
                "Table / Markdown / JSON / LLM output".into(),
                "MCP server over stdio (fourteen tools)".into(),
                "Graph slice queries (callers, callees, blast radius, cycles)".into(),
            ],
            future_hosted: vec![
                "Multi-repo benchmarks across machines".into(),
                "Team-tier judgment sync".into(),
                "Audit log retention + RBAC".into(),
                "Organisation dashboards".into(),
            ],
            feedback_prompt: "Tell us what would unlock you: `gridseak feedback \"<text>\"`".into(),
        }
    }
}

/// Render the tier signal as a small footer block suitable for both
/// terminal and markdown output. The shape is the same in both — two
/// labelled bullet lists followed by a feedback prompt — so it sits
/// inside a markdown export without escaping.
pub fn write_text(signal: &TierSignal, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "Free in this build:")?;
    for item in &signal.free {
        writeln!(out, "  - {item}")?;
    }
    writeln!(out, "Future hosted SaaS (not in this binary):")?;
    for item in &signal.future_hosted {
        writeln!(out, "  - {item}")?;
    }
    writeln!(out)?;
    writeln!(out, "{}", signal.feedback_prompt)?;
    Ok(())
}

/// Render the tier signal as a GitHub-flavoured markdown section.
pub fn write_markdown(signal: &TierSignal, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "## Free vs hosted SaaS (signaling)")?;
    writeln!(out)?;
    writeln!(out, "**Included in this build (free, MIT):**")?;
    for item in &signal.free {
        writeln!(out, "- {item}")?;
    }
    writeln!(out)?;
    writeln!(out, "**Future hosted SaaS (not in this binary):**")?;
    for item in &signal.future_hosted {
        writeln!(out, "- {item}")?;
    }
    writeln!(out)?;
    writeln!(out, "{}", signal.feedback_prompt)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_v0_lists_match_spec() {
        let signal = TierSignal::default_v0();
        assert!(signal.free.iter().any(|s| s.contains("Local scans")));
        assert!(signal
            .future_hosted
            .iter()
            .any(|s| s.contains("Multi-repo")));
        assert!(signal.feedback_prompt.contains("gridseak feedback"));
    }

    #[test]
    fn write_text_produces_stable_shape() {
        let mut buf = Vec::new();
        write_text(&TierSignal::default_v0(), &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.starts_with("Free in this build:\n"));
        assert!(out.contains("Future hosted SaaS (not in this binary):"));
        assert!(out.ends_with("\"`\n"));
    }

    #[test]
    fn write_markdown_produces_h2_heading() {
        let mut buf = Vec::new();
        write_markdown(&TierSignal::default_v0(), &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("## Free vs hosted SaaS (signaling)"));
        assert!(out.contains("**Future hosted SaaS (not in this binary):**"));
    }
}
