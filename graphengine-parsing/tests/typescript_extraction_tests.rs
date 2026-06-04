//! TypeScript Syntax Extraction Application Tests
//!
//! Tests that verify TypeScript syntax is correctly extracted using real tree-sitter
//! parsing on test fixtures. These are integration tests that use real implementations.

use graphengine_parsing::application::ports::{SyntaxExtractor, SyntaxResults};
use graphengine_parsing::domain::{Node, NodeKind, Provenance, Range};
use graphengine_parsing::infrastructure::config::load_config;
use std::path::PathBuf;
use tempfile::TempDir;

/// Simple TypeScript extractor for testing purposes
/// Uses tree-sitter-typescript directly with our config queries
struct TypeScriptTestExtractor {
    config: graphengine_parsing::infrastructure::config::LanguageConfig,
}

impl TypeScriptTestExtractor {
    fn new() -> Self {
        let config = load_config("typescript").expect("Failed to load typescript config");
        Self { config }
    }

    fn parse_file(&self, path: &std::path::Path, content: &str) -> SyntaxResults {
        let lang = tree_sitter_typescript::language_typescript();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(lang).expect("Failed to set language");

        let tree = parser.parse(content, None).expect("Failed to parse");
        let mut results = SyntaxResults::new();
        let file_path = path.to_string_lossy().to_string();

        // Extract functions
        if let Some(query_str) = self.config.queries.get("functions") {
            self.extract_symbols(
                &tree,
                content,
                query_str,
                NodeKind::Function,
                &file_path,
                &mut results,
            );
        }

        // Extract classes
        if let Some(query_str) = self.config.queries.get("classes") {
            self.extract_symbols(
                &tree,
                content,
                query_str,
                NodeKind::Struct,
                &file_path,
                &mut results,
            );
        }

        // Extract interfaces
        if let Some(query_str) = self.config.queries.get("interfaces") {
            self.extract_symbols(
                &tree,
                content,
                query_str,
                NodeKind::Interface,
                &file_path,
                &mut results,
            );
        }

        // Extract call sites
        if let Some(query_str) = self.config.queries.get("call_sites") {
            self.extract_call_sites(&tree, content, query_str, &file_path, &mut results);
        }

        results
    }

    fn extract_symbols(
        &self,
        tree: &tree_sitter::Tree,
        content: &str,
        query_str: &str,
        kind: NodeKind,
        file_path: &str,
        results: &mut SyntaxResults,
    ) {
        let lang = tree_sitter_typescript::language_typescript();
        let query = match tree_sitter::Query::new(lang, query_str) {
            Ok(q) => q,
            Err(_) => return,
        };

        let mut cursor = tree_sitter::QueryCursor::new();
        let matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

        for mat in matches {
            let mut name = None;
            let mut range = None;

            for capture in mat.captures {
                let capture_name = &query.capture_names()[capture.index as usize];
                match capture_name.as_str() {
                    "name" => {
                        name = Some(
                            capture
                                .node
                                .utf8_text(content.as_bytes())
                                .unwrap_or("")
                                .to_string(),
                        );
                    }
                    "func" | "class" | "interface" | "struct" => {
                        let start = capture.node.start_position();
                        let end = capture.node.end_position();
                        range = Some(Range::with_file(
                            start.row as u32 + 1,
                            start.column as u32,
                            end.row as u32 + 1,
                            end.column as u32,
                            file_path.to_string(),
                        ));
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(range)) = (name, range) {
                let fqn = graphengine_parsing::syntax::utils::typescript_fqn::build_typescript_fqn(
                    &name, file_path,
                );
                let node = Node::new(kind, fqn, range, Provenance::tree_sitter());
                results.add_symbol(node);
            }
        }
    }

    fn extract_call_sites(
        &self,
        tree: &tree_sitter::Tree,
        content: &str,
        query_str: &str,
        file_path: &str,
        results: &mut SyntaxResults,
    ) {
        let lang = tree_sitter_typescript::language_typescript();
        let query = match tree_sitter::Query::new(lang, query_str) {
            Ok(q) => q,
            Err(_) => return,
        };

        let mut cursor = tree_sitter::QueryCursor::new();
        let matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

        for mat in matches {
            let mut function_name = None;
            let mut location = None;

            for capture in mat.captures {
                let capture_name = &query.capture_names()[capture.index as usize];
                match capture_name.as_str() {
                    "func" | "method" | "constructor" => {
                        function_name = Some(
                            capture
                                .node
                                .utf8_text(content.as_bytes())
                                .unwrap_or("")
                                .to_string(),
                        );
                    }
                    "call" | "method_call" | "constructor_call" => {
                        let start = capture.node.start_position();
                        let end = capture.node.end_position();
                        location = Some(Range::with_file(
                            start.row as u32 + 1,
                            start.column as u32,
                            end.row as u32 + 1,
                            end.column as u32,
                            file_path.to_string(),
                        ));
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(loc)) = (function_name, location) {
                results.push_call(graphengine_parsing::application::ports::CallSite {
                    location: loc,
                    function_name: name,
                    receiver_range: None,
                    receiver_text: None,
                    arg_types: Vec::new(),
                });
            }
        }
    }
}

#[async_trait::async_trait]
impl SyntaxExtractor for TypeScriptTestExtractor {
    async fn extract(&self, files: &[PathBuf]) -> anyhow::Result<SyntaxResults> {
        let mut combined_results = SyntaxResults::new();

        for file in files {
            if let Ok(content) = std::fs::read_to_string(file) {
                let results = self.parse_file(file, &content);
                combined_results.symbols.extend(results.symbols);
                combined_results.references.extend(results.references);
                combined_results.imports.extend(results.imports);
                combined_results.type_refs.extend(results.type_refs);
            }
        }

        Ok(combined_results)
    }

    fn supported_language(&self) -> &str {
        "typescript"
    }

    fn supports_extension(&self, ext: &str) -> bool {
        matches!(ext, ".ts" | ".tsx" | ".mts" | ".cts")
    }
}

// =============================================================================
// Single Function Tests
// =============================================================================

#[tokio::test]
async fn test_extract_single_function() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("utils.ts");

    std::fs::write(
        &file_path,
        r#"
export function calculateTotal(items: number[]): number {
    return items.reduce((sum, item) => sum + item, 0);
}
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    assert!(
        !results.symbols.is_empty(),
        "Should extract at least one function. Got: {:?}",
        results.symbols
    );

    let function = results
        .symbols
        .iter()
        .find(|s| s.kind == NodeKind::Function);
    assert!(function.is_some(), "Should find a function symbol");

    let func = function.unwrap();
    assert!(
        func.fqn.contains("calculateTotal"),
        "FQN should contain function name. Got: {}",
        func.fqn
    );
}

#[tokio::test]
async fn test_extract_arrow_function() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("arrow.ts");

    std::fs::write(
        &file_path,
        r#"
const add = (a: number, b: number): number => a + b;
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    let arrow_func = results.symbols.iter().find(|s| s.fqn.contains("add"));
    assert!(
        arrow_func.is_some(),
        "Should extract arrow function 'add'. Got: {:?}",
        results.symbols.iter().map(|s| &s.fqn).collect::<Vec<_>>()
    );
}

// =============================================================================
// Class Tests
// =============================================================================

#[tokio::test]
async fn test_extract_class_with_methods() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("service.ts");

    std::fs::write(
        &file_path,
        r#"
export class UserService {
    private users: Map<number, string> = new Map();

    getUser(id: number): string | undefined {
        return this.users.get(id);
    }

    createUser(name: string): number {
        const id = Date.now();
        this.users.set(id, name);
        return id;
    }
}
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    // Should have class node
    let class_node = results.symbols.iter().find(|s| s.kind == NodeKind::Struct);
    assert!(
        class_node.is_some(),
        "Should extract UserService class. Got: {:?}",
        results.symbols
    );

    // Should have method nodes
    let get_user = results.symbols.iter().find(|s| s.fqn.contains("getUser"));
    assert!(get_user.is_some(), "Should extract getUser method");

    let create_user = results
        .symbols
        .iter()
        .find(|s| s.fqn.contains("createUser"));
    assert!(create_user.is_some(), "Should extract createUser method");
}

// =============================================================================
// Interface Tests
// =============================================================================

#[tokio::test]
async fn test_extract_interface() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("types.ts");

    std::fs::write(
        &file_path,
        r#"
export interface User {
    id: number;
    name: string;
    email: string;
}

export interface UserRole {
    roleId: number;
    roleName: string;
}
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    let interfaces: Vec<_> = results
        .symbols
        .iter()
        .filter(|s| s.kind == NodeKind::Interface)
        .collect();

    assert_eq!(interfaces.len(), 2, "Should extract both interfaces");

    let user_interface = interfaces.iter().find(|s| s.fqn.contains("User"));
    assert!(user_interface.is_some(), "Should extract User interface");

    let role_interface = interfaces.iter().find(|s| s.fqn.contains("UserRole"));
    assert!(
        role_interface.is_some(),
        "Should extract UserRole interface"
    );
}

// =============================================================================
// Call Site Tests
// =============================================================================

#[tokio::test]
async fn test_extract_function_call_sites() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("caller.ts");

    std::fs::write(
        &file_path,
        r#"
function helper(): number {
    return 42;
}

function main(): void {
    const result = helper();
    console.log(result);
}
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    // Should have call sites
    assert!(
        !results.references.is_empty(),
        "Should extract call sites. Got: {:?}",
        results.references
    );

    let helper_call = results
        .iter_all_call_sites()
        .find(|c| c.function_name == "helper");
    assert!(helper_call.is_some(), "Should find call to 'helper'");
}

#[tokio::test]
async fn test_extract_method_call_sites() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("method_call.ts");

    std::fs::write(
        &file_path,
        r#"
class Service {
    getData(): string {
        return "data";
    }
}

const service = new Service();
const data = service.getData();
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    let method_call = results
        .iter_all_call_sites()
        .find(|c| c.function_name == "getData");
    assert!(
        method_call.is_some(),
        "Should find method call to 'getData'. Call sites: {:?}",
        results.references
    );
}

#[tokio::test]
async fn test_extract_constructor_call_sites() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("constructor.ts");

    std::fs::write(
        &file_path,
        r#"
class MyClass {
    constructor(public value: number) {}
}

const instance = new MyClass(42);
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    let constructor_call = results
        .iter_all_call_sites()
        .find(|c| c.function_name == "MyClass");
    assert!(
        constructor_call.is_some(),
        "Should find constructor call to 'MyClass'. Call sites: {:?}",
        results.references
    );
}

// =============================================================================
// Cross-File Tests
// =============================================================================

#[tokio::test]
async fn test_extract_multiple_files() {
    let temp_dir = TempDir::new().unwrap();

    let math_file = temp_dir.path().join("math.ts");
    std::fs::write(
        &math_file,
        r#"
export function add(a: number, b: number): number {
    return a + b;
}
"#,
    )
    .unwrap();

    let calc_file = temp_dir.path().join("calculator.ts");
    std::fs::write(
        &calc_file,
        r#"
import { add } from './math';

export function calculate(x: number, y: number): number {
    return add(x, y);
}
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(&[math_file.clone(), calc_file.clone()])
        .await
        .unwrap();

    // Should have symbols from both files
    let add_func = results.symbols.iter().find(|s| s.fqn.contains("add"));
    assert!(
        add_func.is_some(),
        "Should extract 'add' function from math.ts"
    );

    let calc_func = results.symbols.iter().find(|s| s.fqn.contains("calculate"));
    assert!(
        calc_func.is_some(),
        "Should extract 'calculate' function from calculator.ts"
    );

    // Should have call site from calculator.ts calling add
    let add_call = results
        .iter_all_call_sites()
        .find(|c| c.function_name == "add");
    assert!(
        add_call.is_some(),
        "Should find cross-file call to 'add'. Call sites: {:?}",
        results.references
    );
}

// =============================================================================
// Edge Cases
// =============================================================================

#[tokio::test]
async fn test_extract_empty_file() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("empty.ts");

    std::fs::write(&file_path, "").unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    assert!(
        results.symbols.is_empty(),
        "Empty file should produce no symbols"
    );
    assert!(
        results.references.is_empty(),
        "Empty file should produce no call sites"
    );
}

#[tokio::test]
async fn test_extract_comments_only() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("comments.ts");

    std::fs::write(
        &file_path,
        r#"
// This is a comment
/* This is a block comment */
/**
 * This is a JSDoc comment
 */
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    assert!(
        results.symbols.is_empty(),
        "Comments-only file should produce no symbols"
    );
}

#[tokio::test]
async fn test_extract_tsx_file() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("component.tsx");

    std::fs::write(
        &file_path,
        r#"
interface Props {
    name: string;
}

export function Greeting(props: Props): JSX.Element {
    return <div>Hello, {props.name}!</div>;
}
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    // Note: TSX may need different tree-sitter grammar (language_tsx)
    // This test verifies basic handling; full TSX support may need adjustment
    assert!(
        !results.symbols.is_empty(),
        "Should extract symbols from TSX file"
    );
}

// =============================================================================
// FQN Verification Tests
// =============================================================================

#[tokio::test]
async fn test_fqn_format_for_class() {
    let temp_dir = TempDir::new().unwrap();
    let nested_dir = temp_dir.path().join("src").join("auth");
    std::fs::create_dir_all(&nested_dir).unwrap();

    let file_path = nested_dir.join("auth.service.ts");
    std::fs::write(
        &file_path,
        r#"
export class AuthService {
    login(): void {}
}
"#,
    )
    .unwrap();

    let extractor = TypeScriptTestExtractor::new();
    let results = extractor
        .extract(std::slice::from_ref(&file_path))
        .await
        .unwrap();

    let class_node = results.symbols.iter().find(|s| s.kind == NodeKind::Struct);
    assert!(class_node.is_some(), "Should extract class");

    let fqn = &class_node.unwrap().fqn;
    // FQN should include the path structure
    assert!(
        fqn.contains("auth.service") || fqn.contains("auth/auth.service"),
        "FQN should include file path structure. Got: {}",
        fqn
    );
    assert!(
        fqn.contains("AuthService"),
        "FQN should include class name. Got: {}",
        fqn
    );
}

#[tokio::test]
async fn test_supports_extension() {
    let extractor = TypeScriptTestExtractor::new();

    assert!(extractor.supports_extension(".ts"), "Should support .ts");
    assert!(extractor.supports_extension(".tsx"), "Should support .tsx");
    assert!(extractor.supports_extension(".mts"), "Should support .mts");
    assert!(extractor.supports_extension(".cts"), "Should support .cts");

    assert!(
        !extractor.supports_extension(".js"),
        "Should not support .js"
    );
    assert!(
        !extractor.supports_extension(".rs"),
        "Should not support .rs"
    );
}
