//! Sticky PR comment body (impact table, unverified claims, test gaps).

use std::io::{self, Write};

use crate::graph_queries::agent_tools::{
    DiffImpactView, StructuralInvariantsView, VerifyClaimView,
};

pub fn render_pr_comment(
    impact: &DiffImpactView,
    claims: &[VerifyClaimView],
    invariants: &StructuralInvariantsView,
    out: &mut dyn Write,
) -> io::Result<()> {
    writeln!(out, "<!-- gridseak-pr-comment -->")?;
    writeln!(out, "## GridSeak structural review")?;
    writeln!(out)?;
    writeln!(out, "### Impact")?;
    writeln!(out, "| Symbol | Affected | Tier | p_true |")?;
    writeln!(out, "|---|---|---|---|")?;
    if impact.affected.is_empty() {
        writeln!(out, "| _(none computed)_ | | | |")?;
    } else {
        for row in &impact.affected {
            writeln!(
                out,
                "| `{}` | `{}` | {} | {:.2} |",
                row.symbol, row.affected, row.tier, row.p_true
            )?;
        }
    }
    writeln!(out)?;
    writeln!(out, "### Claims")?;
    for c in claims {
        writeln!(out, "- **{:?}** `{}`", c.verdict, c.claim)?;
    }
    if claims.is_empty() {
        writeln!(out, "- _(no session claims)_")?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "### Invariants: {}",
        if invariants.passed { "PASS" } else { "FAIL" }
    )?;
    for r in &invariants.results {
        writeln!(
            out,
            "- {} `{}` — {}",
            if r.passed { "✅" } else { "❌" },
            r.name,
            r.detail
        )?;
    }
    Ok(())
}
