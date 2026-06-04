//! Small helpers shared across the renderer family.
//!
//! Anything that is fundamentally "string formatting for the terminal
//! / markdown / LLM surface" — and that has tempted multiple
//! `render/*.rs` files to grow their own private copy — belongs here.
//! Centralising these primitives prevents the kind of drift we shipped
//! by accident before R4: four `md_escape` definitions with subtly
//! different escape sets would have rendered the same cell three
//! different ways depending on which command produced it.

/// Markdown table-cell escape.
///
/// We only escape the two characters that actively break Markdown
/// table rendering:
///
/// - `|` would terminate the cell mid-string.
/// - `` ` `` would open or close an inline code span that the cell
///   format does not expect to be parsed.
///
/// We deliberately do **not** escape other Markdown control characters
/// (`*`, `_`, `#`, etc.). Cell contents are not parsed as full
/// paragraphs by any of the renderer call-sites; escaping them would
/// just make the output noisy when a function name happens to contain
/// an underscore. The four pre-R4 copies of this function all agreed
/// on this contract — the consolidation here pins it as a single
/// source of truth so future renderers cannot drift.
pub fn md_escape(value: &str) -> String {
    value.replace('|', "\\|").replace('`', "\\`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_unrelated_characters() {
        assert_eq!(md_escape("plain_name"), "plain_name");
        assert_eq!(md_escape("module::Type"), "module::Type");
        assert_eq!(md_escape("normal sentence."), "normal sentence.");
    }

    #[test]
    fn escapes_pipe_and_backtick() {
        assert_eq!(md_escape("a|b"), "a\\|b");
        assert_eq!(md_escape("`code`"), "\\`code\\`");
        assert_eq!(md_escape("a | `b`"), "a \\| \\`b\\`");
    }

    #[test]
    fn leaves_other_markdown_metacharacters_alone() {
        // Underscores in identifiers are common; double-escaping them
        // would visibly garble every function name in the table.
        assert_eq!(md_escape("snake_case_fn"), "snake_case_fn");
        assert_eq!(md_escape("*emphasis*"), "*emphasis*");
        assert_eq!(md_escape("# header"), "# header");
    }
}
