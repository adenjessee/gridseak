use anyhow::bail;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedRoot {
    ById(String),
    ByPathRepoRel(String),
    /// SQL LIKE match against the `fqn` column.
    /// The pattern must include its own wildcards (e.g. `"%handlePayment%"`).
    ByFqnLike(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSeeds {
    pub roots: Vec<SeedRoot>,
}

pub fn parse_seeds_v1(seed_spec: &toml::Value) -> anyhow::Result<ParsedSeeds> {
    // Preferred v1: seed.roots = [{by_id="..."}, {by_path_repo_rel="src"}]
    if let Some(roots) = seed_spec.get("roots").and_then(|v| v.as_array()) {
        let mut out: Vec<SeedRoot> = Vec::new();
        for root in roots {
            let t = root
                .as_table()
                .ok_or_else(|| anyhow::anyhow!("seed.roots entries must be TOML tables"))?;

            if let Some(id) = t.get("by_id").and_then(|v| v.as_str()) {
                out.push(SeedRoot::ById(id.to_string()));
                continue;
            }
            if let Some(p) = t.get("by_path_repo_rel").and_then(|v| v.as_str()) {
                out.push(SeedRoot::ByPathRepoRel(p.to_string()));
                continue;
            }
            if let Some(pattern) = t.get("by_fqn_like").and_then(|v| v.as_str()) {
                out.push(SeedRoot::ByFqnLike(pattern.to_string()));
                continue;
            }

            bail!("seed.roots entry must include one of: by_id, by_path_repo_rel, by_fqn_like");
        }
        if out.is_empty() {
            bail!("seed.roots must contain at least one entry");
        }
        return Ok(ParsedSeeds { roots: out });
    }

    bail!("Traversal mode requires seed.roots = [...]");
}
