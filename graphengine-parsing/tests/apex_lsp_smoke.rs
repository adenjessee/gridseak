#![cfg(feature = "lsp-tests")]
//! End-to-end smoke test for the Apex LSP path (`apex-jorje-lsp.jar`).
//!
//! # Why this exists
//!
//! The entire Apex integration hinges on one untested assumption: that
//! `apex-jorje-lsp.jar` speaks enough of the LSP dialect that
//! [`SimpleLspClient`] can drive it (initialize → textDocument/didOpen →
//! textDocument/definition → shutdown → exit). This test is the
//! load-bearing verification that closes that gap.
//!
//! It is **not** an enforcement gate — it is a committed harness that any
//! developer with a provisioned machine can fire in one command:
//!
//! ```bash
//! export GRAPHENGINE_APEX_JORJE_JAR=/path/to/apex-jorje-lsp.jar
//! export GRAPHENGINE_JAVA_HOME=$(/usr/libexec/java_home -v 17)   # macOS
//! cargo test -p graphengine-parsing --features lsp-tests \
//!     --test apex_lsp_smoke -- --nocapture
//! ```
//!
//! Skips (does not fail) when:
//!   - `java` is not on PATH and `GRAPHENGINE_JAVA_HOME` is unset
//!   - `GRAPHENGINE_APEX_JORJE_JAR` is unset AND no bundled jar is present
//!
//! Matches the skip semantics of [`tests/lsp_integration_real.rs`] so CI
//! stays green on machines that are not Apex-provisioned.
//!
//! # What this test proves
//!
//! 1. `command_locator::resolve_lsp_command` for Apex produces a spawnable
//!    `java -cp <jar> apex.jorje.lsp.ApexLanguageServerLauncher` invocation.
//! 2. `SimpleLspClient::initialize` successfully exchanges `initialize` /
//!    `initialized` with apex-jorje using the `initializationOptions` shape
//!    from `configs/apex.yaml`.
//! 3. The session reaches `SessionState::Ready` within a bounded timeout.
//! 4. A real `.cls` file can be opened, and `textDocument/definition` does
//!    not surface a protocol-level error (regardless of whether a definition
//!    is returned — apex-jorje may need more project context to resolve
//!    on such a small fixture).
//!
//! # What this test does NOT prove
//!
//! - Full recall on large SFDX repos (that's `e2e_validation` against
//!   `dreamhouse-lwc` + NPSP, which the plan sequences separately).
//! - Correctness of the heuristic fallback merge logic (covered by
//!   [`syntax::language::apex::resolver_dispatch`] unit tests).

use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::infrastructure::lsp::command_locator::resolve_lsp_command;
use graphengine_parsing::infrastructure::lsp::session::{
    ReadinessStrategy, SessionState, SessionSupervisor,
};
use graphengine_parsing::syntax::language::apex::build_apex_session_options;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use url::Url;

/// Text of a minimal-but-realistic Apex class that exercises enough of the
/// grammar to trigger apex-jorje's symbol resolution pipeline.
const APEX_FIXTURE: &str = r#"
public class Smoke {
    public static Integer add(Integer a, Integer b) {
        return a + b;
    }

    public static Integer consumer(Integer x) {
        return add(x, 1);
    }
}
"#;

/// Returns `true` if we can plausibly launch apex-jorje on this host.
///
/// Deliberately permissive: if `command_locator` thinks it can resolve the
/// command, we go. The test itself will surface any runtime failure with a
/// useful message.
fn apex_stack_available() -> bool {
    let config = match load_config("apex") {
        Ok(c) => c,
        Err(_) => return false,
    };
    resolve_lsp_command(&config).is_ok()
}

#[tokio::test]
async fn apex_lsp_initializes_and_responds_to_did_open() {
    if !apex_stack_available() {
        eprintln!(
            "Skipping Apex LSP smoke test: apex-jorje stack not available. \
             Set GRAPHENGINE_APEX_JORJE_JAR and either GRAPHENGINE_JAVA_HOME \
             or install `java` to PATH."
        );
        return;
    }

    let config = load_config("apex").expect("apex config should load");

    let temp_dir = tempfile::tempdir().expect("create temp workspace");
    let workspace_root = Url::from_directory_path(temp_dir.path()).unwrap();

    let fixture_path = temp_dir.path().join("Smoke.cls");
    std::fs::write(&fixture_path, APEX_FIXTURE).expect("write Smoke.cls");

    let session = Arc::new(SessionSupervisor::new(config, Some(workspace_root.clone())));

    // apex-jorje is heavier than most LSPs — give it a generous budget but
    // bounded so a hang shows up as a failure, not a stuck test runner.
    let init_deadline = Duration::from_secs(60);
    let init_result = timeout(init_deadline, session.initialize()).await;

    match init_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!(
            "apex-jorje initialize returned a protocol error: {e:?}. \
             This is the load-bearing failure — fix the LSP dialect mismatch \
             before doing anything else on the Apex integration."
        ),
        Err(_) => panic!(
            "apex-jorje did not complete LSP initialize within {:?}. \
             Either the jar startup is slow (increase timeout) or the handshake \
             is stuck (check lsp_initialization_options).",
            init_deadline
        ),
    }

    assert!(
        session.get_state().is_functional(),
        "session must be functional after initialize, got state={:?}",
        session.get_state()
    );
    assert_eq!(
        session.get_state(),
        SessionState::Ready,
        "expected Ready; apex-jorje reported a degraded startup — \
         inspect SessionMetrics for last_error"
    );

    let uri_str = fixture_path.to_string_lossy().to_string();
    let open_result = session
        .document_did_open(&uri_str, APEX_FIXTURE.to_string())
        .await;
    assert!(
        open_result.is_ok(),
        "textDocument/didOpen must succeed for a legal .cls file, got {:?}",
        open_result
    );

    // Give apex-jorje a moment to index the (tiny) file before querying.
    // 500ms is empirically enough for a single-file open; longer budgets
    // belong in the large-repo e2e tests.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // `consumer` calls `add` on line 7, columns ~16-18 (the `add` token).
    // We don't hard-code exact byte positions because whitespace edits to
    // the fixture would silently invalidate the test — instead we ask by
    // symbol name, which is what the real call-resolution path does.
    let ask_at = graphengine_parsing::domain::Range::with_file(
        6, // 0-indexed line containing `return add(x, 1);`
        15,
        6,
        18,
        uri_str.clone(),
    );
    let def = session.find_definition("add", &ask_at).await;

    // We do NOT assert that a definition is returned — apex-jorje may need
    // more project context (sfdx-project.json, compiled class metadata) to
    // resolve even same-file references. What we DO assert is that the
    // request is handled without a protocol error: that proves the wire
    // contract, which is what this smoke test exists to verify.
    assert!(
        def.is_ok(),
        "textDocument/definition must not produce a protocol error, got {:?}",
        def
    );
    if let Ok(Some(location)) = &def {
        eprintln!(
            "apex-jorje resolved `add` -> {} (bonus: this means the LSP is \
             doing real semantic work, not just syntax)",
            location.file
        );
    } else {
        eprintln!(
            "apex-jorje handled definition request but returned None \
             (expected on a single-file fixture with no project manifest)"
        );
    }
}

/// F.2 end-to-end smoke: a real SFDX workspace with
/// `sfdx-project.json`, a package directory, and a class that
/// references itself. Exercises:
///
/// * Workspace-folder advertisement — jorje must see `force-app` as
///   a declared folder, not just `rootUri`.
/// * Readiness barrier — `SessionSupervisor::initialize` now waits
///   for a `$/progress` `end` frame (or the canary probe to succeed)
///   before flipping to `Ready`.
/// * Observability counters — at least one notification must be
///   recorded during startup.
///
/// Assertions are deliberately light on semantic recall (jorje on a
/// one-class fixture is still flaky) but strict on wire contract
/// and observability plumbing.
#[tokio::test]
async fn apex_lsp_readiness_barrier_and_workspace_folders_are_advertised() {
    if !apex_stack_available() {
        eprintln!(
            "Skipping F.2 smoke: apex-jorje stack not available. \
             Set GRAPHENGINE_APEX_JORJE_JAR and either GRAPHENGINE_JAVA_HOME \
             or install `java` to PATH."
        );
        return;
    }

    let config = load_config("apex").expect("apex config should load");

    let temp = tempfile::tempdir().expect("create temp workspace");
    let workspace_root = Url::from_directory_path(temp.path()).unwrap();

    // Minimal but authoritative SFDX project. This is what jorje
    // expects in the wild — without it, `workspaceFolders` carries
    // the root and jorje silently falls through to "no project".
    std::fs::write(
        temp.path().join("sfdx-project.json"),
        r#"{"packageDirectories":[{"path":"force-app","default":true}]}"#,
    )
    .unwrap();
    let classes_dir = temp.path().join("force-app/main/default/classes");
    std::fs::create_dir_all(&classes_dir).unwrap();
    let fixture = classes_dir.join("Smoke.cls");
    std::fs::write(&fixture, APEX_FIXTURE).expect("write Smoke.cls");

    let options = build_apex_session_options(Some(&workspace_root));
    assert_eq!(
        options.workspace_folders.len(),
        1,
        "force-app must be advertised as a single workspaceFolder"
    );
    assert!(
        matches!(
            options.readiness,
            ReadinessStrategy::ProgressAndProbe { .. }
        ),
        "Apex sessions must use the ProgressAndProbe readiness strategy"
    );

    let mut supervisor = SessionSupervisor::new(config, Some(workspace_root));
    supervisor.set_workspace_folders(options.workspace_folders.clone());
    supervisor.set_readiness_strategy(options.readiness);
    let session = Arc::new(supervisor);

    let init_deadline = Duration::from_secs(90);
    let init_result = timeout(init_deadline, session.initialize()).await;
    assert!(
        init_result.is_ok(),
        "initialize did not complete within {init_deadline:?}"
    );
    init_result.unwrap().expect("initialize must succeed");

    assert_eq!(session.get_state(), SessionState::Ready);

    let metrics = session.metrics().await;
    assert!(
        metrics.notifications_received > 0,
        "F.1 observability must record at least one jorje notification; \
         got {metrics:?}"
    );
    eprintln!(
        "F.2 readiness metrics: notifications={} stderr_lines={} indexing_signals={} \
         last_progress={:?}",
        metrics.notifications_received,
        metrics.stderr_lines_observed,
        metrics.indexing_messages_seen,
        metrics.last_indexing_progress
    );
}
