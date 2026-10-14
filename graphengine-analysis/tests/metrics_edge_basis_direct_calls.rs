//! Structural metrics count direct calls; dead code counts every use.
//!
//! Regression fixture for the fake cycles GridSeak reported on itself:
//! `DisclosedSemanticResolver` implements the resolver trait by wrapping
//! another resolver and forwarding `resolve` to it. The forwarding call
//! goes through the trait method, so the compiler index fans it out to
//! every implementation, and together with the apex `resolver_dispatch`
//! helper that dispatches through the same trait the call graph closed a
//! loop no code path takes:
//!
//! ```text
//! ApexResolver::resolve --call--> resolver_dispatch
//! resolver_dispatch --dispatch--> DisclosedResolver::resolve
//! DisclosedResolver::resolve --dispatch--> ApexResolver::resolve
//! ```
//!
//! With `MetricsEdgeBasis::DirectCalls` (the default) that loop is not a
//! cycle; it is listed under `integrity_status.metrics_edge_basis
//! .indirect_only_cycles` instead. Functions reached only through
//! dispatch or used only as a value stay live.

use graphengine_analysis::health::config::DeadCodeConfig;
use graphengine_analysis::health::cycles::{detect_cycles, detect_indirect_only_cycles};
use graphengine_analysis::health::dead_code::detect_dead_code;
use graphengine_analysis::health::graph::{AnalysisGraph, MetricsEdgeBasis};
use graphengine_analysis::health::report::CAVEAT_METRICS_EDGE_BASIS_DIRECT_CALLS_V1;
use rusqlite::Connection;

const APEX_RESOLVE: &str = "fApexResolve";
const DISPATCH: &str = "fDispatch";
const DISCLOSED_RESOLVE: &str = "fDisclosedResolve";
const RUN: &str = "fRun";
const ON_EVENT: &str = "fOnEvent";
const ORPHAN: &str = "fOrphan";

fn create_schema(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE nodes (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            fqn TEXT NOT NULL,
            location TEXT NOT NULL,
            provenance TEXT NOT NULL,
            properties TEXT NOT NULL DEFAULT '{}',
            trait_metadata TEXT
        );
        CREATE TABLE edges (
            from_id TEXT NOT NULL,
            to_id   TEXT NOT NULL,
            kind    TEXT NOT NULL,
            provenance TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, kind)
        );
        CREATE TABLE parse_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        INSERT INTO parse_meta (key, value) VALUES ('schema_version', '3');
        ",
    )
    .unwrap();
}

fn insert_fn(conn: &Connection, id: &str, fqn: &str, file: &str) {
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties)
         VALUES (?1, 'Function', ?2, ?3, ?4, '{\"visibility\":\"private\"}')",
        rusqlite::params![
            id,
            fqn,
            format!(
                r#"{{"file":"{file}","start_line":1,"start_char":0,"end_line":5,"end_char":0}}"#
            ),
            r#"{"source":"Compiler","confidence":"High"}"#,
        ],
    )
    .unwrap();
}

/// `via = None` writes a provenance without the field (older / heuristic edges).
fn insert_call(conn: &Connection, from: &str, to: &str, via: Option<&str>) {
    let provenance = match via {
        Some(v) => format!(r#"{{"source":"Compiler","confidence":"High","via":"{v}"}}"#),
        None => r#"{"source":"Compiler","confidence":"High"}"#.to_string(),
    };
    conn.execute(
        "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, '{\"kind\":\"Call\"}', ?3)",
        rusqlite::params![from, to, provenance],
    )
    .unwrap();
}

fn build_fixture(path: &str) {
    let conn = Connection::open(path).unwrap();
    create_schema(&conn);
    insert_fn(
        &conn,
        APEX_RESOLVE,
        "resolver::apex::ApexResolver::resolve",
        "src/resolver/apex.rs",
    );
    insert_fn(
        &conn,
        DISPATCH,
        "resolver::apex::resolver_dispatch",
        "src/resolver/apex.rs",
    );
    insert_fn(
        &conn,
        DISCLOSED_RESOLVE,
        "resolver::disclosed::DisclosedResolver::resolve",
        "src/resolver/disclosed.rs",
    );
    insert_fn(&conn, RUN, "app::runner::run_pass", "src/app/runner.rs");
    insert_fn(
        &conn,
        ON_EVENT,
        "app::runner::on_event",
        "src/app/runner.rs",
    );
    insert_fn(&conn, ORPHAN, "app::runner::orphan", "src/app/runner.rs");

    // ApexResolver::resolve calls the helper directly.
    insert_call(&conn, APEX_RESOLVE, DISPATCH, Some("call"));
    // The helper calls `resolver.resolve()` through the trait: fanned
    // out to every implementation.
    insert_call(&conn, DISPATCH, APEX_RESOLVE, Some("dispatch"));
    insert_call(&conn, DISPATCH, DISCLOSED_RESOLVE, Some("dispatch"));
    // The wrapper forwards `self.inner.resolve()` through the trait,
    // fanned out to every implementation including itself.
    insert_call(&conn, DISCLOSED_RESOLVE, APEX_RESOLVE, Some("dispatch"));
    insert_call(
        &conn,
        DISCLOSED_RESOLVE,
        DISCLOSED_RESOLVE,
        Some("dispatch"),
    );
    // run_pass calls the helper directly (legacy edge with no `via`)
    // and hands on_event over as a callback.
    insert_call(&conn, RUN, DISPATCH, None);
    insert_call(&conn, RUN, ON_EVENT, Some("value"));
    drop(conn);
}

fn load(path: &str, basis: MetricsEdgeBasis) -> AnalysisGraph {
    let conn = Connection::open(path).unwrap();
    let mut g = AnalysisGraph::load_with_options(&conn, 2, false, basis).unwrap();
    g.finalize_production_edges();
    g
}

#[test]
fn wrapper_forwarding_is_not_a_cycle_under_direct_calls() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();
    build_fixture(&path);

    // The fixture does reproduce the old fake cycle when every use counts.
    let all = load(&path, MetricsEdgeBasis::AllUses);
    let all_cycles = detect_cycles(&all);
    assert_eq!(
        all_cycles.cycles.len(),
        1,
        "fixture must reproduce the fake cycle"
    );
    assert_eq!(
        all_cycles.cycles[0].node_ids,
        vec![APEX_RESOLVE, DISCLOSED_RESOLVE, DISPATCH]
    );
    assert!(detect_indirect_only_cycles(&all).is_empty());

    // Direct calls: no cycle, listed as indirect-only instead.
    let direct = load(&path, MetricsEdgeBasis::DirectCalls);
    let direct_cycles = detect_cycles(&direct);
    assert!(
        direct_cycles.cycles.is_empty(),
        "dispatch fan-out must not create cycles: {:?}",
        direct_cycles.cycles
    );
    let info = detect_indirect_only_cycles(&direct);
    assert_eq!(info.len(), 1);
    assert_eq!(
        info[0].node_ids,
        vec![APEX_RESOLVE, DISCLOSED_RESOLVE, DISPATCH]
    );
    assert_eq!(info[0].indirect_edge_indices.len(), 4);
}

#[test]
fn dispatch_and_value_uses_keep_functions_alive() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();
    build_fixture(&path);

    let g = load(&path, MetricsEdgeBasis::DirectCalls);
    // Direct fan-in drops dispatch and value uses...
    assert_eq!(g.fan_in(DISCLOSED_RESOLVE), 0);
    assert_eq!(g.fan_in(ON_EVENT), 0);
    assert_eq!(g.fan_in(APEX_RESOLVE), 0);
    assert_eq!(
        g.fan_in(DISPATCH),
        2,
        "direct call + legacy edge without via"
    );
    // ...but liveness keeps them.
    assert_eq!(g.liveness_fan_in(DISCLOSED_RESOLVE), 2);
    assert_eq!(g.liveness_fan_in(ON_EVENT), 1);
    assert_eq!(g.liveness_fan_in(APEX_RESOLVE), 2);

    let dead = detect_dead_code(&g, &DeadCodeConfig::default());
    assert!(dead.production.iter().any(|id| id == ORPHAN));
    for live in [DISCLOSED_RESOLVE, ON_EVENT, APEX_RESOLVE, DISPATCH] {
        assert!(
            !dead.production.iter().any(|id| id == live),
            "{live} has dispatch/value/direct callers and must not be dead: {:?}",
            dead.production
        );
    }

    // Same dead set as when every use counts.
    let all = load(&path, MetricsEdgeBasis::AllUses);
    assert_eq!(
        detect_dead_code(&all, &DeadCodeConfig::default()).production,
        dead.production
    );
}

#[test]
fn report_states_edge_basis_and_lists_indirect_only_cycle() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();
    build_fixture(&path);

    let report = graphengine_analysis::health::run_analysis(&path).unwrap();
    assert_eq!(report.summary.cycles_found, 0);
    assert!(report
        .integrity_status
        .schema_caveats
        .iter()
        .any(|c| c == CAVEAT_METRICS_EDGE_BASIS_DIRECT_CALLS_V1));
    let basis = report
        .integrity_status
        .metrics_edge_basis
        .as_ref()
        .expect("metrics_edge_basis stamped");
    assert_eq!(basis.basis, "direct_calls_v1");
    assert_eq!(basis.dispatch_edges_excluded, 4);
    assert_eq!(basis.value_edges_excluded, 1);
    assert_eq!(basis.indirect_only_cycles.len(), 1);
    assert_eq!(basis.indirect_only_cycles[0].node_ids.len(), 3);
}
