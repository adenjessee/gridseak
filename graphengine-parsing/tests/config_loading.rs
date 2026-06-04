//! Tests for configuration loading across different languages

use graphengine_parsing::infrastructure::config::load_config;

#[test]
fn test_rust_config_loading() {
    let config = load_config("rust");
    assert!(
        config.is_ok(),
        "Failed to load Rust config: {:?}",
        config.err()
    );

    let config = config.unwrap();
    assert_eq!(config.language, "rust");
    assert!(config.file_extensions.contains(&".rs".to_string()));
    assert_eq!(config.lsp_command, Some("rust-analyzer".to_string()));
}

#[test]
fn test_javascript_config_loading() {
    let config = load_config("javascript");
    assert!(
        config.is_ok(),
        "Failed to load JavaScript config: {:?}",
        config.err()
    );

    let config = config.unwrap();
    assert_eq!(config.language, "javascript");
    assert!(config.file_extensions.contains(&".js".to_string()));
    assert!(config.file_extensions.contains(&".jsx".to_string()));
    // NOTE: .ts and .tsx should NOT be in JavaScript config - they're handled by typescript.yaml
    assert!(
        !config.file_extensions.contains(&".ts".to_string()),
        "TypeScript files should be handled by typescript.yaml, not javascript.yaml"
    );
    assert_eq!(
        config.lsp_command,
        Some("typescript-language-server".to_string())
    );
}

#[test]
fn test_python_config_loading() {
    let config = load_config("python");
    assert!(
        config.is_ok(),
        "Failed to load Python config: {:?}",
        config.err()
    );

    let config = config.unwrap();
    assert_eq!(config.language, "python");
    assert!(config.file_extensions.contains(&".py".to_string()));
    assert_eq!(config.lsp_command, Some("pyright".to_string()));
}

#[test]
fn test_unsupported_language() {
    let config = load_config("unsupported");
    assert!(
        config.is_err(),
        "Should fail to load unsupported language config"
    );
}

#[test]
fn test_config_queries() {
    let config = load_config("rust").unwrap();

    // Check that key queries are present
    assert!(config.queries.contains_key("functions"));
    assert!(config.queries.contains_key("structs"));
    assert!(config.queries.contains_key("call_sites"));

    // Check that kind mappings are present
    assert!(config.kind_mappings.contains_key("function_item"));
    assert!(config.kind_mappings.contains_key("struct_item"));
}

#[test]
fn test_javascript_queries() {
    let config = load_config("javascript").unwrap();

    // Check that key queries are present
    assert!(config.queries.contains_key("functions"));
    assert!(config.queries.contains_key("classes"));
    assert!(config.queries.contains_key("call_sites"));
    assert!(config.queries.contains_key("imports"));

    // Check that kind mappings are present
    assert!(config.kind_mappings.contains_key("function_declaration"));
    assert!(config.kind_mappings.contains_key("class_declaration"));
}

#[test]
fn test_python_queries() {
    let config = load_config("python").unwrap();

    // Check that key queries are present
    assert!(config.queries.contains_key("functions"));
    assert!(config.queries.contains_key("classes"));
    assert!(config.queries.contains_key("call_sites"));
    assert!(config.queries.contains_key("imports"));

    // Check that kind mappings are present
    assert!(config.kind_mappings.contains_key("function_definition"));
    assert!(config.kind_mappings.contains_key("class_definition"));
}
