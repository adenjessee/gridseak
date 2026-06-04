//! Django rule set.
//!
//! Covers the dispatch idioms Django enforces via convention rather
//! than an in-repo call edge:
//!
//! - **Class-based views**: `get`, `post`, `put`, `delete`, etc.
//!   called by `django.views.generic.View.dispatch`.
//! - **Management commands**: `Command.handle` /
//!   `Command.add_arguments` inside `.../management/commands/*.py`,
//!   invoked by `manage.py`.
//! - **Function-based views**: any function in `views.py` with no
//!   callers is almost certainly bound in a `urls.py` `path(...)`
//!   call that the current Python parser does not resolve through
//!   strings. Marked [`DeclarativeWiringUnparsed`] to keep the
//!   distinction from a framework *annotation* visible.
//! - **`urls.py`**: functions declared inside the URL conf itself
//!   (`def my_view(request): ...` inline with routes). Rare but
//!   exists in older Django codebases.

use crate::health::report::DeadCodeReason;

use super::super::{ClassifyContext, FrameworkRuleSet};

/// Class-based view HTTP method + lifecycle handlers.
const CBV_METHODS: &[&str] = &[
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "head",
    "options",
    "trace",
    "get_queryset",
    "get_object",
    "get_context_data",
    "form_valid",
    "form_invalid",
    "get_success_url",
];

/// Methods the `manage.py` runner invokes on a `Command` subclass.
const COMMAND_METHODS: &[&str] = &["handle", "add_arguments"];

pub struct DjangoRules;

impl FrameworkRuleSet for DjangoRules {
    fn framework(&self) -> &'static str {
        "django"
    }
    fn name(&self) -> &'static str {
        "python-django"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        let node = ctx.graph.nodes.get(ctx.node_id)?;
        let path = parent_path(ctx).unwrap_or_default();

        if is_management_command_file(&path) && COMMAND_METHODS.contains(&node.name.as_str()) {
            return Some((
                DeadCodeReason::DeclarativeWiringUnparsed,
                format!(
                    "fan_in={}; method '{}' on management command ({}); invoked by manage.py",
                    ctx.fan_in, node.name, path
                ),
            ));
        }

        if is_views_file(&path) && CBV_METHODS.contains(&node.name.as_str()) {
            return Some((
                DeadCodeReason::DeclarativeWiringUnparsed,
                format!(
                    "fan_in={}; method '{}' on views.py ({}); invoked by View.dispatch",
                    ctx.fan_in, node.name, path
                ),
            ));
        }

        if is_views_file(&path) && ctx.fan_in == 0 {
            return Some((
                DeadCodeReason::DeclarativeWiringUnparsed,
                format!(
                    "fan_in={}; function in views.py ({}); caller is likely a urls.py path() binding that the parser did not resolve",
                    ctx.fan_in, path
                ),
            ));
        }

        if is_urls_file(&path) {
            return Some((
                DeadCodeReason::DeclarativeWiringUnparsed,
                format!(
                    "fan_in={}; function inside urls.py ({}); URLconf routing is not statically resolved",
                    ctx.fan_in, path
                ),
            ));
        }

        None
    }
}

fn parent_path(ctx: &ClassifyContext<'_>) -> Option<String> {
    let p = ctx.graph.classification_of(ctx.node_id)?;
    p.path_repo_rel.clone().or_else(|| p.file_path.clone())
}

fn is_management_command_file(path: &str) -> bool {
    path.contains("/management/commands/")
}

fn is_views_file(path: &str) -> bool {
    path.ends_with("/views.py") || path.contains("/views/")
}

fn is_urls_file(path: &str) -> bool {
    path.ends_with("/urls.py") || path.ends_with("urls.py")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_py_is_detected() {
        assert!(is_urls_file("myapp/urls.py"));
        assert!(is_urls_file("urls.py"));
    }

    #[test]
    fn views_py_is_detected() {
        assert!(is_views_file("myapp/views.py"));
        assert!(is_views_file("myapp/views/account.py"));
    }

    #[test]
    fn management_command_file_is_detected() {
        assert!(is_management_command_file(
            "myapp/management/commands/sync.py"
        ));
    }
}
