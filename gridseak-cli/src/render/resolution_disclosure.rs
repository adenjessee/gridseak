//! Per-language resolution disclosure lines for scan output.

use std::io::{self, Write};

use graphengine_analysis::health::report::{
    LanguageResolutionDisclosure, ResolutionDisclosureTier, ResolutionSkipReason,
};

pub fn render_resolution_disclosure_lines(
    report: &graphengine_analysis::health::report::HealthReport,
    out: &mut dyn Write,
) -> io::Result<()> {
    let Some(rq) = report.resolution_quality.as_ref() else {
        return Ok(());
    };
    if rq.resolution_disclosure.is_empty() {
        return Ok(());
    }
    for row in &rq.resolution_disclosure {
        writeln!(out, "{}", format_disclosure_line(row))?;
    }
    Ok(())
}

pub fn format_disclosure_line(d: &LanguageResolutionDisclosure) -> String {
    let tier_label = match d.tier_used {
        ResolutionDisclosureTier::Layer2 => "layer2",
        ResolutionDisclosureTier::SubprocessLsp => "subprocess_lsp",
        ResolutionDisclosureTier::Heuristic => "heuristic only",
    };
    let mut line = format!("{}: {}", d.language, tier_label);
    if let Some(n) = d.emitted_edges {
        line.push_str(&format!(" ({n} emitted edges)"));
    }
    if let Some(reason) = d.skip_reason {
        line.push_str(&format!(" (skip: {})", skip_reason_label(reason)));
    }
    line
}

fn skip_reason_label(reason: ResolutionSkipReason) -> &'static str {
    match reason {
        ResolutionSkipReason::AdapterInitFailed => "adapter_init_failed",
        ResolutionSkipReason::ServerMissing => "server_missing",
        ResolutionSkipReason::LanguageNotRouted => "language_not_routed",
        ResolutionSkipReason::NoReferences => "no_references",
        ResolutionSkipReason::PolicyDisabled => "policy_disabled",
        ResolutionSkipReason::IndexerFailed => "indexer_failed",
        ResolutionSkipReason::IndexerNotProvisioned => "indexer_not_provisioned",
        ResolutionSkipReason::IndexStale => "index_stale",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_layer2_and_heuristic_lines() {
        let disclosure = vec![
            LanguageResolutionDisclosure {
                language: "rust".into(),
                tier_attempted: ResolutionDisclosureTier::Layer2,
                tier_used: ResolutionDisclosureTier::Layer2,
                skip_reason: None,
                emitted_edges: Some(42),
            },
            LanguageResolutionDisclosure {
                language: "python".into(),
                tier_attempted: ResolutionDisclosureTier::SubprocessLsp,
                tier_used: ResolutionDisclosureTier::Heuristic,
                skip_reason: Some(ResolutionSkipReason::ServerMissing),
                emitted_edges: None,
            },
        ];
        let mut buf = Vec::new();
        for row in &disclosure {
            writeln!(buf, "{}", format_disclosure_line(row)).unwrap();
        }
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("rust: layer2 (42 emitted edges)"));
        assert!(text.contains("python: heuristic only (skip: server_missing)"));
    }

    #[test]
    fn formats_indexer_failed_skip_reason() {
        let row = LanguageResolutionDisclosure {
            language: "typescript".into(),
            tier_attempted: ResolutionDisclosureTier::Layer2,
            tier_used: ResolutionDisclosureTier::Heuristic,
            skip_reason: Some(ResolutionSkipReason::IndexerFailed),
            emitted_edges: None,
        };
        let line = format_disclosure_line(&row);
        assert!(line.contains("typescript: heuristic only (skip: indexer_failed)"));
    }
}
