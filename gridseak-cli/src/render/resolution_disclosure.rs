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

/// One-line stderr warnings for languages whose compiler (SCIP) tier was
/// attempted but unusable, so heuristic-only call edges are never silent.
pub fn compiler_tier_warnings(rows: &[LanguageResolutionDisclosure]) -> Vec<String> {
    rows.iter()
        .filter(|d| d.compiler_tier_unavailable())
        .map(|d| {
            let reason = d.skip_reason.map(skip_reason_label).unwrap_or("unknown");
            let hint = match d.skip_reason {
                Some(ResolutionSkipReason::IndexStale) => {
                    "cached SCIP index is stale or from another checkout"
                }
                Some(ResolutionSkipReason::IndexerNotProvisioned) => "no SCIP index for this workspace",
                _ => "SCIP index could not be generated or loaded",
            };
            let edges = if d.tier_used == ResolutionDisclosureTier::Layer2 {
                "call edges come from the in-process rust-analyzer resolver (per call site, name-match fallback)"
            } else {
                "call edges are tree-sitter heuristics only"
            };
            format!(
                "warning: {lang}: compiler tier unavailable ({reason}: {hint}); {edges}. Run `gridseak doctor --provision {lang}` in this repo, or set GRIDSEAK_AUTO_PROVISION=1.",
                lang = d.language
            )
        })
        .collect()
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
    fn compiler_tier_warning_names_language_and_reason() {
        let rows = vec![
            LanguageResolutionDisclosure {
                language: "go".into(),
                tier_attempted: ResolutionDisclosureTier::Layer2,
                tier_used: ResolutionDisclosureTier::Heuristic,
                skip_reason: Some(ResolutionSkipReason::IndexStale),
                emitted_edges: None,
            },
            LanguageResolutionDisclosure {
                language: "rust".into(),
                tier_attempted: ResolutionDisclosureTier::Layer2,
                tier_used: ResolutionDisclosureTier::Layer2,
                skip_reason: None,
                emitted_edges: Some(3),
            },
        ];
        let warnings = compiler_tier_warnings(&rows);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("warning: go: compiler tier unavailable (index_stale"));
        assert!(warnings[0].contains("gridseak doctor --provision go"));
    }

    #[test]
    fn rust_fallback_to_ra_ide_warns_without_claiming_heuristics() {
        let rows = vec![LanguageResolutionDisclosure {
            language: "rust".into(),
            tier_attempted: ResolutionDisclosureTier::Layer2,
            tier_used: ResolutionDisclosureTier::Layer2,
            skip_reason: Some(ResolutionSkipReason::IndexerNotProvisioned),
            emitted_edges: Some(7),
        }];
        let warnings = compiler_tier_warnings(&rows);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("in-process rust-analyzer resolver"));
        assert!(warnings[0].contains("gridseak doctor --provision rust"));
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
