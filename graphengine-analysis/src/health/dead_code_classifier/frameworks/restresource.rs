//! Apex `@RestResource` rule set.
//!
//! Universal entry-point-tag detection (see `super::universal`)
//! already catches `rest_resource` / `http_get` / `http_post` etc.
//! tags on individual methods. This rule set exists so the
//! verdict's `classifier` field accurately attributes a REST-file
//! dispatch to the framework-keyed rule stream rather than to the
//! universal pre-pass — that distinction matters for
//! `dead_code_classifier_breakdown` per-module coverage stats and
//! for the Wave 2 hand-audit.
//!
//! It also handles the edge case of an un-annotated helper method
//! sitting in a `@RestResource` class: such a method has no caller
//! in Apex code yet is logically part of the REST surface (often
//! invoked from an `@HttpGet`-annotated sibling via `this.helper()`
//! — an in-file edge our resolver does link when the receiver is
//! explicit but misses on bare-identifier calls). We deliberately
//! stop short of claiming those helpers as framework-invoked; the
//! Apex Framework Resolver (Wave 3) owns that resolution.

use crate::health::report::DeadCodeReason;

use super::super::{ClassifyContext, FrameworkRuleSet};

const REST_TAGS: &[&str] = &[
    "rest_resource",
    "http_get",
    "http_post",
    "http_put",
    "http_delete",
    "http_patch",
];

pub struct RestResourceRules;

impl FrameworkRuleSet for RestResourceRules {
    fn framework(&self) -> &'static str {
        "restresource"
    }
    fn name(&self) -> &'static str {
        "apex-restresource"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        let node = ctx.graph.nodes.get(ctx.node_id)?;
        let matched: Vec<&str> = node
            .entry_point_tags
            .iter()
            .filter(|t| REST_TAGS.contains(&t.as_str()))
            .map(|s| s.as_str())
            .collect();
        if !matched.is_empty() {
            return Some((
                DeadCodeReason::FrameworkAnnotationUnresolved,
                format!(
                    "fan_in={}; apex REST tags={:?}; invoked by Salesforce REST runtime",
                    ctx.fan_in, matched
                ),
            ));
        }
        None
    }
}
