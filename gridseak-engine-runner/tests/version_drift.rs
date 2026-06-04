//! Regression tests for sidecar version drift detection.
//!
//! The originating bug: a user (or CI flow) installs a new `gridseak`
//! into `~/.gridseak/bin` over an existing prefix without rebuilding
//! `ge-analyze` and `graphengine-parsing`. The runner happily execs the
//! stale sidecar and every scan fails (or worse, silently produces
//! wrong output) deep in the pipeline.
//!
//! These tests pin the runner-side contract that `run_pipeline` fails
//! fast — *before* invoking any sidecar logic — when a binary reports
//! a version that disagrees with the runner's own `CARGO_PKG_VERSION`.

mod common;

use std::path::PathBuf;

use common::{ensure_engine_binaries, pipeline_config, workspace_root};
use gridseak_engine_runner::{run_pipeline, BinaryKind, RunError};
use uuid::Uuid;

/// A short shell script that mimics the clap-default `--version` output
/// shape. We use this instead of a precompiled binary so the test
/// stays hermetic — no build step, no rustc invocation, just `sh` on
/// every supported platform.
fn write_fake_version_binary(path: &PathBuf, name: &str, version: &str) {
    // On unix-like targets we drop a small shell script. Windows
    // support for this runner isn't part of the current shipping
    // surface; if/when it lands, the fake should switch to a `.bat`
    // / native binary built from a build.rs helper.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(
            path,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = \"--version\" ]; then echo \"{name} {version}\"; exit 0; fi\n\
                 echo \"fake binary; arg-passthrough not implemented\" >&2\n\
                 exit 1\n"
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }
    #[cfg(not(unix))]
    {
        let _ = (path, name, version);
        panic!("version_drift tests require a unix-like host");
    }
}

#[tokio::test]
async fn stale_analyzer_version_surfaces_typed_error_before_scan_runs() {
    // The parser binary stays as the real workspace build (so the
    // version-check passes for the parser leg), but we swap the
    // analyzer for a fake that reports an older version. The runner
    // must reject the scan with `BinaryVersionMismatch { which:
    // Analyzer, .. }` BEFORE running the parser, BEFORE loading the
    // language registry, BEFORE writing anything to scratch.
    ensure_engine_binaries();
    let temp = tempfile::tempdir().unwrap();
    let fake_analyzer = temp.path().join("ge-analyze");
    write_fake_version_binary(&fake_analyzer, "ge-analyze", "0.0.0-stale");

    let mut cfg = pipeline_config(temp.path(), Uuid::new_v4(), vec!["rust".to_string()]);
    cfg.analyzer_bin = fake_analyzer.clone();

    let err = run_pipeline(cfg)
        .await
        .expect_err("stale analyzer must produce RunError::BinaryVersionMismatch");
    match err {
        RunError::BinaryVersionMismatch {
            which,
            expected: _,
            actual,
            path,
        } => {
            assert_eq!(which, BinaryKind::Analyzer);
            assert_eq!(actual, "0.0.0-stale");
            assert_eq!(path, fake_analyzer);
        }
        other => panic!("expected BinaryVersionMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn analyzer_that_refuses_to_print_version_surfaces_unreadable() {
    // Older `ge-analyze` builds did not accept `--version` at all and
    // exited 2. The runner must still fail fast with a clear "this
    // binary is too old / wrong" message instead of falling through
    // to a confusing scan-time error.
    let temp = tempfile::tempdir().unwrap();
    let fake_analyzer = temp.path().join("ge-analyze");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(
            &fake_analyzer,
            "#!/bin/sh\n\
             echo \"error: unexpected argument '--version' found\" >&2\n\
             exit 2\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake_analyzer).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_analyzer, perms).unwrap();
    }
    #[cfg(not(unix))]
    {
        panic!("version_drift tests require a unix-like host");
    }

    ensure_engine_binaries();
    let mut cfg = pipeline_config(temp.path(), Uuid::new_v4(), vec!["rust".to_string()]);
    cfg.analyzer_bin = fake_analyzer.clone();

    let err = run_pipeline(cfg)
        .await
        .expect_err("pre-version-flag analyzer must produce BinaryVersionUnreadable");
    match err {
        RunError::BinaryVersionUnreadable { which, path, .. } => {
            assert_eq!(which, BinaryKind::Analyzer);
            assert_eq!(path, fake_analyzer);
        }
        other => panic!("expected BinaryVersionUnreadable, got {other:?}"),
    }
}

/// Sanity check: when both binaries are the real workspace build, the
/// version check passes and the pipeline proceeds normally (this run
/// will reach the registry-load stage; we don't assert on the full
/// success because the polyglot.rs file already covers that — we just
/// want to confirm we did not break the green path).
#[tokio::test]
async fn matching_versions_do_not_short_circuit_the_runner() {
    ensure_engine_binaries();
    let temp = tempfile::tempdir().unwrap();
    let cfg = pipeline_config(temp.path(), Uuid::new_v4(), vec!["rust".to_string()]);
    // The fixture under polyglot-tiny is intentionally tiny but real;
    // any error here should NOT be a `BinaryVersionMismatch` /
    // `BinaryVersionUnreadable`.
    let result = run_pipeline(cfg).await;
    match result {
        Ok(_) => {}
        Err(RunError::BinaryVersionMismatch { .. })
        | Err(RunError::BinaryVersionUnreadable { .. }) => {
            panic!("matching real binaries must not fail the version check; got: {result:?}")
        }
        Err(_) => {
            // Other RunError variants are out of scope for this test
            // (polyglot.rs is the green-path test). We only assert
            // that the version-check leg does not regress.
        }
    }
    // Touch `workspace_root` so a future refactor that drops it from
    // `common.rs` flips a compile error here instead of silently
    // breaking the integration test scaffolding.
    let _ = workspace_root();
}
