//! Apex trigger-metadata extraction.
//!
//! The `structs` query routes `trigger_declaration` nodes to
//! `NodeKind::Struct` and captures the target SObject under `@sobject`
//! (already lifted into `Node.properties.sobject`). What it does NOT
//! capture is the list of **trigger events** — `before insert`,
//! `after update`, etc. — because those tokens are scattered across
//! multiple grammar nodes (`trigger_event` named children of
//! `trigger_declaration`) and packing them into the struct-level query
//! would tangle two extraction concerns.
//!
//! This helper runs the YAML-defined `trigger_events` query in
//! [configs/apex.yaml](../../../../configs/apex.yaml) scoped to a
//! single `trigger_declaration` node and returns the normalised event
//! strings. The caller (`symbol_extractor`) attaches the result as
//! `Node.properties.trigger_events`.
//!
//! Keeping the query string in YAML (rather than hard-coded here) is
//! deliberate: it preserves the single-source-of-truth invariant that
//! downstream LSP tooling, lint rules, and future grammar migrations
//! all read the same grammar shape from one place.

use tree_sitter::{Language, Node, Query, QueryCursor};

/// Extract the ordered list of trigger-event phrases (e.g.
/// `"before insert"`, `"after update"`) from a `trigger_declaration`
/// AST node.
///
/// Arguments:
/// - `trigger_node`: a tree-sitter `trigger_declaration` node. Called
///   from `symbol_extractor` when `is_trigger` is set.
/// - `source`: full file bytes the node was parsed from.
/// - `events_query`: the `trigger_events` query string from
///   `apex.yaml`, fetched via `LanguageConfig::get_query`. Passing it
///   in (rather than re-reading YAML here) keeps this helper pure and
///   keeps the YAML the single source of truth.
/// - `language`: tree-sitter language handle, required to compile the
///   query against the exact grammar that parsed `trigger_node`.
///
/// Returns events in source order with internal whitespace compacted
/// to single spaces (`"before   insert"` → `"before insert"`). An
/// invalid query or malformed source returns an empty vec — a missing
/// property is benign (the node is still present), whereas a panic
/// would take the whole scan down.
pub fn extract_events(
    trigger_node: &Node,
    source: &[u8],
    events_query: &str,
    language: Language,
) -> Vec<String> {
    let Ok(query) = Query::new(language, events_query) else {
        return Vec::new();
    };

    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let matches = cursor.matches(&query, *trigger_node, source);
    for mat in matches {
        for capture in mat.captures {
            let text = match capture.node.utf8_text(source) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let normalised: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalised.is_empty() {
                out.push(normalised);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    const EVENTS_QUERY: &str = "(trigger_event) @event";

    fn parse(src: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(tree_sitter_sfapex_vendored::apex::language())
            .expect("load apex grammar");
        parser.parse(src, None).expect("parse ok")
    }

    fn find_trigger<'a>(node: tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
        if node.kind() == "trigger_declaration" {
            return Some(node);
        }
        let mut w = node.walk();
        for child in node.named_children(&mut w) {
            if let Some(n) = find_trigger(child) {
                return Some(n);
            }
        }
        None
    }

    #[test]
    fn extracts_all_events_in_source_order() {
        let src = "trigger T on Account (before insert, after update, before delete) {}";
        let tree = parse(src);
        let trigger = find_trigger(tree.root_node()).unwrap();
        let events = extract_events(
            &trigger,
            src.as_bytes(),
            EVENTS_QUERY,
            tree_sitter_sfapex_vendored::apex::language(),
        );
        assert_eq!(
            events,
            vec!["before insert", "after update", "before delete"]
        );
    }

    #[test]
    fn compacts_internal_whitespace_to_single_spaces() {
        let src = "trigger T on Lead (before   insert) {}";
        let tree = parse(src);
        let trigger = find_trigger(tree.root_node()).unwrap();
        let events = extract_events(
            &trigger,
            src.as_bytes(),
            EVENTS_QUERY,
            tree_sitter_sfapex_vendored::apex::language(),
        );
        assert_eq!(events, vec!["before insert"]);
    }

    #[test]
    fn zero_events_yields_empty_vec() {
        // Malformed trigger declaration with no events — the query
        // matches nothing and we must return `Vec::new()`, not panic.
        let src = "trigger T on Contact () {}";
        let tree = parse(src);
        let trigger = find_trigger(tree.root_node()).unwrap();
        let events = extract_events(
            &trigger,
            src.as_bytes(),
            EVENTS_QUERY,
            tree_sitter_sfapex_vendored::apex::language(),
        );
        assert!(events.is_empty(), "expected no events, got {events:?}");
    }

    #[test]
    fn malformed_query_string_yields_empty_vec_not_panic() {
        let src = "trigger T on Account (before insert) {}";
        let tree = parse(src);
        let trigger = find_trigger(tree.root_node()).unwrap();
        let events = extract_events(
            &trigger,
            src.as_bytes(),
            "not a valid query",
            tree_sitter_sfapex_vendored::apex::language(),
        );
        assert!(events.is_empty());
    }
}
