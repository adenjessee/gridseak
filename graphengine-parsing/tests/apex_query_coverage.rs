//! Sprint E.4 — Apex query coverage meta-test.
//!
//! Every top-level query block in `configs/apex.yaml` MUST be bound
//! to at least one production-path consumer. The trail of evidence:
//!
//! 1. The YAML parser (`infrastructure::config::LanguageConfig`)
//!    exposes queries through `get_query(name)`.
//! 2. Any extractor that actually consumes a query calls
//!    `config.get_query("foo")` somewhere in `src/`.
//!
//! A query that lives in YAML but is never fetched via `get_query` is
//! dead code — the grammar shape it documents never reaches the
//! pipeline, and nothing prevents it from silently drifting out of
//! sync with the rest of the extractor. The meta-test below walks the
//! apex YAML, lists every query name, and scans all Rust source files
//! under `graphengine-parsing/src/` for a matching `get_query("...")`
//! binding. The test fails loudly if any query is orphaned.
//!
//! When this test fires on a new query, the fix is usually one of:
//! - wire the query into an extractor path (preferred), or
//! - remove the unused YAML block (if the query is truly obsolete).
//!
//! Letting an exception slip in silently is explicitly disallowed.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Walk `src/` recursively and collect every `get_query("NAME")` call.
fn collect_get_query_bindings() -> BTreeSet<String> {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = BTreeSet::new();
    walk(&src_root, &mut |path| {
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            return;
        }
        let Ok(contents) = fs::read_to_string(path) else {
            return;
        };
        // Simple, robust scan: look for `get_query("` and extract the
        // quoted identifier. Regex would pull in a dev dep for what is
        // effectively a handful of substring splits.
        for (idx, _) in contents.match_indices("get_query(\"") {
            let after = &contents[idx + "get_query(\"".len()..];
            if let Some(end) = after.find('"') {
                let name = &after[..end];
                if !name.is_empty() {
                    found.insert(name.to_string());
                }
            }
        }
    });
    found
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, f);
        } else {
            f(&path);
        }
    }
}

/// Load `configs/apex.yaml` and return every top-level query name.
fn apex_query_names() -> Vec<String> {
    let yaml_path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("configs")
        .join("apex.yaml");
    let yaml = fs::read_to_string(&yaml_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", yaml_path.display()));
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&yaml).expect("apex.yaml must be valid YAML");
    let queries = parsed
        .get("queries")
        .and_then(|v| v.as_mapping())
        .expect("apex.yaml must have top-level `queries` mapping");
    queries
        .iter()
        .filter_map(|(k, _)| k.as_str().map(String::from))
        .collect()
}

#[test]
fn every_apex_yaml_query_is_bound_to_an_extractor() {
    let declared = apex_query_names();
    assert!(
        !declared.is_empty(),
        "apex.yaml declared zero queries; file likely malformed"
    );

    let bindings = collect_get_query_bindings();

    let mut orphaned: Vec<&str> = declared
        .iter()
        .filter(|name| !bindings.contains(name.as_str()))
        .map(|s| s.as_str())
        .collect();
    orphaned.sort_unstable();

    assert!(
        orphaned.is_empty(),
        "Orphaned apex.yaml queries (declared in YAML but never fetched via `get_query` \
         anywhere under graphengine-parsing/src/): {orphaned:?}. \
         Fix by wiring the query into an extractor path, or by removing the dead YAML \
         block. Known `get_query` bindings found: {:?}",
        bindings
    );
}

#[test]
fn apex_yaml_query_names_are_lowercase_snake_case() {
    // Stability guard: query names are a public contract between YAML
    // and Rust. A typo like `Trigger_Events` breaks silently because
    // `get_query` would simply return `None`. Pin the convention.
    for name in apex_query_names() {
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "apex.yaml query name `{name}` must be lowercase snake_case"
        );
        assert!(
            !name.starts_with('_') && !name.ends_with('_'),
            "apex.yaml query name `{name}` must not start or end with `_`"
        );
    }
}
