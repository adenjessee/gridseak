use anyhow::bail;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNodeFilter {
    pub kinds: Option<Vec<String>>, // normalized kinds
    pub role: Option<Vec<String>>,  // role == or role in
    pub is_vendor: Option<bool>,
    pub is_build_output: Option<bool>,
    pub is_generated: Option<bool>,
    pub is_test: Option<bool>,
    pub path_repo_rel_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEdgeFilter {
    pub rel_kinds: Option<Vec<String>>, // normalized edge kinds
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterOp {
    Eq,
    In,
    StartsWith,
}

/// Contract v1 filter parsing/validation.
///
/// We allow a limited `and` conjunction of supported clauses because:
/// - existing templates in-repo use it
/// - it remains deterministic and AI-friendly
///   Anything else (OR/NOT/parens/unknown keys/operators) is rejected as a hard error.
pub fn parse_node_filter_v1(raw: &str) -> anyhow::Result<ParsedNodeFilter> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("node_filter is empty");
    }
    reject_boolean_complexity(raw, "node_filter")?;

    let mut out = ParsedNodeFilter {
        kinds: None,
        role: None,
        is_vendor: None,
        is_build_output: None,
        is_generated: None,
        is_test: None,
        path_repo_rel_prefix: None,
    };

    for clause in split_top_level_and(raw) {
        let c = clause.trim();
        if c.is_empty() {
            continue;
        }

        // node_type + legacy aliases (label/kind)
        if let Some(v) = try_parse_in_list(c, "node_type")
            .or_else(|| try_parse_in_list(c, "label"))
            .or_else(|| try_parse_in_list(c, "kind"))
        {
            let normalized: Vec<String> = v
                .into_iter()
                .filter_map(|s| normalize_node_kind(&s).map(|k| k.to_string()))
                .collect();
            if normalized.is_empty() {
                bail!("node_filter: node_type list contains no recognized node kinds");
            }
            merge_vec_opt(&mut out.kinds, normalized);
            continue;
        }
        if let Some(v) = try_parse_eq_quoted(c, "node_type")
            .or_else(|| try_parse_eq_quoted(c, "label"))
            .or_else(|| try_parse_eq_quoted(c, "kind"))
        {
            let kind = normalize_node_kind(&v)
                .ok_or_else(|| anyhow::anyhow!("node_filter: unknown node kind '{v}'"))?
                .to_string();
            merge_vec_opt(&mut out.kinds, vec![kind]);
            continue;
        }

        // role
        if let Some(v) = try_parse_eq_quoted(c, "role") {
            merge_vec_opt(&mut out.role, vec![v]);
            continue;
        }
        if let Some(v) = try_parse_in_list(c, "role") {
            if v.is_empty() {
                bail!("node_filter: role in [] is empty");
            }
            merge_vec_opt(&mut out.role, v);
            continue;
        }

        // boolean flags
        for key in ["is_vendor", "is_build_output", "is_generated", "is_test"] {
            if let Some(v) = try_parse_eq_bool(c, key)? {
                match key {
                    "is_vendor" => out.is_vendor = Some(v),
                    "is_build_output" => out.is_build_output = Some(v),
                    "is_generated" => out.is_generated = Some(v),
                    "is_test" => out.is_test = Some(v),
                    _ => {}
                }
                continue;
            }
        }

        // path_repo_rel starts_with 'src/'
        if let Some(prefix) = try_parse_starts_with_quoted(c, "path_repo_rel") {
            out.path_repo_rel_prefix = Some(prefix);
            continue;
        }

        bail!("node_filter: unsupported clause '{c}'");
    }

    Ok(out)
}

pub fn parse_edge_filter_v1(raw: &str) -> anyhow::Result<ParsedEdgeFilter> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("edge_filter is empty");
    }
    reject_boolean_complexity(raw, "edge_filter")?;

    let mut out = ParsedEdgeFilter { rel_kinds: None };
    for clause in split_top_level_and(raw) {
        let c = clause.trim();
        if c.is_empty() {
            continue;
        }

        if let Some(v) = try_parse_eq_quoted(c, "rel") {
            let kind = normalize_edge_kind(&v)
                .ok_or_else(|| anyhow::anyhow!("edge_filter: unknown edge kind '{v}'"))?
                .to_string();
            merge_vec_opt(&mut out.rel_kinds, vec![kind]);
            continue;
        }
        if let Some(v) = try_parse_in_list(c, "rel") {
            let normalized: Vec<String> = v
                .into_iter()
                .filter_map(|r| normalize_edge_kind(&r).map(|k| k.to_string()))
                .collect();
            if normalized.is_empty() {
                bail!("edge_filter: rel list contains no recognized edge kinds");
            }
            merge_vec_opt(&mut out.rel_kinds, normalized);
            continue;
        }

        bail!("edge_filter: unsupported clause '{c}'");
    }

    Ok(out)
}

fn merge_vec_opt(dst: &mut Option<Vec<String>>, mut new_values: Vec<String>) {
    match dst {
        None => *dst = Some(new_values),
        Some(existing) => {
            existing.append(&mut new_values);
            existing.sort();
            existing.dedup();
        }
    }
}

fn reject_boolean_complexity(raw: &str, field: &str) -> anyhow::Result<()> {
    // Reject OR/NOT/parens. Allow AND at top-level only (handled by split_top_level_and).
    let lowered = raw.to_ascii_lowercase();
    for needle in [" or ", "||", " not ", "!", "(", ")"] {
        if lowered.contains(needle) {
            bail!("{field}: unsupported boolean logic (only 'and' is allowed in v1)");
        }
    }
    Ok(())
}

fn split_top_level_and(input: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => {}
        }

        if !in_single && !in_double {
            // match " and " (case-insensitive) without allocating lowercased full string repeatedly
            if i + 5 <= chars.len() {
                let slice: String = chars[i..(i + 5)].iter().collect();
                if slice.eq_ignore_ascii_case(" and ") {
                    parts.push(buf.trim().to_string());
                    buf.clear();
                    i += 5;
                    continue;
                }
            }
        }

        buf.push(c);
        i += 1;
    }
    if !buf.trim().is_empty() {
        parts.push(buf.trim().to_string());
    }
    parts
}

fn try_parse_eq_bool(clause: &str, key: &str) -> anyhow::Result<Option<bool>> {
    let c = clause.trim();
    let needle = format!("{key} ==");
    if !c.contains(&needle) {
        return Ok(None);
    }
    let rhs = c.split(&needle).nth(1).map(|s| s.trim()).unwrap_or("");
    if rhs.eq_ignore_ascii_case("true") {
        Ok(Some(true))
    } else if rhs.eq_ignore_ascii_case("false") {
        Ok(Some(false))
    } else {
        bail!("node_filter: expected boolean for {key}, got '{rhs}'");
    }
}

fn try_parse_eq_quoted(clause: &str, key: &str) -> Option<String> {
    let needle = format!("{key} ==");
    let rhs = clause.split(&needle).nth(1)?;
    extract_quoted_value(rhs)
}

fn try_parse_starts_with_quoted(clause: &str, key: &str) -> Option<String> {
    // "path_repo_rel starts_with 'src/'"
    let needle = format!("{key} starts_with");
    let rhs = clause.split(&needle).nth(1)?;
    extract_quoted_value(rhs)
}

fn try_parse_in_list(clause: &str, key: &str) -> Option<Vec<String>> {
    // `key in ['a','b']`
    let needle = format!("{key} in [");
    let start = clause.find(&needle)?;
    let after = &clause[start + needle.len()..];
    let end = after.find(']')?;
    let inner = &after[..end];
    let items = inner
        .split(',')
        .map(|s| s.trim().trim_matches('\'').trim_matches('"'))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    Some(items)
}

fn extract_quoted_value(input: &str) -> Option<String> {
    // Accept either single or double quotes.
    let s = input.trim();
    if let Some(start) = s.find('\'') {
        if let Some(end) = s[start + 1..].find('\'') {
            return Some(s[start + 1..start + 1 + end].to_string());
        }
    }
    if let Some(start) = s.find('"') {
        if let Some(end) = s[start + 1..].find('"') {
            return Some(s[start + 1..start + 1 + end].to_string());
        }
    }
    None
}

fn normalize_node_kind(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "Function" => Some("Function"),
        "Method" => Some("Function"),
        "Struct" => Some("Struct"),
        "Module" => Some("Module"),
        "Interface" => Some("Interface"),
        "Enum" => Some("Enum"),
        "Variable" => Some("Variable"),
        "Type" => Some("Type"),
        "Import" => Some("Import"),
        "Project" => Some("Project"),
        "Crate" => Some("Crate"),
        "File" => Some("File"),
        "Folder" => Some("Folder"),
        "Trait" => Some("Interface"),
        "Impl" => None,
        other => match other.to_ascii_lowercase().as_str() {
            "function" => Some("Function"),
            "method" => Some("Function"),
            "struct" => Some("Struct"),
            "module" => Some("Module"),
            "interface" => Some("Interface"),
            "trait" => Some("Interface"),
            "enum" => Some("Enum"),
            "variable" => Some("Variable"),
            "type" => Some("Type"),
            "import" => Some("Import"),
            "project" => Some("Project"),
            "crate" => Some("Crate"),
            "file" => Some("File"),
            "folder" => Some("Folder"),
            _ => None,
        },
    }
}

fn normalize_edge_kind(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "Call" => Some("Call"),
        "Contains" => Some("Contains"),
        "Import" => Some("Import"),
        "Type" => Some("Type"),
        "Uses" => Some("Uses"),
        "Calls" => Some("Call"),
        "Imports" => Some("Import"),
        "Types" => Some("Type"),
        other => match other.to_ascii_lowercase().as_str() {
            "call" | "calls" => Some("Call"),
            "contains" => Some("Contains"),
            "import" | "imports" => Some("Import"),
            "type" | "types" => Some("Type"),
            "uses" => Some("Uses"),
            _ => None,
        },
    }
}
