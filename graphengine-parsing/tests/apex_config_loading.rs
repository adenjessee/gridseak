//! Apex configuration loading and query-compilation tests.
//!
//! Verifies the Apex language config:
//! 1. Loads and validates against the schema (no missing required queries, no
//!    invalid YAML).
//! 2. Advertises the expected file extensions (`.cls`, `.trigger`, `.apxc`).
//! 3. Carries the full Salesforce `apex-jorje` LSP wiring — `lsp_command`,
//!    `lsp_args`, the JAR placeholder the Apex branch of `command_locator.rs`
//!    rewrites, `lsp_initialization_options` with `enableSemanticErrors`, and
//!    the extended 15s request timeout.
//! 4. Compiles every tree-sitter query string against the vendored Apex
//!    grammar — catches query drift before it reaches production.

use graphengine_parsing::infrastructure::config::load_config;

#[test]
fn apex_config_loads_and_validates() {
    let config = load_config("apex").expect("Failed to load Apex config");

    assert_eq!(config.language, "apex");
    assert!(config.file_extensions.contains(&".cls".to_string()));
    assert!(config.file_extensions.contains(&".trigger".to_string()));
    assert!(config.file_extensions.contains(&".apxc".to_string()));
}

#[test]
fn apex_config_advertises_jorje_lsp() {
    let config = load_config("apex").expect("Failed to load Apex config");

    assert_eq!(config.lsp_command.as_deref(), Some("java"));

    let args = config
        .lsp_args
        .as_ref()
        .expect("Apex lsp_args must be declared for command_locator rewrite");

    // The exact placeholder the Apex branch of command_locator.rs rewrites
    // into a real JAR path. If this changes, command_locator must change too.
    assert!(
        args.iter().any(|a| a == "APEX_JORJE_JAR_PLACEHOLDER"),
        "Apex lsp_args must contain the jar placeholder, got: {:?}",
        args
    );

    assert!(
        args.iter()
            .any(|a| a == "apex.jorje.lsp.ApexLanguageServerLauncher"),
        "Apex lsp_args must invoke the apex-jorje launcher class"
    );
}

#[test]
fn apex_config_enables_semantic_errors_at_init() {
    let config = load_config("apex").expect("Failed to load Apex config");

    let init = config.lsp_initialization_options.as_ref().expect(
        "apex requires lsp_initialization_options to trigger the \
                 with-options initialize path in SimpleLspClient",
    );

    let enabled = init
        .get("enableSemanticErrors")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    assert!(
        enabled,
        "apex-jorje semantic analysis must be enabled for full type resolution"
    );
}

#[test]
fn apex_config_has_extended_request_timeout() {
    let config = load_config("apex").expect("Failed to load Apex config");
    // apex-jorje is slower than native LSPs — 15s lets it complete workspace
    // scans on large NPSP-scale orgs.
    let timeout = config
        .lsp_request_timeout_ms
        .expect("apex requires explicit lsp_request_timeout_ms");
    assert!(
        timeout >= 10_000,
        "apex-jorje needs at least 10s per request; got {timeout}ms"
    );
}

#[test]
fn apex_config_all_queries_compile_against_vendored_grammar() {
    let config = load_config("apex").expect("Failed to load Apex config");
    let language = tree_sitter_sfapex_vendored::apex::language();

    for (name, query_str) in &config.queries {
        let result = tree_sitter::Query::new(language, query_str);
        assert!(
            result.is_ok(),
            "Apex query '{name}' failed to compile against the vendored \
             Apex grammar: {:?}\n--- query ---\n{query_str}\n-------------",
            result.err()
        );
    }
}
