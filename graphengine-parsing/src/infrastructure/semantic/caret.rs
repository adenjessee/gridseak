//! Shared caret placement for index-backed and rust-analyzer Layer-2 resolvers.
//!
//! Method calls `obj.method()` must land on `method`, not the receiver.

use crate::application::ports::CallSite;

/// 1-based line / 0-based character caret for a callee lookup.
pub fn caret_for_callee(call_site: &CallSite) -> (u32, u32) {
    if let Some(recv) = &call_site.receiver_range {
        if recv.end_line == call_site.location.end_line && recv.end_line == recv.start_line {
            return (recv.end_line, recv.end_char.saturating_add(1));
        }
    }
    let path = callee_path_name(&call_site.function_name);
    if path.contains("::") {
        return caret_for_qualified_path(call_site, path);
    }
    (call_site.location.start_line, call_site.location.start_char)
}

/// Strip extractor call-type prefixes (`method_call:`, etc.).
pub fn callee_path_name(function_name: &str) -> &str {
    function_name
        .strip_prefix("method_call:")
        .or_else(|| function_name.strip_prefix("constructor_call:"))
        .or_else(|| function_name.strip_prefix("chained_call:"))
        .unwrap_or(function_name)
}

/// Caret on the last path segment for `a::b::f` and multi-line
/// `a::b::\n    f` shapes.
pub fn caret_for_qualified_path(call_site: &CallSite, path: &str) -> (u32, u32) {
    let Some(last_colon) = path.rfind("::") else {
        return (call_site.location.start_line, call_site.location.start_char);
    };
    let prefix = &path[..last_colon + 2];
    let after_colons = &path[last_colon + 2..];
    let newlines_before = prefix.matches('\n').count() as u32;

    if after_colons.contains('\n') {
        let extra_lines = after_colons.matches('\n').count() as u32;
        let last_line = after_colons.lines().last().unwrap_or(after_colons);
        let leading = last_line.len() - last_line.trim_start().len();
        return (
            call_site.location.start_line + newlines_before + extra_lines,
            leading as u32,
        );
    }

    if newlines_before == 0 {
        let prefix_chars = prefix.chars().count() as u32;
        return (
            call_site.location.start_line,
            call_site.location.start_char + prefix_chars,
        );
    }

    let leading = after_colons.len() - after_colons.trim_start().len();
    (
        call_site.location.start_line + newlines_before,
        leading as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Range;

    fn site(name: &str, start_col: u32, recv_end_col: Option<u32>) -> CallSite {
        CallSite {
            location: Range {
                file: "a.ts".into(),
                start_line: 3,
                start_char: start_col,
                end_line: 3,
                end_char: start_col + 10,
            },
            function_name: name.into(),
            receiver_range: recv_end_col.map(|c| Range {
                file: "a.ts".into(),
                start_line: 3,
                start_char: 0,
                end_line: 3,
                end_char: c,
            }),
            receiver_text: None,
            arg_types: vec![],
        }
    }

    #[test]
    fn method_call_lands_after_dot() {
        let (line, col) = caret_for_callee(&site("method_call:parse", 0, Some(3)));
        assert_eq!(line, 3);
        assert_eq!(col, 4);
    }

    #[test]
    fn bare_ident_uses_start() {
        let (line, col) = caret_for_callee(&site("parse", 7, None));
        assert_eq!((line, col), (3, 7));
    }

    #[test]
    fn go_selector_lands_on_method_after_receiver() {
        // `chi.NewRouter()` — receiver `chi` cols 5-8, method starts at 9.
        let (line, col) = caret_for_callee(&site("method_call:NewRouter", 5, Some(8)));
        assert_eq!(line, 3);
        assert_eq!(col, 9);
    }
}
