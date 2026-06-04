//! Wave 2.4: polyglot-repo integration fixture for the
//! framework-keyed dead-code classifier.
//!
//! This test proves that a single analysis run over a graph that
//! mixes **Apex** (NPSP-style TDTM handlers), **Python** (Django
//! class-based views), and **Lightning Web Components** (LWC JS
//! methods) routes each node to its framework's own rule set —
//! NOT to a single repo-level ecosystem bucket.
//!
//! Note (A3 follow-up, 2026-05): the `properties.language` on the
//! Project node is no longer authoritative — the parser stopped
//! writing it because the polyglot orchestrator's
//! `INSERT OR REPLACE` was clobbering it on every pass. The
//! analyzer now derives canonical primary language from File-node
//! majority (see `tests/primary_language_detection.rs`). This
//! fixture's framework-dispatch assertions are independent of that
//! field and were already correct, but if you copy this file as a
//! starting point for new tests, remember: dispatch is per-node
//! via `GraphNode.frameworks`, never via a single project-level
//! language label.
//!
//! Failure mode this guards against (pre-Wave-2 behaviour):
//! `cfg.resolved_ecosystem()` is a single enum value like
//! `Ecosystem::Apex`, so every dead node in an NPSP checkout was
//! classified through `ApexClassifier`. The Python and LWC files
//! silently landed in `NoCallers`/`VisibilityPrivateUnused` buckets
//! because the Apex rules didn't match them — a false-negative on
//! those languages and a misattribution on the Apex verdicts.
//!
//! Wave 2 replaces the ecosystem-keyed dispatch with per-node
//! `GraphNode.frameworks` dispatch. Polyglot repos now route
//! correctly on a per-node basis.
//!
//! # Fixture shape
//!
//! ```text
//! project/
//!   force-app/main/default/classes/TDTM_Opportunity.cls   (apex, tdtm)
//!     └── run()                        — expected: DynamicDispatchTarget (apex-tdtm)
//!   force-app/main/default/classes/OpportunityService.cls (apex, plain)
//!     └── recalculate()                — expected: NoCallers (generic)
//!   force-app/main/default/classes/AccountRest.cls        (apex, restresource)
//!     └── doGet()  [http_get tag]      — expected: FrameworkAnnotationUnresolved (universal tag)
//!   force-app/main/default/classes/AuraController.cls     (apex, plain)
//!     └── getData() [aura_enabled tag] — expected: FrameworkAnnotationUnresolved (universal tag)
//!   django/app/views.py                                   (python, django)
//!     └── get()                        — expected: DeclarativeWiringUnparsed (python-django)
//!   django/app/tasks.py                                   (python, celery)
//!     └── process_job()                — expected: FrameworkAnnotationUnresolved (python-celery)
//!   force-app/main/default/lwc/contactList/contactList.js (javascript, lwc)
//!     └── handleClick()                — expected: DeclarativeWiringUnparsed (js-lwc)
//!   force-app/main/default/aura/GE_GiftEntryForm/GE_GiftEntryFormController.js (javascript, aura)
//!     └── handleSave()                 — expected: DeclarativeWiringUnparsed (js-aura)
//!   force-app/main/default/aura/GE_GiftEntryForm/FormUtils.js (javascript, aura)
//!     └── formatMoney()                — expected: DeclarativeWiringUnparsed (js-aura)
//!       (proves §13 "broad segment-match" — a narrow rule that only
//!        matched `*Controller.js` / `*Helper.js` would miss this.)
//!   jest.setup.js                                         (javascript, jest)
//!     └── setupJestGlobals()           — expected: FrameworkAnnotationUnresolved (js-jest)
//!   vitest.setup.ts                                       (typescript, vitest)
//!     └── setupVitestGlobals()         — expected: FrameworkAnnotationUnresolved (js-vitest)
//! ```
//!
//! The assertions are deliberately tight — every verdict's
//! `classifier` string is checked as well as the reason — so that
//! accidental fallthrough to the generic classifier fails the
//! test loudly.

use graphengine_analysis::health::config::Ecosystem;
use graphengine_analysis::health::dead_code_classifier::FrameworkRuleRegistry;
use graphengine_analysis::health::graph::AnalysisGraph;
use rusqlite::Connection;

fn create_schema(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS nodes (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            fqn TEXT NOT NULL,
            location TEXT NOT NULL,
            provenance TEXT NOT NULL,
            properties TEXT NOT NULL DEFAULT '{}',
            trait_metadata TEXT
        );
        CREATE TABLE IF NOT EXISTS edges (
            from_id TEXT NOT NULL REFERENCES nodes(id),
            to_id TEXT NOT NULL REFERENCES nodes(id),
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, kind)
        );
        ",
    )
    .unwrap();
}

fn insert_node(
    conn: &Connection,
    id: &str,
    kind: &str,
    fqn: &str,
    location: &str,
    properties: &str,
) {
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            kind,
            fqn,
            location,
            r#"{"source": "Lsp", "confidence": "High"}"#,
            properties,
        ],
    )
    .unwrap();
}

fn insert_edge(conn: &Connection, from: &str, to: &str, kind: &str) {
    conn.execute(
        "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![from, to, kind, r#"{"source": "Lsp", "confidence": "High"}"#,],
    )
    .unwrap();
}

fn loc(path: &str) -> String {
    format!(
        r#"{{"file": "{}", "start_line": 1, "start_char": 0, "end_line": 10, "end_char": 0}}"#,
        path
    )
}

fn file_props(path: &str, language: &str, frameworks: &[&str]) -> String {
    let fw_json = frameworks
        .iter()
        .map(|s| format!("\"{}\"", s))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"path_repo_rel":"{}","role":"source","language":"{}","frameworks":[{}]}}"#,
        path, language, fw_json
    )
}

fn fn_props(tags: &[&str]) -> String {
    if tags.is_empty() {
        "{}".into()
    } else {
        let tag_json = tags
            .iter()
            .map(|s| format!("\"{}\"", s))
            .collect::<Vec<_>>()
            .join(",");
        format!(r#"{{"entry_points":[{}]}}"#, tag_json)
    }
}

#[test]
fn polyglot_mixed_dispatch_routes_each_node_to_its_framework() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);

    // --- Files -----------------------------------------------------
    insert_node(
        &conn,
        "f_tdtm",
        "File",
        "classes::TDTM_Opportunity",
        &loc("force-app/main/default/classes/TDTM_Opportunity.cls"),
        &file_props(
            "force-app/main/default/classes/TDTM_Opportunity.cls",
            "apex",
            &["tdtm"],
        ),
    );
    insert_node(
        &conn,
        "f_plain_apex",
        "File",
        "classes::OpportunityService",
        &loc("force-app/main/default/classes/OpportunityService.cls"),
        &file_props(
            "force-app/main/default/classes/OpportunityService.cls",
            "apex",
            &["plain"],
        ),
    );
    insert_node(
        &conn,
        "f_rest",
        "File",
        "classes::AccountRest",
        &loc("force-app/main/default/classes/AccountRest.cls"),
        &file_props(
            "force-app/main/default/classes/AccountRest.cls",
            "apex",
            &["restresource"],
        ),
    );
    insert_node(
        &conn,
        "f_aura",
        "File",
        "classes::AuraController",
        &loc("force-app/main/default/classes/AuraController.cls"),
        &file_props(
            "force-app/main/default/classes/AuraController.cls",
            "apex",
            &["plain"],
        ),
    );
    insert_node(
        &conn,
        "f_views",
        "File",
        "django::app::views",
        &loc("django/app/views.py"),
        &file_props("django/app/views.py", "python", &["django"]),
    );
    insert_node(
        &conn,
        "f_tasks",
        "File",
        "django::app::tasks",
        &loc("django/app/tasks.py"),
        &file_props("django/app/tasks.py", "python", &["celery"]),
    );
    insert_node(
        &conn,
        "f_lwc",
        "File",
        "lwc::contactList",
        &loc("force-app/main/default/lwc/contactList/contactList.js"),
        &file_props(
            "force-app/main/default/lwc/contactList/contactList.js",
            "javascript",
            &["lwc"],
        ),
    );
    insert_node(
        &conn,
        "f_aura_canonical",
        "File",
        "aura::GE_GiftEntryForm::GE_GiftEntryFormController",
        &loc("force-app/main/default/aura/GE_GiftEntryForm/GE_GiftEntryFormController.js"),
        &file_props(
            "force-app/main/default/aura/GE_GiftEntryForm/GE_GiftEntryFormController.js",
            "javascript",
            &["aura"],
        ),
    );
    // Non-canonical helper inside the Aura bundle — proves the
    // roadmap §13 broad segment-match. A narrow rule keyed on
    // `<Name>Controller.js` / `<Name>Helper.js` would leave this
    // file tagged `plain` and misclassify its symbols as
    // visibility_private_unused, re-introducing the R28 bug in a
    // new shape.
    insert_node(
        &conn,
        "f_aura_noncanonical",
        "File",
        "aura::GE_GiftEntryForm::FormUtils",
        &loc("force-app/main/default/aura/GE_GiftEntryForm/FormUtils.js"),
        &file_props(
            "force-app/main/default/aura/GE_GiftEntryForm/FormUtils.js",
            "javascript",
            &["aura"],
        ),
    );
    insert_node(
        &conn,
        "f_jest",
        "File",
        "jest::setup",
        &loc("jest.setup.js"),
        &file_props("jest.setup.js", "javascript", &["jest"]),
    );
    insert_node(
        &conn,
        "f_vitest",
        "File",
        "vitest::setup",
        &loc("vitest.setup.ts"),
        &file_props("vitest.setup.ts", "typescript", &["vitest"]),
    );

    // --- Functions -------------------------------------------------
    // NOTE: Function nodes inherit language/frameworks via
    // `AnalysisGraph::propagate_file_metadata_to_descendants`, so
    // we leave them blank here. That codepath is the one under test.
    insert_node(
        &conn,
        "fn_tdtm_run",
        "Function",
        "force-app/main/default/classes/TDTM_Opportunity.cls::TDTM_Opportunity::run()",
        &loc("force-app/main/default/classes/TDTM_Opportunity.cls"),
        &fn_props(&[]),
    );
    insert_node(
        &conn,
        "fn_plain_apex_recalc",
        "Function",
        "OpportunityService.recalculate",
        &loc("force-app/main/default/classes/OpportunityService.cls"),
        &fn_props(&[]),
    );
    insert_node(
        &conn,
        "fn_rest_get",
        "Function",
        "AccountRest.doGet",
        &loc("force-app/main/default/classes/AccountRest.cls"),
        &fn_props(&["http_get"]),
    );
    insert_node(
        &conn,
        "fn_aura_get",
        "Function",
        "AuraController.getData",
        &loc("force-app/main/default/classes/AuraController.cls"),
        &fn_props(&["aura_enabled"]),
    );
    insert_node(
        &conn,
        "fn_django_get",
        "Function",
        "views.OpportunityView.get",
        &loc("django/app/views.py"),
        &fn_props(&[]),
    );
    insert_node(
        &conn,
        "fn_celery_process",
        "Function",
        "tasks.process_job",
        &loc("django/app/tasks.py"),
        &fn_props(&[]),
    );
    insert_node(
        &conn,
        "fn_lwc_click",
        "Function",
        "contactList.handleClick",
        &loc("force-app/main/default/lwc/contactList/contactList.js"),
        &fn_props(&[]),
    );
    insert_node(
        &conn,
        "fn_aura_canonical_save",
        "Function",
        "GE_GiftEntryFormController.handleSave",
        &loc("force-app/main/default/aura/GE_GiftEntryForm/GE_GiftEntryFormController.js"),
        &fn_props(&[]),
    );
    insert_node(
        &conn,
        "fn_aura_noncanonical_fmt",
        "Function",
        "FormUtils.formatMoney",
        &loc("force-app/main/default/aura/GE_GiftEntryForm/FormUtils.js"),
        &fn_props(&[]),
    );
    insert_node(
        &conn,
        "fn_jest_setup",
        "Function",
        "jest.setup.setupJestGlobals",
        &loc("jest.setup.js"),
        &fn_props(&[]),
    );
    insert_node(
        &conn,
        "fn_vitest_setup",
        "Function",
        "vitest.setup.setupVitestGlobals",
        &loc("vitest.setup.ts"),
        &fn_props(&[]),
    );

    // --- Containment ----------------------------------------------
    for (f, t) in [
        ("f_tdtm", "fn_tdtm_run"),
        ("f_plain_apex", "fn_plain_apex_recalc"),
        ("f_rest", "fn_rest_get"),
        ("f_aura", "fn_aura_get"),
        ("f_views", "fn_django_get"),
        ("f_tasks", "fn_celery_process"),
        ("f_lwc", "fn_lwc_click"),
        ("f_aura_canonical", "fn_aura_canonical_save"),
        ("f_aura_noncanonical", "fn_aura_noncanonical_fmt"),
        ("f_jest", "fn_jest_setup"),
        ("f_vitest", "fn_vitest_setup"),
    ] {
        insert_edge(&conn, f, t, "Contains");
    }

    // --- Run the classifier ---------------------------------------
    let graph = AnalysisGraph::load(&conn).unwrap();

    // Sanity: propagation ran and every function inherited its
    // file's framework list.
    for (fn_id, expected) in [
        ("fn_tdtm_run", "tdtm"),
        ("fn_rest_get", "restresource"),
        ("fn_django_get", "django"),
        ("fn_celery_process", "celery"),
        ("fn_lwc_click", "lwc"),
        ("fn_aura_canonical_save", "aura"),
        ("fn_aura_noncanonical_fmt", "aura"),
        ("fn_jest_setup", "jest"),
        ("fn_vitest_setup", "vitest"),
    ] {
        let node = graph.nodes.get(fn_id).expect("function node present");
        assert!(
            node.frameworks.iter().any(|f| f == expected),
            "expected {fn_id} to carry framework {expected}, got {:?}",
            node.frameworks
        );
    }

    let reg = FrameworkRuleRegistry::default();
    let ids = vec![
        "fn_tdtm_run".to_string(),
        "fn_plain_apex_recalc".to_string(),
        "fn_rest_get".to_string(),
        "fn_aura_get".to_string(),
        "fn_django_get".to_string(),
        "fn_celery_process".to_string(),
        "fn_lwc_click".to_string(),
        "fn_aura_canonical_save".to_string(),
        "fn_aura_noncanonical_fmt".to_string(),
        "fn_jest_setup".to_string(),
        "fn_vitest_setup".to_string(),
    ];

    // Advisory ecosystem hint is intentionally wrong (Apex) to
    // prove the registry ignores it for dispatch.
    let verdicts = reg.classify_batch(&ids, &graph, Ecosystem::Apex);
    let by_id: std::collections::BTreeMap<&str, &_> =
        verdicts.iter().map(|v| (v.node_id.as_str(), v)).collect();

    use graphengine_analysis::health::report::DeadCodeReason;

    let tdtm = by_id["fn_tdtm_run"];
    assert_eq!(tdtm.classifier, "apex-tdtm");
    assert_eq!(tdtm.reason, DeadCodeReason::DynamicDispatchTarget);

    let plain_apex = by_id["fn_plain_apex_recalc"];
    // No tag, no framework heuristic, no visibility set → generic
    // falls through to NoCallers.
    assert_eq!(plain_apex.classifier, "generic");
    assert_eq!(plain_apex.reason, DeadCodeReason::NoCallers);

    // Apex @HttpGet method → universal entry-point-tag pre-rule
    // wins; the `restresource` framework rule is redundant for
    // tagged methods but remains live for evidence on untagged
    // siblings. This also proves the universal pre-pass runs
    // *before* the framework rule set.
    let rest = by_id["fn_rest_get"];
    assert_eq!(rest.classifier, "universal-entry-point-tag");
    assert_eq!(rest.reason, DeadCodeReason::FrameworkAnnotationUnresolved);
    assert!(rest.evidence.contains("http_get"));

    let aura = by_id["fn_aura_get"];
    assert_eq!(aura.classifier, "universal-entry-point-tag");
    assert_eq!(aura.reason, DeadCodeReason::FrameworkAnnotationUnresolved);
    assert!(aura.evidence.contains("aura_enabled"));

    let django = by_id["fn_django_get"];
    assert_eq!(django.classifier, "python-django");
    assert_eq!(django.reason, DeadCodeReason::DeclarativeWiringUnparsed);

    let celery = by_id["fn_celery_process"];
    assert_eq!(celery.classifier, "python-celery");
    assert_eq!(celery.reason, DeadCodeReason::FrameworkAnnotationUnresolved);

    let lwc = by_id["fn_lwc_click"];
    assert_eq!(lwc.classifier, "js-lwc");
    assert_eq!(lwc.reason, DeadCodeReason::DeclarativeWiringUnparsed);

    // Canonical Aura controller method — broad segment-match
    // routes through the `aura` rule set.
    let aura_canonical = by_id["fn_aura_canonical_save"];
    assert_eq!(aura_canonical.classifier, "js-aura");
    assert_eq!(
        aura_canonical.reason,
        DeadCodeReason::DeclarativeWiringUnparsed
    );
    assert!(aura_canonical.evidence.contains("Aura bundle"));

    // Non-canonical JS helper inside the Aura bundle — same
    // verdict proves the broad-match contract (a narrow
    // `*Controller.js` / `*Helper.js` filename rule would leave
    // this node on the generic fallback).
    let aura_noncanonical = by_id["fn_aura_noncanonical_fmt"];
    assert_eq!(aura_noncanonical.classifier, "js-aura");
    assert_eq!(
        aura_noncanonical.reason,
        DeadCodeReason::DeclarativeWiringUnparsed
    );

    let jest = by_id["fn_jest_setup"];
    assert_eq!(jest.classifier, "js-jest");
    assert_eq!(jest.reason, DeadCodeReason::FrameworkAnnotationUnresolved);
    assert!(jest.evidence.contains("Jest"));
    assert!(jest.evidence.contains("jest.setup.js"));

    let vitest = by_id["fn_vitest_setup"];
    assert_eq!(vitest.classifier, "js-vitest");
    assert_eq!(vitest.reason, DeadCodeReason::FrameworkAnnotationUnresolved);
    assert!(vitest.evidence.contains("Vitest"));
    assert!(vitest.evidence.contains("vitest.setup.ts"));

    // Jest and Vitest must stay attribution-distinct so
    // `reason_breakdown` histograms remain honest — see the
    // roadmap §13 sizing-rule rationale.
    assert_ne!(
        jest.classifier, vitest.classifier,
        "Jest and Vitest must keep distinct classifier attributions"
    );

    // Final invariant: no two nodes share a classifier attribution
    // where the framework differs — i.e. no cross-contamination
    // between Apex / Python / JavaScript rule sets.
    let apex_classifiers: Vec<_> = verdicts
        .iter()
        .filter(|v| v.classifier.starts_with("apex-"))
        .map(|v| v.node_id.clone())
        .collect();
    let py_classifiers: Vec<_> = verdicts
        .iter()
        .filter(|v| v.classifier.starts_with("python-"))
        .map(|v| v.node_id.clone())
        .collect();
    let mut js_classifiers: Vec<_> = verdicts
        .iter()
        .filter(|v| v.classifier.starts_with("js-"))
        .map(|v| v.node_id.clone())
        .collect();
    js_classifiers.sort();

    assert!(
        apex_classifiers.iter().all(|id| id.starts_with("fn_tdtm")),
        "apex-* classifier leaked onto non-apex nodes: {:?}",
        apex_classifiers
    );
    assert!(
        py_classifiers
            .iter()
            .all(|id| id.starts_with("fn_django") || id.starts_with("fn_celery")),
        "python-* classifier leaked: {:?}",
        py_classifiers
    );
    let mut expected_js = vec![
        "fn_aura_canonical_save".to_string(),
        "fn_aura_noncanonical_fmt".to_string(),
        "fn_jest_setup".to_string(),
        "fn_lwc_click".to_string(),
        "fn_vitest_setup".to_string(),
    ];
    expected_js.sort();
    assert_eq!(
        js_classifiers, expected_js,
        "js-* classifiers must land only on JS / template nodes"
    );
}
