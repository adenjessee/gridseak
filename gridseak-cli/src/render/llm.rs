//! Compact, agent-friendly text render of [`ScanReportView`].
//!
//! Optimised for fitting in an LLM context window: every section
//! starts with a stable heading so the model can find it deterministically;
//! every row uses short key/value lines instead of decorative borders;
//! a trailing manifest of `Next commands` keeps the agent in a tight
//! "act → call tool → act" loop instead of asking the user how to
//! continue.
//!
//! Token budgets are enforced by trimming the lowest-priority sections
//! first: schema caveats, then metric tail, then priority tail.
//! The output's first line always reports the actual estimated tokens
//! so callers can verify the renderer respected their `--budget`.

use std::io::{self, Write};

use crate::render::view::{ConfidenceNote, MetricRow, PriorityRow, ScanReportView};

/// Rough char-to-token ratio used when no explicit estimator is wired
/// in. ~4 chars/token is the rule-of-thumb used by OpenAI's tokenizer
/// for general English; close enough for budget enforcement and
/// well-known by anyone who has read the public guidance.
const APPROX_CHARS_PER_TOKEN: f64 = 4.0;

/// Estimate the token count of `text` using a char-count heuristic.
/// Stage 7 may swap this for a real tokenizer; the trait is the
/// integer-token-count surface either way.
pub fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count() as f64;
    (chars / APPROX_CHARS_PER_TOKEN).ceil() as usize
}

/// Render the scan view as an LLM-friendly text blob, respecting an
/// optional token budget. Writes to `out` and returns the rendered
/// string for snapshot tests / verification.
pub fn render(
    view: &ScanReportView,
    budget: Option<usize>,
    out: &mut dyn Write,
) -> io::Result<String> {
    let mut text = render_to_string(view, budget);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    out.write_all(text.as_bytes())?;
    Ok(text)
}

/// Build the LLM rendering as an in-memory `String`. Trim-to-budget
/// happens here so callers (e.g. `gridseak context`) can reuse the
/// logic without going through an `io::Write`.
pub fn render_to_string(view: &ScanReportView, budget: Option<usize>) -> String {
    let mut sections = build_sections(view);
    let mut rendered = render_sections(&sections);
    if let Some(budget) = budget {
        while estimate_tokens(&rendered) > budget && trim_one(&mut sections) {
            rendered = render_sections(&sections);
        }
    }
    let header = format!(
        "[gridseak scan_report tokens≈{}{}]\n",
        estimate_tokens(&rendered),
        budget.map(|b| format!(" budget={b}")).unwrap_or_default()
    );
    format!("{header}{rendered}")
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Sections {
    overview: Vec<String>,
    confidence: Vec<String>,
    priorities: Vec<String>,
    metrics: Vec<String>,
    next: Vec<String>,
    caveats: Vec<String>,
    tier_signal: Vec<String>,
}

fn build_sections(view: &ScanReportView) -> Sections {
    let mut overview = Vec::new();
    overview.push(format!("repo={}", view.repo_name));
    overview.push(format!("scan_id={}", view.scan_id));
    overview.push(format!("scanned_at={}", view.scanned_at_display));
    if let Some(branch) = &view.branch {
        let dirty = if view.dirty.unwrap_or(false) {
            " dirty"
        } else {
            ""
        };
        overview.push(format!("branch={branch}{dirty}"));
    }
    if let Some(commit) = &view.commit_short {
        overview.push(format!("commit={commit}"));
    }
    if !view.languages.is_empty() {
        overview.push(format!("languages={}", view.languages.join(",")));
    }
    let score = match view.score {
        Some(s) => format!("{s}/100"),
        None => "unknown".into(),
    };
    overview.push(format!(
        "score={score} band={}",
        view.score_band
            .label()
            .to_ascii_lowercase()
            .replace(' ', "_")
    ));

    let confidence = build_confidence(&view.confidence_notes);
    let priorities = build_priorities(&view.priorities);
    let metrics = build_metrics(&view.metrics);
    let next = view.next_commands.to_vec();
    let caveats = view.schema_caveats.clone();
    let tier_signal = build_tier_signal(&view.tier_signal);

    Sections {
        overview,
        confidence,
        priorities,
        metrics,
        next,
        caveats,
        tier_signal,
    }
}

fn build_tier_signal(signal: &crate::render::tier_signaling::TierSignal) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("free:".into());
    for item in &signal.free {
        lines.push(format!("  - {item}"));
    }
    lines.push("future_hosted:".into());
    for item in &signal.future_hosted {
        lines.push(format!("  - {item}"));
    }
    lines.push(signal.feedback_prompt.clone());
    lines
}

fn build_confidence(notes: &[ConfidenceNote]) -> Vec<String> {
    notes
        .iter()
        .map(|note| format!("{}: {}", note.level, note.metrics.join(", ")))
        .collect()
}

fn build_priorities(rows: &[PriorityRow]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let confidence = row
                .confidence
                .as_deref()
                .map(|c| format!(" (confidence={c})"))
                .unwrap_or_default();
            format!(
                "#{rank} [{severity}] {finding} :: {target} :: {evidence}{confidence} id={id}",
                rank = row.rank,
                severity = row.severity,
                finding = row.finding,
                target = row.target,
                evidence = row.evidence,
                confidence = confidence,
                id = row.finding_id,
            )
        })
        .collect()
}

fn build_metrics(rows: &[MetricRow]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let confidence = row
                .confidence
                .as_deref()
                .map(|c| format!(" confidence={c}"))
                .unwrap_or_default();
            format!(
                "{name} = {value} ({status}){confidence}",
                name = row.name,
                value = row.value,
                status = row.status,
                confidence = confidence,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Rendering and trimming
// ---------------------------------------------------------------------------

fn render_sections(sections: &Sections) -> String {
    let mut out = String::new();
    out.push_str("## overview\n");
    for line in &sections.overview {
        out.push_str(line);
        out.push('\n');
    }
    if !sections.confidence.is_empty() {
        out.push_str("\n## confidence\n");
        for line in &sections.confidence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str("\n## priorities\n");
    if sections.priorities.is_empty() {
        out.push_str("(none)\n");
    } else {
        for line in &sections.priorities {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str("\n## metrics\n");
    if sections.metrics.is_empty() {
        out.push_str("(none)\n");
    } else {
        for line in &sections.metrics {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str("\n## next\n");
    for line in &sections.next {
        out.push_str(line);
        out.push('\n');
    }
    if !sections.caveats.is_empty() {
        out.push_str("\n## caveats\n");
        for line in &sections.caveats {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !sections.tier_signal.is_empty() {
        out.push_str("\n## tier_signal\n");
        for line in &sections.tier_signal {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Trim the next least-essential row to fit a token budget. Returns
/// `false` when there is nothing left to trim — callers should stop
/// looping in that case.
fn trim_one(sections: &mut Sections) -> bool {
    // The tier_signal block is a marketing footer; a half-trimmed
    // footer is worse than no footer, so we drop the whole thing in
    // one step before touching anything substantive.
    if !sections.tier_signal.is_empty() {
        sections.tier_signal.clear();
        return true;
    }
    if !sections.caveats.is_empty() {
        sections.caveats.pop();
        return true;
    }
    if sections.metrics.len() > 3 {
        sections.metrics.pop();
        return true;
    }
    if sections.priorities.len() > 2 {
        sections.priorities.pop();
        return true;
    }
    if !sections.confidence.is_empty() {
        sections.confidence.pop();
        return true;
    }
    if sections.metrics.len() > 1 {
        sections.metrics.pop();
        return true;
    }
    if sections.priorities.len() > 1 {
        sections.priorities.pop();
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::view::{fixture_spec_example, MetricRow, PriorityRow, ScoreBand};

    fn view() -> ScanReportView {
        ScanReportView {
            repo_name: "demo".into(),
            root_path: "/tmp/demo".into(),
            scan_id: "scan_x".into(),
            scanned_at_display: "2026-05-17 11:12".into(),
            branch: Some("main".into()),
            commit_short: Some("abcdef1".into()),
            dirty: Some(false),
            languages: vec!["rust".into()],
            score: Some(80),
            score_band: ScoreBand::Good,
            confidence_notes: vec![ConfidenceNote {
                level: "high".into(),
                metrics: vec!["coupling".into(), "depth".into()],
            }],
            priorities: (1..=5)
                .map(|i| PriorityRow {
                    rank: i,
                    severity: "High".into(),
                    finding_id: format!("f{i}"),
                    finding: "Cycle".into(),
                    target: format!("mod::a{i}"),
                    evidence: "cycle of 3 nodes".into(),
                    confidence: Some("high".into()),
                })
                .collect(),
            metrics: (0..10)
                .map(|i| MetricRow {
                    name: format!("metric{i}"),
                    value: i.to_string(),
                    status: "OK".into(),
                    confidence: None,
                })
                .collect(),
            next_commands: vec!["gridseak explain f1".into()],
            schema_caveats: vec!["legacy_pre_orderfix".into()],
            // Fixed, small tier signal on purpose. The budget tests below
            // assert exactly which sections survive a byte-budget cut, and
            // tier_signal is the first thing trimmed. Using the production
            // `default_v0()` copy here couples those token-math assertions
            // to marketing wording, so an innocuous edit (e.g. "twelve" ->
            // "fourteen" tools) silently flips a trim boundary. This stable
            // fixture keeps the trim arithmetic independent of product copy.
            tier_signal: crate::render::tier_signaling::TierSignal {
                free: vec![
                    "feature one".into(),
                    "feature two".into(),
                    "feature three".into(),
                ],
                future_hosted: vec!["future one".into(), "future two".into()],
                feedback_prompt: "feedback prompt".into(),
            },
        }
    }

    #[test]
    fn unbounded_render_contains_all_sections() {
        let s = render_to_string(&view(), None);
        assert!(s.starts_with("[gridseak scan_report tokens≈"));
        assert!(s.contains("## overview"));
        assert!(s.contains("## confidence"));
        assert!(s.contains("## priorities"));
        assert!(s.contains("## metrics"));
        assert!(s.contains("## next"));
        assert!(s.contains("## caveats"));
    }

    #[test]
    fn budget_trims_tail_sections() {
        let unbounded = render_to_string(&view(), None);
        let unbounded_tokens = estimate_tokens(&unbounded);
        let tight = render_to_string(&view(), Some(unbounded_tokens.saturating_sub(40)));
        assert!(estimate_tokens(&tight) <= unbounded_tokens);
        // The tier_signal block is the lowest-priority section and the
        // largest by line count, so a small budget cut should drop it
        // before touching caveats or metrics.
        assert!(!tight.contains("## tier_signal"));
    }

    #[test]
    fn tight_budget_drops_tier_then_caveats() {
        let unbounded = render_to_string(&view(), None);
        let unbounded_tokens = estimate_tokens(&unbounded);
        // Cut deep enough that both tier_signal and caveats are pruned.
        let tight = render_to_string(&view(), Some(unbounded_tokens.saturating_sub(120)));
        assert!(!tight.contains("## tier_signal"));
        assert!(!tight.contains("## caveats"));
    }

    #[test]
    fn estimate_tokens_handles_empty_and_short() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abc"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    /// Snapshot lock for the spec-fixture LLM rendering at no budget.
    /// Pairs with `hero_llm_budget_snapshot` to show what trimming
    /// looks like under a constrained budget. Note that the first
    /// line carries the estimated token count, so any drift in
    /// content length flips that header — this is intentional and
    /// the snapshot's first line is the contract.
    #[test]
    fn hero_llm_snapshot() {
        let rendered = render_to_string(&fixture_spec_example(), None);
        insta::assert_snapshot!("hero_llm_unbounded", rendered);
    }

    #[test]
    fn hero_llm_budget_snapshot() {
        let rendered = render_to_string(&fixture_spec_example(), Some(180));
        insta::assert_snapshot!("hero_llm_budget_180", rendered);
    }
}
