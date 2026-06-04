//! Celery rule set.
//!
//! `@task` / `@shared_task` / `@app.task` decorated functions are
//! invoked by the Celery worker runtime. When the Python parser
//! tags the symbol with `is_attribute_invoked=true`, the universal
//! pre-pass catches it before we get here. This rule set handles
//! the residual case: a function in `tasks.py` whose decorator the
//! parser failed to link (common when `tasks.py` uses
//! `from celery import shared_task` at module scope and the parser
//! can't statically tie it to the decorator chain). In that case
//! the file-scope `celery` tag on the parent file, plus fan-in 0,
//! is strong enough evidence.

use crate::health::report::DeadCodeReason;

use super::super::{ClassifyContext, FrameworkRuleSet};

pub struct CeleryRules;

impl FrameworkRuleSet for CeleryRules {
    fn framework(&self) -> &'static str {
        "celery"
    }
    fn name(&self) -> &'static str {
        "python-celery"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        let path = parent_path(ctx).unwrap_or_default();
        if is_tasks_file(&path) && ctx.fan_in == 0 {
            return Some((
                DeadCodeReason::FrameworkAnnotationUnresolved,
                format!(
                    "fan_in={}; function in tasks.py ({}); likely Celery @task / @shared_task not tagged by parser",
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

fn is_tasks_file(path: &str) -> bool {
    path.ends_with("/tasks.py") || path.contains("/tasks/")
}
