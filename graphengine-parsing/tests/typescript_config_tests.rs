//! TypeScript Config Loading and Query Validation Tests
//!
//! Infrastructure tests that verify:
//! 1. typescript.yaml config loads without error
//! 2. Config contains expected queries
//! 3. Kind mappings are complete
//! 4. Tree-sitter queries parse correctly

use graphengine_parsing::domain::NodeKind;
use graphengine_parsing::infrastructure::config::load_config;

/// Get the TypeScript configuration
fn get_typescript_config() -> graphengine_parsing::infrastructure::config::LanguageConfig {
    // load_config expects just the language name, not a path
    // It looks for configs/{language}.yaml
    load_config("typescript").expect("Failed to load typescript config")
}

// =============================================================================
// Config Loading Tests
// =============================================================================

#[test]
fn test_typescript_config_loads_without_error() {
    let config = get_typescript_config();
    assert_eq!(config.language, "typescript");
}

#[test]
fn test_typescript_config_has_correct_file_extensions() {
    let config = get_typescript_config();

    let extensions: Vec<&str> = config.file_extensions.iter().map(|s| s.as_str()).collect();

    assert!(extensions.contains(&".ts"), "Missing .ts extension");
    assert!(extensions.contains(&".tsx"), "Missing .tsx extension");
    assert!(extensions.contains(&".mts"), "Missing .mts extension");
    assert!(extensions.contains(&".cts"), "Missing .cts extension");
}

#[test]
fn test_typescript_config_has_lsp_settings() {
    let config = get_typescript_config();

    assert_eq!(
        config.lsp_command.as_deref(),
        Some("typescript-language-server")
    );
    assert!(config
        .lsp_args
        .as_ref()
        .map(|args| args.contains(&"--stdio".to_string()))
        .unwrap_or(false));
}

#[test]
fn test_typescript_config_has_version() {
    let config = get_typescript_config();
    assert!(!config.version.is_empty());
}

// =============================================================================
// Query Presence Tests
// =============================================================================

#[test]
fn test_typescript_config_has_functions_query() {
    let config = get_typescript_config();
    assert!(
        config.queries.contains_key("functions"),
        "Missing functions query"
    );
    let query = config.queries.get("functions").unwrap();
    assert!(!query.is_empty(), "Functions query is empty");
}

#[test]
fn test_typescript_config_has_structs_query() {
    let config = get_typescript_config();
    assert!(
        config.queries.contains_key("structs"),
        "Missing structs query (required by validation)"
    );
}

#[test]
fn test_typescript_config_has_modules_query() {
    let config = get_typescript_config();
    assert!(
        config.queries.contains_key("modules"),
        "Missing modules query (required by validation)"
    );
}

#[test]
fn test_typescript_config_has_classes_query() {
    let config = get_typescript_config();
    assert!(
        config.queries.contains_key("classes"),
        "Missing classes query"
    );
}

#[test]
fn test_typescript_config_has_interfaces_query() {
    let config = get_typescript_config();
    assert!(
        config.queries.contains_key("interfaces"),
        "Missing interfaces query"
    );
}

#[test]
fn test_typescript_config_has_call_sites_query() {
    let config = get_typescript_config();
    assert!(
        config.queries.contains_key("call_sites"),
        "Missing call_sites query"
    );
}

#[test]
fn test_typescript_config_has_imports_query() {
    let config = get_typescript_config();
    assert!(
        config.queries.contains_key("imports"),
        "Missing imports query"
    );
}

#[test]
fn test_typescript_config_has_type_refs_query() {
    let config = get_typescript_config();
    assert!(
        config.queries.contains_key("type_refs"),
        "Missing type_refs query"
    );
}

// =============================================================================
// Kind Mappings Tests
// =============================================================================

#[test]
fn test_typescript_config_has_function_declaration_mapping() {
    let config = get_typescript_config();
    assert_eq!(
        config.kind_mappings.get("function_declaration"),
        Some(&NodeKind::Function)
    );
}

#[test]
fn test_typescript_config_has_method_definition_mapping() {
    let config = get_typescript_config();
    assert_eq!(
        config.kind_mappings.get("method_definition"),
        Some(&NodeKind::Function)
    );
}

#[test]
fn test_typescript_config_has_class_declaration_mapping() {
    let config = get_typescript_config();
    assert_eq!(
        config.kind_mappings.get("class_declaration"),
        Some(&NodeKind::Struct)
    );
}

#[test]
fn test_typescript_config_has_interface_declaration_mapping() {
    let config = get_typescript_config();
    assert_eq!(
        config.kind_mappings.get("interface_declaration"),
        Some(&NodeKind::Interface)
    );
}

#[test]
fn test_typescript_config_has_import_statement_mapping() {
    let config = get_typescript_config();
    assert_eq!(
        config.kind_mappings.get("import_statement"),
        Some(&NodeKind::Import)
    );
}

#[test]
fn test_typescript_config_has_all_required_kind_mappings() {
    let config = get_typescript_config();

    let required_mappings = [
        "function_declaration",
        "method_definition",
        "arrow_function",
        "class_declaration",
        "interface_declaration",
        "import_statement",
        "variable_declaration",
        "lexical_declaration",
    ];

    for mapping in required_mappings.iter() {
        assert!(
            config.kind_mappings.contains_key(*mapping),
            "Missing kind mapping for: {}",
            mapping
        );
    }
}

// =============================================================================
// Tree-sitter Query Syntax Validation Tests
// =============================================================================

/// Helper to create TypeScript tree-sitter language
fn get_typescript_language() -> tree_sitter::Language {
    tree_sitter_typescript::language_typescript()
}

#[test]
fn test_functions_query_parses_without_syntax_error() {
    let config = get_typescript_config();
    let lang = get_typescript_language();

    let query_str = config.queries.get("functions").unwrap();
    let result = tree_sitter::Query::new(lang, query_str);

    assert!(
        result.is_ok(),
        "Functions query has syntax error: {:?}",
        result.err()
    );
}

#[test]
fn test_classes_query_parses_without_syntax_error() {
    let config = get_typescript_config();
    let lang = get_typescript_language();

    let query_str = config.queries.get("classes").unwrap();
    let result = tree_sitter::Query::new(lang, query_str);

    assert!(
        result.is_ok(),
        "Classes query has syntax error: {:?}",
        result.err()
    );
}

#[test]
fn test_interfaces_query_parses_without_syntax_error() {
    let config = get_typescript_config();
    let lang = get_typescript_language();

    let query_str = config.queries.get("interfaces").unwrap();
    let result = tree_sitter::Query::new(lang, query_str);

    assert!(
        result.is_ok(),
        "Interfaces query has syntax error: {:?}",
        result.err()
    );
}

#[test]
fn test_call_sites_query_parses_without_syntax_error() {
    let config = get_typescript_config();
    let lang = get_typescript_language();

    let query_str = config.queries.get("call_sites").unwrap();
    let result = tree_sitter::Query::new(lang, query_str);

    assert!(
        result.is_ok(),
        "Call sites query has syntax error: {:?}",
        result.err()
    );
}

#[test]
fn test_imports_query_parses_without_syntax_error() {
    let config = get_typescript_config();
    let lang = get_typescript_language();

    let query_str = config.queries.get("imports").unwrap();
    let result = tree_sitter::Query::new(lang, query_str);

    assert!(
        result.is_ok(),
        "Imports query has syntax error: {:?}",
        result.err()
    );
}

#[test]
fn test_type_refs_query_parses_without_syntax_error() {
    let config = get_typescript_config();
    let lang = get_typescript_language();

    let query_str = config.queries.get("type_refs").unwrap();
    let result = tree_sitter::Query::new(lang, query_str);

    assert!(
        result.is_ok(),
        "Type refs query has syntax error: {:?}",
        result.err()
    );
}

// =============================================================================
// Query Capture Tests - Verify queries capture expected constructs
// =============================================================================

/// Helper to parse TypeScript code and run a query
fn run_query_on_code(code: &str, query_str: &str) -> Vec<String> {
    let lang = get_typescript_language();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(lang).unwrap();

    let tree = parser.parse(code, None).unwrap();
    let query = tree_sitter::Query::new(lang, query_str).unwrap();

    let mut cursor = tree_sitter::QueryCursor::new();
    let matches = cursor.matches(&query, tree.root_node(), code.as_bytes());

    let mut captured = Vec::new();
    for mat in matches {
        for capture in mat.captures {
            let text = capture.node.utf8_text(code.as_bytes()).unwrap();
            captured.push(text.to_string());
        }
    }
    captured
}

#[test]
fn test_functions_query_captures_named_function() {
    let config = get_typescript_config();
    let query_str = config.queries.get("functions").unwrap();

    let code = r#"
function greet(name: string): string {
    return "Hello, " + name;
}
"#;

    let captured = run_query_on_code(code, query_str);
    assert!(
        captured.iter().any(|s| s == "greet"),
        "Should capture function name 'greet'. Captured: {:?}",
        captured
    );
}

#[test]
fn test_functions_query_captures_method_definition() {
    let config = get_typescript_config();
    let query_str = config.queries.get("functions").unwrap();

    let code = r#"
class Service {
    getData(): string {
        return "data";
    }
}
"#;

    let captured = run_query_on_code(code, query_str);
    assert!(
        captured.iter().any(|s| s == "getData"),
        "Should capture method name 'getData'. Captured: {:?}",
        captured
    );
}

#[test]
fn test_functions_query_captures_arrow_function() {
    let config = get_typescript_config();
    let query_str = config.queries.get("functions").unwrap();

    let code = r#"
const add = (a: number, b: number): number => a + b;
"#;

    let captured = run_query_on_code(code, query_str);
    assert!(
        captured.iter().any(|s| s == "add"),
        "Should capture arrow function name 'add'. Captured: {:?}",
        captured
    );
}

#[test]
fn test_classes_query_captures_class_declaration() {
    let config = get_typescript_config();
    let query_str = config.queries.get("classes").unwrap();

    let code = r#"
class UserService {
    private users: string[] = [];
}
"#;

    let captured = run_query_on_code(code, query_str);
    assert!(
        captured.iter().any(|s| s == "UserService"),
        "Should capture class name 'UserService'. Captured: {:?}",
        captured
    );
}

#[test]
fn test_interfaces_query_captures_interface_declaration() {
    let config = get_typescript_config();
    let query_str = config.queries.get("interfaces").unwrap();

    let code = r#"
interface User {
    id: number;
    name: string;
}
"#;

    let captured = run_query_on_code(code, query_str);
    assert!(
        captured.iter().any(|s| s == "User"),
        "Should capture interface name 'User'. Captured: {:?}",
        captured
    );
}

#[test]
fn test_call_sites_query_captures_function_call() {
    let config = get_typescript_config();
    let query_str = config.queries.get("call_sites").unwrap();

    let code = r#"
const result = calculateTotal(items);
"#;

    let captured = run_query_on_code(code, query_str);
    assert!(
        captured.iter().any(|s| s == "calculateTotal"),
        "Should capture function call 'calculateTotal'. Captured: {:?}",
        captured
    );
}

#[test]
fn test_call_sites_query_captures_method_call() {
    let config = get_typescript_config();
    let query_str = config.queries.get("call_sites").unwrap();

    let code = r#"
const user = userService.getUser(123);
"#;

    let captured = run_query_on_code(code, query_str);
    assert!(
        captured.iter().any(|s| s == "getUser"),
        "Should capture method call 'getUser'. Captured: {:?}",
        captured
    );
}

#[test]
fn test_call_sites_query_captures_constructor_call() {
    let config = get_typescript_config();
    let query_str = config.queries.get("call_sites").unwrap();

    let code = r#"
const service = new UserService();
"#;

    let captured = run_query_on_code(code, query_str);
    assert!(
        captured.iter().any(|s| s == "UserService"),
        "Should capture constructor call 'UserService'. Captured: {:?}",
        captured
    );
}

// =============================================================================
// Negative Tests - Verify queries don't capture wrong constructs
// =============================================================================

#[test]
fn test_functions_query_ignores_function_call() {
    let config = get_typescript_config();
    let query_str = config.queries.get("functions").unwrap();

    // This code only has function calls, no function declarations
    let code = r#"
processData();
handleResult(value);
"#;

    let captured = run_query_on_code(code, query_str);
    // Should not capture the function calls as function declarations
    assert!(
        captured.is_empty()
            || !captured
                .iter()
                .any(|s| s == "processData" || s == "handleResult"),
        "Should NOT capture function calls as declarations. Captured: {:?}",
        captured
    );
}

#[test]
fn test_classes_query_ignores_interface() {
    let config = get_typescript_config();
    let query_str = config.queries.get("classes").unwrap();

    let code = r#"
interface NotAClass {
    method(): void;
}
"#;

    let captured = run_query_on_code(code, query_str);
    assert!(
        !captured.iter().any(|s| s == "NotAClass"),
        "Should NOT capture interface as class. Captured: {:?}",
        captured
    );
}
