//! Determinism regression gate for the analysis pipeline (R35).
//!
//! R35 surfaced during WS-TRUTH-A PR 2 pre-flight: two independent
//! cargo builds of `ge-analyze` run against the *same* parse DB at
//! the *same* git SHA produced a ~24 000-line normalised JSON diff.
//! The drift was traced to `HashMap` iteration order feeding into
//!   * cohesion finding ID assignment (`cohesion-<n>`),
//!   * avg_cohesion / avg_distance float summation (last-ulp drift),
//!   * coupling finding emission order.
//!
//! The fix (PR 5.5) converts the relevant result collections to
//! `BTreeMap<String, _>` so every downstream consumer sees modules
//! in a canonical lexical order. **This test is the gate that
//! prevents the regression from reappearing.**
//!
//! Shape
//! -----
//! * Build a synthetic parse DB with a deliberately non-trivial
//!   module graph (≥ 3 modules with ≥ 6 functions each and enough
//!   cross-module edges to force cohesion + coupling + dead-code
//!   analysis to actually run). The module set is specifically
//!   chosen to produce ≥ 2 cohesion findings so the ID assignment
//!   bug — which only manifests when more than one finding exists
//!   — is exercised.
//! * Run `run_analysis` twice in the same process against that DB.
//! * Strip the inherently non-deterministic fields (`generated_at`,
//!   `analysis_duration_ms`, `integrity_status.engine_version`,
//!   `integrity_status.engine_commit`) from both reports.
//! * Assert the canonical-key JSON serialisation of the two reports
//!   is byte-identical. On failure, print a bounded diff so the
//!   responsible drift site is visible in the CI log.
//!
//! Why two runs in the same process instead of two cargo builds
//! --------------------------------------------------------------
//! `HashMap` iteration order across two cargo builds drifts due to
//! the `RandomState` seed being reseeded per process. Within a
//! single process, `HashMap` iteration order also drifts between
//! map insertions — the seed is stable per-map, but the insertion-
//! order effect is enough to flush the drift if the fix is
//! regressed. Running two back-to-back `run_analysis` invocations
//! in the same test process reliably reproduces the pre-fix drift
//! on a 3-module fixture and matches what CI can assert without
//! re-invoking cargo.

use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;

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
        CREATE INDEX IF NOT EXISTS idx_nodes_fqn ON nodes(fqn);
        CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);

        CREATE TABLE IF NOT EXISTS edges (
            from_id TEXT NOT NULL REFERENCES nodes(id),
            to_id TEXT NOT NULL REFERENCES nodes(id),
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, kind)
        );
        CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id);
        CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id);
        CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);
        ",
    )
    .unwrap();
}

fn insert_node(conn: &Connection, id: &str, kind: &str, fqn: &str, properties: &str) {
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            kind,
            fqn,
            r#"{"file": "src/test.ts", "start_line": 1, "start_char": 0, "end_line": 10, "end_char": 0}"#,
            r#"{"source": "Lsp", "confidence": "High"}"#,
            properties,
        ],
    ).unwrap();
}

fn insert_edge(conn: &Connection, from_id: &str, to_id: &str, kind: &str) {
    conn.execute(
        "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            from_id,
            to_id,
            format!(r#"{{"kind":"{kind}"}}"#),
            r#"{"source": "Lsp", "confidence": "High"}"#,
        ],
    )
    .unwrap();
}

/// Build a synthetic parse DB with three Folder-level analysis
/// modules. Each module contains a file with six functions, two of
/// which are connected — guaranteeing ≥ 2 connected components per
/// module and therefore a cohesion finding per module at the
/// default threshold. Cross-module call edges introduce coupling
/// pressure so `compute_coupling` also emits findings.
fn build_multi_module_db(path: &std::path::Path) {
    let _ = fs::remove_file(path);
    let conn = Connection::open(path).unwrap();
    create_schema(&conn);

    insert_node(
        &conn,
        "proj",
        "Project",
        "project",
        r#"{"language": "typescript"}"#,
    );

    // Module folders. The key names are deliberately NOT in
    // lexical order of insertion — the test has to pass despite
    // `src/zeta`, `src/alpha`, `src/mu` being inserted here in a
    // non-sorted order.
    for (mid, mfqn, mpath) in &[
        ("m_zeta", "src::zeta", "src/zeta"),
        ("m_alpha", "src::alpha", "src/alpha"),
        ("m_mu", "src::mu", "src/mu"),
    ] {
        insert_node(
            &conn,
            mid,
            "Folder",
            mfqn,
            &format!(r#"{{"path_repo_rel": "{mpath}", "role": "source"}}"#),
        );
        insert_edge(&conn, "proj", mid, "Contains");

        let file_id = format!("{mid}_file");
        insert_node(
            &conn,
            &file_id,
            "File",
            &format!("{mfqn}::file"),
            &format!(r#"{{"path_repo_rel": "{mpath}/file.ts", "role": "source"}}"#),
        );
        insert_edge(&conn, mid, &file_id, "Contains");

        for idx in 0..6 {
            let fn_id = format!("{mid}_fn_{idx}");
            insert_node(
                &conn,
                &fn_id,
                "Function",
                &format!("{mfqn}::fn_{idx}"),
                r#"{"cyclomatic_complexity": 5, "cognitive_complexity": 4}"#,
            );
            insert_edge(&conn, &file_id, &fn_id, "Contains");
        }

        // Two internal call edges → one connected pair, four
        // singletons → 5 connected components → cohesion 0.2 → a
        // cohesion finding at the default critical threshold.
        insert_edge(
            &conn,
            &format!("{mid}_fn_0"),
            &format!("{mid}_fn_1"),
            "Call",
        );
    }

    // Cross-module call edges to introduce coupling. Each module
    // calls exactly one function in the next lexical module so the
    // coupling scores are all ~0.5. This keeps the test
    // deterministic-input itself.
    insert_edge(&conn, "m_alpha_fn_2", "m_mu_fn_2", "Call");
    insert_edge(&conn, "m_mu_fn_2", "m_zeta_fn_2", "Call");
    insert_edge(&conn, "m_zeta_fn_2", "m_alpha_fn_2", "Call");
}

/// Strip the fields whose non-determinism is intrinsic (wall-clock
/// timing, engine identity) — none of them are part of the R35
/// byte-identical contract. Everything else stays in the diff
/// surface.
fn normalise(v: &mut serde_json::Value) {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("generated_at");
        obj.remove("analysis_duration_ms");
        if let Some(integrity) = obj
            .get_mut("integrity_status")
            .and_then(|x| x.as_object_mut())
        {
            integrity.remove("engine_version");
            integrity.remove("engine_commit");
        }
    }
}

fn canonical_json(report: &graphengine_analysis::health::report::HealthReport) -> String {
    let mut v = serde_json::to_value(report).unwrap();
    normalise(&mut v);
    serde_json::to_string_pretty(&v).unwrap()
}

#[test]
fn analysis_pipeline_is_byte_identical_across_runs() {
    let tmp_dir = std::env::temp_dir();
    let db_path: PathBuf = tmp_dir.join(format!(
        "ge_analyze_determinism_{}.sqlite",
        std::process::id()
    ));
    build_multi_module_db(&db_path);

    let db_str = db_path.to_str().unwrap();

    let r1 = graphengine_analysis::health::run_analysis(db_str).expect("run 1 failed");
    let r2 = graphengine_analysis::health::run_analysis(db_str).expect("run 2 failed");

    let j1 = canonical_json(&r1);
    let j2 = canonical_json(&r2);

    // Cleanup the temp DB on success; on failure we leave it so
    // the developer can inspect.
    if j1 == j2 {
        let _ = fs::remove_file(&db_path);
        return;
    }

    let mut bounded = String::new();
    let mut line_count = 0usize;
    for (i, (l1, l2)) in j1.lines().zip(j2.lines()).enumerate() {
        if l1 != l2 {
            bounded.push_str(&format!("  line {i}:\n    run1: {l1}\n    run2: {l2}\n"));
            line_count += 1;
            if line_count >= 40 {
                break;
            }
        }
    }

    panic!(
        "R35 regression: two back-to-back run_analysis invocations on the \
         same parse DB produced different HealthReport JSON after \
         normalisation. DB kept at {} for inspection. First {} diverging \
         lines:\n{}",
        db_path.display(),
        line_count,
        bounded
    );
}

#[test]
fn cohesion_modules_iterate_in_lexical_order() {
    // Direct unit-level gate on the BTreeMap contract. If anyone
    // converts `CohesionResult::modules` back to a `HashMap`, this
    // test fails on a 3-module fixture because `Vec::from_iter` on
    // a `HashMap` would surface a non-lexical order. With the
    // BTreeMap contract in place, the lexical order is guaranteed.
    let tmp_dir = std::env::temp_dir();
    let db_path: PathBuf = tmp_dir.join(format!(
        "ge_analyze_cohesion_order_{}.sqlite",
        std::process::id()
    ));
    build_multi_module_db(&db_path);

    let r = graphengine_analysis::health::run_analysis(db_path.to_str().unwrap())
        .expect("analysis failed");

    let cohesion_finding_descs: Vec<&str> = r
        .findings
        .iter()
        .filter(|f| {
            matches!(
                f.finding_type,
                graphengine_analysis::health::report::FindingType::LowCohesion
            )
        })
        .map(|f| f.description.as_str())
        .collect();

    // Each description is prefixed by the module id. Verify
    // strictly-lexical module-id order on those prefixes.
    let mid_prefixes: Vec<&str> = cohesion_finding_descs
        .iter()
        .filter_map(|d| d.split(':').next())
        .collect();
    let mut sorted = mid_prefixes.clone();
    sorted.sort();
    assert_eq!(
        mid_prefixes, sorted,
        "cohesion findings are not in lexical module-id order — \
         CohesionResult::modules is likely a HashMap again (R35 regression)"
    );

    let _ = fs::remove_file(&db_path);
}
