//! Per-file framework detection.
//!
//! A "framework" is a stable short name (e.g. `"django"`, `"lwc"`,
//! `"tdtm"`) that routes a file's symbols to a framework-keyed
//! classifier rule set in the analysis layer. The dead-code
//! classifier looks up rules by *framework*, not by ecosystem, so
//! `GraphNode.frameworks` controls dispatch for polyglot repos
//! (NPSP Apex + LWC JavaScript + Python build scripts all sit in
//! the same graph but need different rules).
//!
//! # Design notes
//!
//! - Detection is additive: a single file can carry multiple
//!   frameworks (e.g. a REST-facing TDTM handler is both `tdtm`
//!   and `restresource`). The result is a deduplicated, sorted
//!   `Vec<String>` so downstream tests can assert on a stable
//!   ordering.
//! - Path detection runs before parsing, so it uses only the
//!   repo-relative path and language hint. This is fast, cheap,
//!   and deterministic.
//! - Content / symbol-tag detection is a second pass that runs
//!   after extractors produce per-symbol `entry_points` tags
//!   (e.g. `"rest_resource"`, `"aura_enabled"`). Those tags are
//!   folded into the parent `File`'s frameworks so the classifier
//!   does not have to re-discover them.
//! - When no framework rule fires, the file is tagged `"plain"`.
//!   This is intentional — "no framework" is a valid bucket that
//!   lets the classifier registry dispatch cleanly instead of
//!   falling through an empty match.
//!
//! # Why `Vec<String>` instead of an enum
//!
//! New frameworks are added frequently (every Salesforce
//! dispatch idiom, every Python task runner, every Rust web
//! framework). Baking the taxonomy into a Rust enum forces a
//! schema-breaking release whenever one is added (see R27 in
//! `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md`).
//! The analysis layer reads the strings as keys into a registry,
//! so adding a detector + a rule set is a non-breaking change.

use std::path::Path;

/// Framework-tag constants. Centralised so the parser, classifier,
/// and tests share a single source of truth for the string keys.
///
/// These are intentionally public `const &str`s rather than an
/// `enum` — see the module-level rationale.
pub mod tag {
    pub const DJANGO: &str = "django";
    pub const CELERY: &str = "celery";
    pub const LWC: &str = "lwc";
    /// Salesforce Aura components. Bundle layout mirrors LWC
    /// (`aura/<Component>/<Name>{Controller,Helper,Renderer}.js` +
    /// `.cmp` / `.app` / `.evt` markup). Detection is broad-
    /// segment by design — see the roadmap §13 "Framework-tag
    /// sizing rule" — so non-canonical helpers inside the
    /// bundle (`FormUtils.js`) are still tagged.
    pub const AURA: &str = "aura";
    pub const TDTM: &str = "tdtm";
    /// Apex `.trigger` files — DML-driven dispatch into trigger
    /// framework handlers.
    pub const TRIGGER_DML: &str = "triggerdml";
    pub const REST_RESOURCE: &str = "restresource";
    /// Jest test-runner harness files (`jest.setup.*`,
    /// `jest.config.*`). Top-level functions inside these files
    /// are invoked by the Jest runner during suite bootstrap, not
    /// by user code. Narrow file-name match by design — the
    /// `jest` tag means "this exact runner", which matters for
    /// per-runner rule divergence (see §13).
    pub const JEST: &str = "jest";
    /// Vitest test-runner harness files (`vitest.setup.*`,
    /// `vitest.config.*`). Shares the setup contract with Jest
    /// today but keeps a distinct tag so future runner-specific
    /// rules (e.g. `vi.mock` module-level side effects) and
    /// customer-facing attribution remain precise.
    pub const VITEST: &str = "vitest";
    /// No framework detected. Still present so classifier
    /// dispatch has a stable key to match on.
    pub const PLAIN: &str = "plain";
}

/// Path-based framework detection. Pure function over the
/// repo-relative path and language hint. Does not read file
/// contents.
///
/// Returned list is deduplicated and sorted ascending for
/// deterministic downstream comparisons. When no framework
/// rule fires, the returned list is `["plain"]`.
pub fn detect_frameworks_by_path(
    abs_path: &Path,
    workspace_root: Option<&Path>,
    language: Option<&str>,
) -> Vec<String> {
    let repo_rel =
        workspace_root.and_then(|root| super::classification::repo_relative_path(root, abs_path));
    let rel_lower = repo_rel.as_deref().unwrap_or("").to_ascii_lowercase();
    let file_name = abs_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let file_name_lower = file_name.to_ascii_lowercase();
    let lang = language.map(|s| s.to_ascii_lowercase());

    let mut frameworks: std::collections::BTreeSet<&'static str> =
        std::collections::BTreeSet::new();

    if is_lwc_path(&rel_lower) {
        frameworks.insert(tag::LWC);
    }
    if is_aura_path(&rel_lower) {
        frameworks.insert(tag::AURA);
    }
    if is_tdtm_path(&file_name_lower, lang.as_deref()) {
        frameworks.insert(tag::TDTM);
    }
    if is_apex_trigger_file(&file_name_lower) {
        frameworks.insert(tag::TRIGGER_DML);
    }
    if is_django_file(&file_name_lower, lang.as_deref()) {
        frameworks.insert(tag::DJANGO);
    }
    if is_celery_file(&file_name_lower, lang.as_deref()) {
        frameworks.insert(tag::CELERY);
    }
    if is_jest_harness_file(&file_name_lower) {
        frameworks.insert(tag::JEST);
    }
    if is_vitest_harness_file(&file_name_lower) {
        frameworks.insert(tag::VITEST);
    }

    if frameworks.is_empty() {
        frameworks.insert(tag::PLAIN);
    }

    frameworks.iter().map(|s| s.to_string()).collect()
}

/// Fold per-symbol `entry_points` tags into a File's framework
/// list. Called by the graph builder after all extractors have
/// produced their node property payload.
///
/// Example: NPSP's `REST_AccountManager.cls` has `entry_points:
/// ["rest_resource"]` on the class. Path detection already tagged
/// it as `plain`, but this pass promotes it to `restresource`
/// so the classifier dispatches the right rule set.
///
/// Tags that do not map to a framework (e.g. `"aura_enabled"`,
/// `"invocable_method"`) are handled by the classifier directly
/// via `entry_point_tags` on the node. This function only
/// promotes tags that correspond to a *file-scope* framework
/// (i.e. REST endpoints, TDTM handlers).
pub fn augment_frameworks_from_symbol_tags<I>(existing: Vec<String>, symbol_tags: I) -> Vec<String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut set: std::collections::BTreeSet<String> = existing.into_iter().collect();
    let mut promoted_any = false;

    for tag in symbol_tags {
        match tag.as_ref() {
            "rest_resource" | "http_get" | "http_post" | "http_put" | "http_delete"
            | "http_patch" => {
                set.insert(tag::REST_RESOURCE.to_string());
                promoted_any = true;
            }
            "tdtm_runnable" | "tdtm_handler" => {
                set.insert(tag::TDTM.to_string());
                promoted_any = true;
            }
            _ => {}
        }
    }

    // Only drop `plain` when we actually promoted a real
    // framework — otherwise `augment([plain], [aura_enabled])`
    // would incorrectly delete the plain tag and return an empty
    // list, breaking classifier dispatch for files whose symbols
    // carry only per-symbol entry-point signals.
    if promoted_any {
        set.remove(tag::PLAIN);
    }

    if set.is_empty() {
        set.insert(tag::PLAIN.to_string());
    }

    set.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Per-framework path heuristics
// ---------------------------------------------------------------------------

/// Salesforce Lightning Web Components live in an `lwc/<bundle>/...`
/// directory structure. Any file whose repo-relative path has an
/// `lwc` segment is part of an LWC bundle.
fn is_lwc_path(rel_lower: &str) -> bool {
    rel_lower.split('/').any(|segment| segment == "lwc")
}

/// Salesforce Aura components live in an `aura/<Component>/...`
/// bundle, analogous to LWC. Detection is a broad path-segment
/// match by design (see the roadmap §13 sizing rule and
/// `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md` R28):
/// the `aura/<Component>/` folder *is* the contract, so every file
/// inside — canonical `<Name>Controller.js` / `<Name>Helper.js` /
/// `<Name>Renderer.js`, the `.cmp` / `.app` / `.evt` markup, the
/// `.css` / `.design` / `.svg` / `.auradoc` siblings, and any
/// non-canonical `.js` helper (e.g. `FormUtils.js`) imported from
/// the canonical files — inherits the framework tag. The narrower
/// "only `*Controller.js` / `*Helper.js`" rule R28 initially
/// recommended would miss non-canonical helpers; broad-match is
/// the safer shape for the class of problem.
fn is_aura_path(rel_lower: &str) -> bool {
    rel_lower.split('/').any(|segment| segment == "aura")
}

/// Jest test-runner harness: root-level `jest.setup.{js,ts,mjs,cjs}`
/// or `jest.config.{js,ts,mjs,cjs}`. Top-level functions inside
/// these files are invoked by the Jest runner during suite
/// bootstrap (global setup, custom environment factories, etc.) —
/// they are entry points, not dead code. Match is by file *name*
/// (not full path) so the detector works identically for monorepo
/// packages that keep their own harness file one directory down.
fn is_jest_harness_file(file_name_lower: &str) -> bool {
    matches_harness_file(file_name_lower, "jest")
}

/// Vitest test-runner harness: `vitest.setup.{js,ts,mjs,cjs}` /
/// `vitest.config.{js,ts,mjs,cjs}`. Intentionally a distinct tag
/// from Jest — shared setup contract today, divergent runner
/// semantics (module mocking, snapshot layout) so future rules
/// benefit from the precise attribution.
fn is_vitest_harness_file(file_name_lower: &str) -> bool {
    matches_harness_file(file_name_lower, "vitest")
}

/// Shared body for the Jest / Vitest harness file detectors.
/// Matches `<runner>.setup.<ext>` and `<runner>.config.<ext>` for
/// the four ES module extensions the ecosystems use in practice.
fn matches_harness_file(file_name_lower: &str, runner_prefix: &str) -> bool {
    // `rsplit_once('.')` peels the final extension; the remaining
    // stem must be exactly `<runner>.setup` or `<runner>.config`.
    // Rejecting anything else keeps the rule narrow (no drift onto
    // `jest.utilities.js` or similar name-spoofing).
    let Some((stem, ext)) = file_name_lower.rsplit_once('.') else {
        return false;
    };
    if !matches!(ext, "js" | "ts" | "mjs" | "cjs") {
        return false;
    }
    stem == format!("{runner_prefix}.setup") || stem == format!("{runner_prefix}.config")
}

/// NPSP's Table-Driven Trigger Management convention: handler
/// classes have `TDTM` in the filename (e.g.
/// `CON_ContactMergeTDTM.cls`, `REL_Relationships_TDTM.cls`).
/// This is an Apex-only convention.
fn is_tdtm_path(file_name_lower: &str, lang: Option<&str>) -> bool {
    if !matches!(lang, Some("apex")) {
        return false;
    }

    // Strip the extension for a clean stem check. Apex class files
    // are `.cls`; ignore anything else.
    let Some((stem, ext)) = file_name_lower.rsplit_once('.') else {
        return false;
    };
    if ext != "cls" {
        return false;
    }

    // Match on word boundaries: `TDTM_` prefix, `_TDTM` suffix, or
    // standalone segment. Substring match is deliberately narrow —
    // a variable name `updateTDTM` in a non-handler file would
    // false-positive on a pure substring check.
    stem.starts_with("tdtm_")
        || stem.ends_with("_tdtm")
        || stem.contains("_tdtm_")
        // NPSP occasionally uses `FooTDTM.cls` (no underscore).
        // These are rare but real; the trailing-word form is
        // still safe because TDTM is a 4-char token that only
        // appears in NPSP handler names.
        || stem.ends_with("tdtm")
}

/// Apex triggers are files with a `.trigger` extension. These are
/// dispatched by the Salesforce platform on DML events, not by
/// explicit `Call` edges in Apex code.
fn is_apex_trigger_file(file_name_lower: &str) -> bool {
    file_name_lower.ends_with(".trigger")
}

/// Django has a stable file naming convention per app:
/// `views.py`, `urls.py`, `models.py`, `admin.py`, `forms.py`,
/// `apps.py`, `settings.py`, `middleware.py`. Matching on the
/// filename alone is coarse (a random `views.py` in a non-Django
/// repo will false-positive) but the classifier's generic
/// fallback already handles non-framework Python, so a false
/// framework tag does no harm — it just routes through the
/// Django rule set, which is a no-op for files that do not
/// actually dispatch through Django.
///
/// A stricter check (walk up the tree looking for `manage.py`)
/// is deferred to Wave 3 where the detector gains workspace
/// context.
fn is_django_file(file_name_lower: &str, lang: Option<&str>) -> bool {
    if !matches!(lang, Some("python")) {
        return false;
    }

    matches!(
        file_name_lower,
        "views.py"
            | "urls.py"
            | "models.py"
            | "admin.py"
            | "forms.py"
            | "apps.py"
            | "settings.py"
            | "middleware.py"
    )
}

/// Celery task modules are conventionally `tasks.py` or
/// `celery.py` / `celery_app.py`. `tasks.py` overlaps with other
/// background-job runners (RQ, Huey) — the same rule-set applies
/// in spirit ("the caller is a background-job dispatcher") so
/// tagging the file `celery` is harmless false-positive for
/// those. If a project uses multiple runners, the classifier
/// can be extended with dedicated tags.
fn is_celery_file(file_name_lower: &str, lang: Option<&str>) -> bool {
    if !matches!(lang, Some("python")) {
        return false;
    }

    matches!(file_name_lower, "tasks.py" | "celery.py" | "celery_app.py")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(path: &str) -> PathBuf {
        PathBuf::from(path)
    }

    // ----- plain fallback -----

    #[test]
    fn plain_fallback_when_nothing_matches() {
        let fws = detect_frameworks_by_path(
            &p("/repo/src/util/helpers.py"),
            Some(&p("/repo")),
            Some("python"),
        );
        assert_eq!(fws, vec![tag::PLAIN.to_string()]);
    }

    #[test]
    fn plain_fallback_for_rust_file() {
        let fws =
            detect_frameworks_by_path(&p("/repo/src/lib.rs"), Some(&p("/repo")), Some("rust"));
        assert_eq!(fws, vec![tag::PLAIN.to_string()]);
    }

    // ----- lwc -----

    #[test]
    fn lwc_detected_by_path_segment() {
        let fws = detect_frameworks_by_path(
            &p("/repo/force-app/main/default/lwc/greeting/greeting.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(fws.iter().any(|s| s == tag::LWC));
    }

    #[test]
    fn lwc_not_detected_when_lwc_is_substring_only() {
        // `/lwcomponent/` has `lwcomponent` as a segment, not `lwc`.
        let fws = detect_frameworks_by_path(
            &p("/repo/lwcomponent/greeting.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(!fws.iter().any(|s| s == tag::LWC));
    }

    #[test]
    fn lwc_detected_for_html_template() {
        let fws = detect_frameworks_by_path(
            &p("/repo/force-app/main/default/lwc/greeting/greeting.html"),
            Some(&p("/repo")),
            None,
        );
        assert!(fws.iter().any(|s| s == tag::LWC));
    }

    // ----- aura -----

    #[test]
    fn aura_detected_by_path_segment() {
        let fws = detect_frameworks_by_path(
            &p("/repo/force-app/main/default/aura/Hello/HelloController.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(fws.iter().any(|s| s == tag::AURA));
    }

    #[test]
    fn aura_detected_for_noncanonical_helper() {
        // The broad-match guarantee: a non-canonical JS helper
        // (not `<Name>Controller.js` / `<Name>Helper.js`) inside
        // the `aura/<Component>/` folder still gets tagged. A
        // narrow rule restricted to the two canonical filenames
        // would miss this file and re-introduce the R28 bug
        // shape (visibility_private_unused on a bundled helper).
        let fws = detect_frameworks_by_path(
            &p("/repo/force-app/main/default/aura/GE_GiftEntryForm/FormUtils.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(fws.iter().any(|s| s == tag::AURA));
    }

    #[test]
    fn aura_detected_for_cmp_template() {
        let fws = detect_frameworks_by_path(
            &p("/repo/force-app/main/default/aura/Hello/Hello.cmp"),
            Some(&p("/repo")),
            None,
        );
        assert!(fws.iter().any(|s| s == tag::AURA));
    }

    #[test]
    fn aura_not_detected_when_aura_is_substring_only() {
        // `/auralogs/` has `auralogs` as a segment, not `aura`.
        let fws = detect_frameworks_by_path(
            &p("/repo/src/auralogs/foo.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(!fws.iter().any(|s| s == tag::AURA));
    }

    // ----- jest -----

    #[test]
    fn jest_detected_for_setup_file() {
        let fws = detect_frameworks_by_path(
            &p("/repo/jest.setup.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(fws.iter().any(|s| s == tag::JEST));
    }

    #[test]
    fn jest_detected_for_config_file() {
        let fws = detect_frameworks_by_path(
            &p("/repo/jest.config.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(fws.iter().any(|s| s == tag::JEST));
    }

    #[test]
    fn jest_detected_for_ts_variant() {
        let fws = detect_frameworks_by_path(
            &p("/repo/jest.setup.ts"),
            Some(&p("/repo")),
            Some("typescript"),
        );
        assert!(fws.iter().any(|s| s == tag::JEST));
    }

    #[test]
    fn jest_detected_in_monorepo_package() {
        // Detection is name-based, not path-based, so a harness
        // file nested under a monorepo package directory still
        // matches.
        let fws = detect_frameworks_by_path(
            &p("/repo/packages/api/jest.setup.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(fws.iter().any(|s| s == tag::JEST));
    }

    #[test]
    fn non_harness_js_is_not_tagged_jest() {
        // A test file (`foo.test.js`) must not be tagged `jest`.
        // The runner invokes the test function via discovery,
        // not as a setup/config entry point, and the
        // `classify_path` test-file heuristic owns that case.
        let fws = detect_frameworks_by_path(
            &p("/repo/src/foo.test.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(!fws.iter().any(|s| s == tag::JEST));
    }

    #[test]
    fn name_spoofing_jest_prefix_is_not_tagged() {
        // `jest.utilities.js` is not a setup/config file and must
        // not inherit the tag on a loose prefix match.
        let fws = detect_frameworks_by_path(
            &p("/repo/jest.utilities.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(!fws.iter().any(|s| s == tag::JEST));
    }

    // ----- vitest -----

    #[test]
    fn vitest_detected_for_setup_file() {
        let fws = detect_frameworks_by_path(
            &p("/repo/vitest.setup.ts"),
            Some(&p("/repo")),
            Some("typescript"),
        );
        assert!(fws.iter().any(|s| s == tag::VITEST));
    }

    #[test]
    fn vitest_detected_for_config_file() {
        let fws = detect_frameworks_by_path(
            &p("/repo/vitest.config.ts"),
            Some(&p("/repo")),
            Some("typescript"),
        );
        assert!(fws.iter().any(|s| s == tag::VITEST));
    }

    #[test]
    fn jest_and_vitest_are_distinct_tags() {
        // A `jest.setup.js` must carry `jest` and must NOT carry
        // `vitest`; the reverse holds for a `vitest.setup.ts`.
        let jest = detect_frameworks_by_path(
            &p("/repo/jest.setup.js"),
            Some(&p("/repo")),
            Some("javascript"),
        );
        assert!(jest.iter().any(|s| s == tag::JEST));
        assert!(!jest.iter().any(|s| s == tag::VITEST));

        let vitest = detect_frameworks_by_path(
            &p("/repo/vitest.setup.ts"),
            Some(&p("/repo")),
            Some("typescript"),
        );
        assert!(vitest.iter().any(|s| s == tag::VITEST));
        assert!(!vitest.iter().any(|s| s == tag::JEST));
    }

    // ----- tdtm -----

    #[test]
    fn tdtm_detected_for_underscore_suffix() {
        let fws = detect_frameworks_by_path(
            &p("/repo/src/classes/CON_ContactMergeTDTM.cls"),
            Some(&p("/repo")),
            Some("apex"),
        );
        assert!(fws.iter().any(|s| s == tag::TDTM));
    }

    #[test]
    fn tdtm_detected_for_underscore_separated_suffix() {
        let fws = detect_frameworks_by_path(
            &p("/repo/src/classes/REL_Relationships_TDTM.cls"),
            Some(&p("/repo")),
            Some("apex"),
        );
        assert!(fws.iter().any(|s| s == tag::TDTM));
    }

    #[test]
    fn tdtm_detected_for_prefix() {
        let fws = detect_frameworks_by_path(
            &p("/repo/src/classes/TDTM_Config.cls"),
            Some(&p("/repo")),
            Some("apex"),
        );
        assert!(fws.iter().any(|s| s == tag::TDTM));
    }

    #[test]
    fn tdtm_not_detected_for_non_apex() {
        let fws = detect_frameworks_by_path(
            &p("/repo/src/my_tdtm_notes.py"),
            Some(&p("/repo")),
            Some("python"),
        );
        assert!(!fws.iter().any(|s| s == tag::TDTM));
    }

    #[test]
    fn tdtm_not_detected_for_unrelated_apex_class() {
        let fws = detect_frameworks_by_path(
            &p("/repo/src/classes/Utils.cls"),
            Some(&p("/repo")),
            Some("apex"),
        );
        assert!(!fws.iter().any(|s| s == tag::TDTM));
    }

    // ----- trigger_dml -----

    #[test]
    fn apex_trigger_file_is_tagged_triggerdml() {
        let fws = detect_frameworks_by_path(
            &p("/repo/src/triggers/AccountTrigger.trigger"),
            Some(&p("/repo")),
            Some("apex"),
        );
        assert!(fws.iter().any(|s| s == tag::TRIGGER_DML));
    }

    #[test]
    fn apex_class_file_is_not_triggerdml() {
        let fws = detect_frameworks_by_path(
            &p("/repo/src/classes/AccountTriggerHandler.cls"),
            Some(&p("/repo")),
            Some("apex"),
        );
        assert!(!fws.iter().any(|s| s == tag::TRIGGER_DML));
    }

    // ----- django -----

    #[test]
    fn django_detected_for_views_py() {
        let fws =
            detect_frameworks_by_path(&p("/repo/app/views.py"), Some(&p("/repo")), Some("python"));
        assert!(fws.iter().any(|s| s == tag::DJANGO));
    }

    #[test]
    fn django_detected_for_urls_py() {
        let fws =
            detect_frameworks_by_path(&p("/repo/app/urls.py"), Some(&p("/repo")), Some("python"));
        assert!(fws.iter().any(|s| s == tag::DJANGO));
    }

    #[test]
    fn django_not_detected_when_language_is_unknown() {
        let fws = detect_frameworks_by_path(&p("/repo/app/views.py"), Some(&p("/repo")), None);
        assert!(!fws.iter().any(|s| s == tag::DJANGO));
    }

    // ----- celery -----

    #[test]
    fn celery_detected_for_tasks_py() {
        let fws =
            detect_frameworks_by_path(&p("/repo/app/tasks.py"), Some(&p("/repo")), Some("python"));
        assert!(fws.iter().any(|s| s == tag::CELERY));
    }

    #[test]
    fn celery_detected_for_celery_py() {
        let fws = detect_frameworks_by_path(
            &p("/repo/project/celery.py"),
            Some(&p("/repo")),
            Some("python"),
        );
        assert!(fws.iter().any(|s| s == tag::CELERY));
    }

    // ----- multi-framework -----

    #[test]
    fn multiple_frameworks_are_returned_sorted_and_deduped() {
        // Hypothetical Apex file whose path includes "lwc" AND is a
        // TDTM class. Constructs the set to ensure dedup+sort work
        // end-to-end.
        let fws = detect_frameworks_by_path(
            &p("/repo/force-app/lwc/CON_FooTDTM.cls"),
            Some(&p("/repo")),
            Some("apex"),
        );
        assert_eq!(fws, vec![tag::LWC.to_string(), tag::TDTM.to_string()]);
    }

    // ----- augmentation -----

    #[test]
    fn augment_from_symbol_tags_promotes_rest_resource() {
        let out = augment_frameworks_from_symbol_tags(
            vec![tag::PLAIN.to_string()],
            &["rest_resource"][..],
        );
        assert_eq!(out, vec![tag::REST_RESOURCE.to_string()]);
    }

    #[test]
    fn augment_from_symbol_tags_preserves_existing_frameworks() {
        let out = augment_frameworks_from_symbol_tags(
            vec![tag::TDTM.to_string()],
            &["rest_resource"][..],
        );
        assert_eq!(
            out,
            vec![tag::REST_RESOURCE.to_string(), tag::TDTM.to_string()]
        );
    }

    #[test]
    fn augment_from_symbol_tags_drops_plain_when_framework_added() {
        let out = augment_frameworks_from_symbol_tags(
            vec![tag::PLAIN.to_string()],
            &["tdtm_runnable"][..],
        );
        assert_eq!(out, vec![tag::TDTM.to_string()]);
    }

    #[test]
    fn augment_from_symbol_tags_leaves_plain_when_no_relevant_tags() {
        // `aura_enabled` is per-symbol (handled by node.entry_point_tags),
        // not a file-scope framework. It must NOT promote the file.
        let out = augment_frameworks_from_symbol_tags(
            vec![tag::PLAIN.to_string()],
            &["aura_enabled"][..],
        );
        assert_eq!(out, vec![tag::PLAIN.to_string()]);
    }

    #[test]
    fn augment_from_symbol_tags_handles_http_verbs() {
        let out = augment_frameworks_from_symbol_tags(
            vec![tag::PLAIN.to_string()],
            &["http_get", "http_post"][..],
        );
        assert_eq!(out, vec![tag::REST_RESOURCE.to_string()]);
    }
}
