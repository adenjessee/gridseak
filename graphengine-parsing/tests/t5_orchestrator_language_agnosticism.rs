//! T5 behavioural regression gate.
//!
//! The T5 design at
//! `docs/workstreams/universal-fidelity/tasks/T5-orchestrator-collapse.md`
//! §6.2 promised two named integration assertions that were not
//! actually written during T5 implementation:
//!
//! * `non_apex_parse_emits_no_apex_hook_log_lines`
//! * `apex_parse_still_emits_hook_log_lines`
//!
//! This file closes that promise. Both assertions operate at the
//! [`SyntaxExtractor::post_syntax_hooks`] boundary — the exact seam
//! the orchestrator now dispatches through — rather than driving the
//! full [`ParseRepoUseCase`] (which would pull in the async runtime
//! and persistence layer for no extra signal on T5's specific
//! contract).
//!
//! # What failure shapes these catch
//!
//! * **Apex-hook bleed into a non-Apex parse.** If someone wires the
//!   VF stage or framework-entry-point propagation into a shared
//!   dispatch path (e.g., by default-implementing them in the
//!   `SyntaxExtractor` port instead of the Apex override), the
//!   non-Apex test's `HookOutcome::NoOp` expectation breaks.
//! * **Silent Apex-hook regression.** If the Apex override stops
//!   running VF extraction or propagation (e.g., a refactor drops
//!   the second stage from `ApexExtractor::post_syntax_hooks`), the
//!   Apex test's summary-substring assertions break.

use graphengine_parsing::application::ports::{SyntaxExtractor, SyntaxResults};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::language::extractor::HookOutcome;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::{Path, PathBuf};

fn apex_vf_fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("apex_resolver")
        .join("r23_a5_vf")
        .join("util_jobprogress")
}

fn nightly_batch_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("apex")
        .join("force-app")
        .join("main")
        .join("default")
        .join("classes")
        .join("NightlyBatch.cls")
}

#[tokio::test]
async fn non_apex_parse_emits_no_apex_hook_log_lines() {
    // A language that does not override `post_syntax_hooks` must
    // inherit the default `HookOutcome::NoOp`. Python is representative;
    // every other non-Apex stock extractor is covered by the
    // parameterised `post_syntax_hooks_default` unit test in
    // `syntax::language::extractor`. This test's job is different:
    // it locks the property end-to-end through `TreeSitterExtractor`
    // (the real `SyntaxExtractor` implementation that the orchestrator
    // consumes), so a regression that only appears at the port-level
    // delegation — not at the trait-level default — still surfaces.

    let config = load_config("python").expect("load python config");
    let extractor = TreeSitterExtractor::new(config).expect("build python extractor");

    let mut results = SyntaxResults::new();
    let outcome = extractor.post_syntax_hooks(Path::new("/tmp/fake-root-python"), &mut results);

    assert_eq!(
        outcome,
        HookOutcome::NoOp,
        "non-Apex languages must inherit `HookOutcome::NoOp`; got: {outcome:?}"
    );

    // Defence-in-depth: the summary channel Apex uses to report its
    // hook results is the `Ok { summary }` variant. A regression that
    // widens the default to e.g. `Ok { summary: None }` would still
    // parse as "no Apex strings leaked" by a naive substring test but
    // would change the observable orchestrator log shape. The exact-
    // variant assertion above catches that; this redundant shape check
    // documents the intent.
    match outcome {
        HookOutcome::NoOp => {}
        other => panic!("expected `NoOp`; got {other:?}"),
    }
}

#[tokio::test]
async fn apex_parse_still_emits_hook_log_lines() {
    // Positive-case canary: the Apex `post_syntax_hooks` override
    // must still execute both the Visualforce extraction stage and
    // the framework-entry-point propagation stage, and its combined
    // summary must still contain the two substrings the orchestrator
    // logs as `info!("post-syntax hooks: {summary}")`.

    // Merge two fixtures so both post-syntax stages have something
    // to report:
    //
    // * `util_jobprogress` supplies the `.page` + controller that
    //   exercises VF extraction (emits the "VF extraction:" line).
    // * `NightlyBatch` directly implements `Database.Batchable` and
    //   `Schedulable`, so framework-entry-point propagation visits
    //   its contract methods and emits the "framework entry-point
    //   propagation:" line (under `function_nodes_already_tagged`
    //   even though the AST-local path also tagged them — the
    //   summary is conditioned on either counter being non-zero).
    let dir = apex_vf_fixture_dir();
    let mut cls_paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("cls"))
        .collect();
    cls_paths.push(nightly_batch_fixture());
    cls_paths.sort();
    assert!(
        !cls_paths.is_empty(),
        "apex VF fixture produced no .cls files under {}",
        dir.display()
    );

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");

    let mut results = extractor.extract(&cls_paths).await.expect("parse apex");
    results.set_workspace_root(dir.to_string_lossy().to_string());

    let outcome = extractor.post_syntax_hooks(&dir, &mut results);

    match outcome {
        HookOutcome::Ok { summary: Some(s) } => {
            assert!(
                s.contains("VF extraction:"),
                "Apex summary missing VF extraction substring; got: {s:?}"
            );
            assert!(
                s.contains("framework entry-point propagation:"),
                "Apex summary missing framework entry-point propagation substring; got: {s:?}"
            );
        }
        HookOutcome::Ok { summary: None } => panic!(
            "Apex hook emitted `Ok {{ summary: None }}` — VF fixture should have produced a summary"
        ),
        HookOutcome::NoOp => panic!(
            "Apex hook returned `NoOp` — the override regressed to the default. \
             Verify `ApexExtractor::post_syntax_hooks` still runs the two stages."
        ),
        HookOutcome::Warning { message } => {
            panic!("Apex hook returned `Warning` on a known-good fixture — stage failed: {message}")
        }
    }
}
