//! Report card: impact of the current diff, refutations, invariants, disclosure.

use std::io::{self, Write};

use crate::graph_queries::agent_tools::{
    DiffImpactView, StructuralInvariantsView, VerifyClaimView,
};

pub fn render_report_card(
    impact: &DiffImpactView,
    claims: &[VerifyClaimView],
    invariants: &StructuralInvariantsView,
    out: &mut dyn Write,
) -> io::Result<()> {
    writeln!(out, "GridSeak report card")?;
    writeln!(out, "====================")?;
    writeln!(out)?;
    writeln!(out, "Changed symbols: {}", impact.changed_symbols.len())?;
    for row in &impact.affected {
        writeln!(
            out,
            "  {} → {} [{}] p_true={:.2}",
            row.symbol, row.affected, row.tier, row.p_true
        )?;
    }
    writeln!(out)?;
    writeln!(out, "Claims")?;
    if claims.is_empty() {
        writeln!(out, "  (none this session)")?;
    } else {
        for c in claims {
            writeln!(out, "  {:?} {}", c.verdict, c.claim)?;
        }
    }
    writeln!(out)?;
    writeln!(
        out,
        "Invariants: {}",
        if invariants.passed { "PASS" } else { "FAIL" }
    )?;
    for r in &invariants.results {
        writeln!(out, "  {} {}", if r.passed { "ok" } else { "fail" }, r.name)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_queries::agent_tools::{
        diff_impact_empty, structural_invariants_ok, ClaimVerdict, VerifyClaimView,
    };

    #[test]
    fn snapshot_empty_card() {
        let impact = diff_impact_empty(vec!["foo::bar".into()]);
        let claims = vec![VerifyClaimView {
            claim: "calls(A,B)".into(),
            verdict: ClaimVerdict::Refuted,
            witnesses: vec![],
            p_true: 0.88,
        }];
        let inv = structural_invariants_ok();
        let mut buf = Vec::new();
        render_report_card(&impact, &claims, &inv, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("Changed symbols: 1"));
        assert!(text.contains("Refuted") || text.contains("refuted"));
        assert!(text.contains("Invariants: PASS"));
    }
}
