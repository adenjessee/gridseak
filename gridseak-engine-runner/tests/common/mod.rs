//! Shared scaffolding for the runner's integration tests.
//!
//! Each integration test in this crate (`parity.rs`, `polyglot.rs`)
//! needs the same three things to drive a real pipeline run:
//!
//! 1. A path to the workspace root, so we can locate `target/debug/`
//!    binaries and the `graphengine-parsing/configs/` directory.
//! 2. A guarantee that the engine binaries (`graphengine-parsing`,
//!    `ge-analyze`) are built before the test invokes them. `cargo test
//!    --workspace` does *not* schedule sibling-crate `[[bin]]` builds
//!    deterministically, so we run `cargo build -p <name>` ourselves
//!    once per test process (it's idempotent — instant on warm cache).
//! 3. A configured `RunPipelineConfig` pointing at a temp scratch dir
//!    and the `polyglot-tiny` fixture, so each test can override the
//!    fields it needs (languages, parser_bin) without re-deriving the
//!    rest.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use gridseak_engine_runner::{progress::DiscardSink, RunPipelineConfig};
use uuid::Uuid;

/// Returns the absolute path of the workspace root (the directory that
/// contains the top-level `Cargo.toml`). We compute it from
/// `CARGO_MANIFEST_DIR` (which points at this crate) rather than
/// hard-coding a relative path so the helper survives any future move
/// of the runner crate inside the workspace.
pub fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .expect("crate manifest must have a parent")
        .to_path_buf()
}

/// Path to the on-disk fixture used by every integration test.
pub fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("polyglot-tiny")
}

/// Path to `graphengine-parsing/configs/` (where `parser --configs-dir`
/// expects to find language YAML descriptors).
pub fn configs_dir() -> PathBuf {
    workspace_root().join("graphengine-parsing").join("configs")
}

/// Path to a built engine binary in `target/debug/`. Returns the path
/// regardless of whether the binary currently exists — callers who
/// need the binary present should call [`ensure_engine_binaries`] first.
pub fn target_debug_bin(name: &str) -> PathBuf {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    workspace_root().join("target").join("debug").join(exe)
}

/// Build `graphengine-parsing` and `ge-analyze` if they aren't already
/// built. Idempotent and safe to call from every test — the work runs
/// at most once per test process.
///
/// We invoke `cargo build` rather than asserting the binaries exist
/// because `cargo test --workspace` does not currently schedule
/// sibling-crate binary builds before running this crate's tests; the
/// build IS scheduled eventually, but not necessarily before our
/// integration tests start.
pub fn ensure_engine_binaries() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let status = Command::new(env!("CARGO"))
            .arg("build")
            .arg("-p")
            .arg("graphengine-parsing")
            .arg("-p")
            .arg("graphengine-analysis")
            .arg("--bin")
            .arg("graphengine-parsing")
            .arg("--bin")
            .arg("ge-analyze")
            .current_dir(workspace_root())
            .status()
            .expect("failed to invoke cargo to build engine binaries");
        assert!(
            status.success(),
            "cargo build of engine binaries failed: {status}"
        );
    });
}

/// Construct a `RunPipelineConfig` pre-wired to the polyglot-tiny
/// fixture, real engine binaries, the workspace configs dir, and the
/// caller-supplied scratch dir. The caller selects `languages` and may
/// override any field on the returned struct before driving the runner.
pub fn pipeline_config(scratch: &Path, scan_id: Uuid, languages: Vec<String>) -> RunPipelineConfig {
    RunPipelineConfig {
        root: fixture_root(),
        languages,
        parser_bin: target_debug_bin("graphengine-parsing"),
        analyzer_bin: target_debug_bin("ge-analyze"),
        configs_dir: configs_dir(),
        scratch_dir: scratch.to_path_buf(),
        // Integration-test default: ephemeral per-scan DB (legacy
        // behaviour). Tests that want to exercise S1-ε persistence
        // should override this field with a stable tempfile path.
        persistent_parse_db: None,
        scan_id,
        exclude_tests: true,
        exclude_generated: true,
        // S1-ε: incremental defaults to ON to mirror the CLI's
        // shipping default. Tests that want a verifiably-cold
        // pipeline should override with `incremental: false`.
        incremental: true,
        full_analysis: false,
        // Fixture is not a git repo, so temporal-coupling signals are off.
        // That's intentional: keeping `git_dir = None` means the parity
        // test isn't subject to "what happens if the on-disk .git/HEAD
        // changes between runs" non-determinism.
        git_dir: None,
        progress: Box::new(DiscardSink),
        cancel: None,
    }
}
