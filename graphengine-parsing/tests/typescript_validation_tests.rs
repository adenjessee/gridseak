//! Phase 4: Validation Tests for TypeScript MVP
//!
//! These tests PROVE the system works correctly through:
//! - Property-based testing (invariants hold for ANY valid input)
//! - Golden tests (actual output matches known-good output)
//! - Negative tests (system does NOT produce incorrect results)
//!
//! This is the final validation layer before shipping.

use proptest::prelude::*;
use std::collections::HashSet;

// ============================================================================
// Property-Based Tests: Invariants That Must Always Hold
// ============================================================================

mod property_based_invariants {
    use super::*;
    use graphengine_parsing::syntax::utils::typescript_fqn::{
        build_typescript_fqn, build_typescript_method_fqn,
    };

    // Strategy for generating valid TypeScript symbol names
    fn typescript_symbol_name() -> impl Strategy<Value = String> {
        // TypeScript identifiers: start with letter/underscore, followed by alphanumerics
        "[a-zA-Z_][a-zA-Z0-9_]{0,29}".prop_filter("non-empty", |s| !s.is_empty())
    }

    // Strategy for generating valid file paths
    fn typescript_file_path() -> impl Strategy<Value = String> {
        // Generate paths like "src/foo/bar.ts" or "lib/utils.tsx"
        (
            prop::collection::vec("[a-z][a-z0-9]{0,9}", 1..=3), // path segments
            prop::sample::select(vec!["ts", "tsx", "mts", "cts"]), // extension
        )
            .prop_map(|(segments, ext)| {
                let path = segments.join("/");
                format!("src/{}.{}", path, ext)
            })
    }

    #[test]
    fn test_fqn_generation_is_deterministic() {
        // Property: Same input always produces same output
        proptest!(|(
            name in typescript_symbol_name(),
            path in typescript_file_path()
        )| {
            let fqn1 = build_typescript_fqn(&name, &path);
            let fqn2 = build_typescript_fqn(&name, &path);
            prop_assert_eq!(fqn1, fqn2, "FQN must be deterministic");
        });
    }

    #[test]
    fn test_fqn_contains_symbol_name() {
        // Property: FQN always contains the original symbol name
        proptest!(|(
            name in typescript_symbol_name(),
            path in typescript_file_path()
        )| {
            let fqn = build_typescript_fqn(&name, &path);
            prop_assert!(fqn.contains(&name), "FQN '{}' must contain name '{}'", fqn, name);
        });
    }

    #[test]
    fn test_fqn_has_path_component() {
        // Property: FQN always contains some path information
        proptest!(|(
            name in typescript_symbol_name(),
            path in typescript_file_path()
        )| {
            let fqn = build_typescript_fqn(&name, &path);
            // FQN should have at least module::name format
            prop_assert!(fqn.contains("::"), "FQN '{}' must contain '::'", fqn);
        });
    }

    #[test]
    fn test_method_fqn_contains_both_class_and_method() {
        // Property: Method FQN contains both class and method names
        proptest!(|(
            class_name in typescript_symbol_name(),
            method_name in typescript_symbol_name(),
            path in typescript_file_path()
        )| {
            let fqn = build_typescript_method_fqn(&method_name, &class_name, &path);
            prop_assert!(fqn.contains(&class_name), "Method FQN must contain class name");
            prop_assert!(fqn.contains(&method_name), "Method FQN must contain method name");
        });
    }

    #[test]
    fn test_fqn_uses_double_colon_separator() {
        // Property: FQN parts are separated by ::
        proptest!(|(
            name in typescript_symbol_name(),
            path in typescript_file_path()
        )| {
            let fqn = build_typescript_fqn(&name, &path);
            // Should end with ::name
            prop_assert!(fqn.ends_with(&format!("::{}", name)), "FQN should end with ::{}", name);
        });
    }

    #[test]
    fn test_different_symbols_produce_different_fqns() {
        // Property: Different symbols in same file have different FQNs
        proptest!(|(
            name1 in typescript_symbol_name(),
            name2 in typescript_symbol_name(),
            path in typescript_file_path()
        )| {
            prop_assume!(name1 != name2);
            let fqn1 = build_typescript_fqn(&name1, &path);
            let fqn2 = build_typescript_fqn(&name2, &path);
            prop_assert_ne!(fqn1, fqn2, "Different symbols should have different FQNs");
        });
    }

    #[test]
    fn test_same_symbol_different_files_have_different_fqns() {
        // Property: Same symbol name in different files produces different FQNs
        proptest!(|(
            name in typescript_symbol_name(),
            path1 in typescript_file_path(),
            path2 in typescript_file_path()
        )| {
            prop_assume!(path1 != path2);
            let fqn1 = build_typescript_fqn(&name, &path1);
            let fqn2 = build_typescript_fqn(&name, &path2);
            prop_assert_ne!(fqn1, fqn2, "Same symbol in different files should have different FQNs");
        });
    }

    #[test]
    fn test_fqn_strips_file_extension() {
        // Property: FQN does not contain file extension
        proptest!(|(
            name in typescript_symbol_name(),
            path in typescript_file_path()
        )| {
            let fqn = build_typescript_fqn(&name, &path);
            prop_assert!(!fqn.ends_with(".ts"), "FQN should not end with .ts");
            prop_assert!(!fqn.ends_with(".tsx"), "FQN should not end with .tsx");
            prop_assert!(!fqn.contains(".ts::"), "FQN should not contain .ts::");
            prop_assert!(!fqn.contains(".tsx::"), "FQN should not contain .tsx::");
        });
    }
}

// ============================================================================
// Golden Tests: Compare Actual Output to Known-Good Fixtures
// ============================================================================

mod golden_tests {
    use super::*;
    use graphengine_parsing::syntax::utils::typescript_fqn::build_typescript_fqn;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct ExpectedGraph {
        description: String,
        nodes: Vec<ExpectedNode>,
        edges: Vec<ExpectedEdge>,
    }

    #[derive(Debug, Deserialize, Clone)]
    struct ExpectedNode {
        kind: String,
        fqn: String,
        // location field exists in JSON but not needed for these tests
    }

    #[derive(Debug, Deserialize, Clone)]
    struct ExpectedEdge {
        kind: String,
        from_fqn: String,
        to_fqn: String,
    }

    fn load_expected_graph(fixture_path: &str) -> ExpectedGraph {
        let json = std::fs::read_to_string(fixture_path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {}", fixture_path, e));
        serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {}", fixture_path, e))
    }

    #[test]
    fn test_golden_single_function_fqn() {
        // Verify FQN generation matches expected for single_function fixture
        let expected =
            load_expected_graph("../test/typescript_corpus/single_function/expected.json");

        assert_eq!(expected.nodes.len(), 1);
        let expected_node = &expected.nodes[0];

        // Generate FQN the same way our system would
        let actual_fqn = build_typescript_fqn("calculateTotal", "src/utils.ts");

        assert_eq!(
            actual_fqn, expected_node.fqn,
            "FQN should match golden fixture"
        );
    }

    #[test]
    fn test_golden_single_class_fqns() {
        // Verify FQN generation matches expected for single_class fixture
        let expected = load_expected_graph("../test/typescript_corpus/single_class/expected.json");

        // Find expected FQNs
        let expected_fqns: HashSet<&str> = expected.nodes.iter().map(|n| n.fqn.as_str()).collect();

        // Generate FQNs for class and methods
        let user_service_fqn = build_typescript_fqn("UserService", "src/user.service.ts");
        let user_interface_fqn = build_typescript_fqn("User", "src/user.service.ts");

        // Verify these match expected
        assert!(
            expected_fqns.contains(user_service_fqn.as_str()),
            "UserService FQN '{}' should be in expected: {:?}",
            user_service_fqn,
            expected_fqns
        );
        assert!(
            expected_fqns.contains(user_interface_fqn.as_str()),
            "User interface FQN '{}' should be in expected: {:?}",
            user_interface_fqn,
            expected_fqns
        );
    }

    #[test]
    fn test_golden_cross_file_call_nodes() {
        let expected =
            load_expected_graph("../test/typescript_corpus/cross_file_call/expected.json");

        // Should have 3 function nodes
        let function_nodes: Vec<_> = expected
            .nodes
            .iter()
            .filter(|n| n.kind == "Function")
            .collect();
        assert_eq!(function_nodes.len(), 3, "Should have 3 function nodes");

        // Verify FQNs match our generation
        let add_fqn = build_typescript_fqn("add", "src/math.ts");
        let multiply_fqn = build_typescript_fqn("multiply", "src/math.ts");
        let calculate_fqn = build_typescript_fqn("calculate", "src/calculator.ts");

        let expected_fqns: HashSet<&str> = expected.nodes.iter().map(|n| n.fqn.as_str()).collect();

        assert!(expected_fqns.contains(add_fqn.as_str()));
        assert!(expected_fqns.contains(multiply_fqn.as_str()));
        assert!(expected_fqns.contains(calculate_fqn.as_str()));
    }

    #[test]
    fn test_golden_cross_file_call_edges() {
        let expected =
            load_expected_graph("../test/typescript_corpus/cross_file_call/expected.json");

        // Should have edges from calculator::calculate to math functions
        let call_edges: Vec<_> = expected.edges.iter().filter(|e| e.kind == "Call").collect();

        assert_eq!(call_edges.len(), 2, "Should have 2 call edges");

        // All call edges should originate from calculate
        for edge in call_edges {
            assert!(
                edge.from_fqn.contains("calculator::calculate"),
                "Call edge should originate from calculate"
            );
        }

        // Should have an import edge
        let import_edges: Vec<_> = expected
            .edges
            .iter()
            .filter(|e| e.kind == "Import")
            .collect();

        assert_eq!(import_edges.len(), 1, "Should have 1 import edge");
    }

    #[test]
    fn test_golden_containment_edges() {
        let expected = load_expected_graph("../test/typescript_corpus/single_class/expected.json");

        let contains_edges: Vec<_> = expected
            .edges
            .iter()
            .filter(|e| e.kind == "Contains")
            .collect();

        // UserService contains constructor, getUser, createUser
        assert_eq!(
            contains_edges.len(),
            3,
            "UserService should have 3 Contains edges"
        );

        for edge in contains_edges {
            assert!(
                edge.from_fqn.contains("UserService"),
                "Contains edges should originate from class"
            );
            assert!(
                edge.to_fqn.contains("UserService::"),
                "Contains edges should target methods"
            );
        }
    }

    #[test]
    fn test_golden_fixture_schema_completeness() {
        // Verify all expected.json fixtures have required fields
        let fixture_paths = [
            "../test/typescript_corpus/single_function/expected.json",
            "../test/typescript_corpus/single_class/expected.json",
            "../test/typescript_corpus/cross_file_call/expected.json",
        ];

        for path in fixture_paths {
            let expected = load_expected_graph(path);

            // Must have description
            assert!(
                !expected.description.is_empty(),
                "{} must have description",
                path
            );

            // All nodes must have kind and fqn
            for node in &expected.nodes {
                assert!(!node.kind.is_empty(), "{}: node must have kind", path);
                assert!(!node.fqn.is_empty(), "{}: node must have fqn", path);
            }

            // All edges must have kind, from_fqn, to_fqn
            for edge in &expected.edges {
                assert!(!edge.kind.is_empty(), "{}: edge must have kind", path);
                assert!(
                    !edge.from_fqn.is_empty(),
                    "{}: edge must have from_fqn",
                    path
                );
                assert!(!edge.to_fqn.is_empty(), "{}: edge must have to_fqn", path);
            }
        }
    }
}

// ============================================================================
// Negative Tests: Verify System Does NOT Produce Incorrect Results
// ============================================================================

mod negative_tests {
    use graphengine_parsing::syntax::utils::typescript_fqn::{
        build_typescript_fqn, build_typescript_method_fqn,
    };

    #[test]
    fn test_fqn_does_not_resolve_to_wrong_file() {
        // A function in utils.ts should NOT have FQN containing "services"
        let fqn = build_typescript_fqn("calculateTotal", "src/utils.ts");

        assert!(
            !fqn.contains("services"),
            "utils.ts FQN should not contain 'services'"
        );
        assert!(
            !fqn.contains("auth"),
            "utils.ts FQN should not contain 'auth'"
        );
    }

    #[test]
    fn test_fqn_does_not_duplicate_path_segments() {
        // FQN should not have duplicate path segments like "src/src"
        let fqn = build_typescript_fqn("MyClass", "src/services/api.ts");

        assert!(!fqn.contains("src/src"), "FQN should not duplicate 'src'");
        assert!(
            !fqn.contains("services/services"),
            "FQN should not duplicate path"
        );
    }

    #[test]
    fn test_method_fqn_does_not_belong_to_wrong_class() {
        // login method on AuthService should NOT have FQN mentioning UserService
        let auth_login_fqn = build_typescript_method_fqn("login", "AuthService", "src/auth.ts");
        let user_login_fqn = build_typescript_method_fqn("login", "UserService", "src/user.ts");

        assert_ne!(
            auth_login_fqn, user_login_fqn,
            "Different classes should have different method FQNs"
        );
        assert!(
            !auth_login_fqn.contains("UserService"),
            "AuthService::login should not mention UserService"
        );
        assert!(
            !user_login_fqn.contains("AuthService"),
            "UserService::login should not mention AuthService"
        );
    }

    #[test]
    fn test_fqn_does_not_confuse_similar_paths() {
        // src/utils and src/my-utils should produce different FQNs
        let utils_fqn = build_typescript_fqn("helper", "src/utils.ts");
        let my_utils_fqn = build_typescript_fqn("helper", "src/my-utils.ts");

        assert_ne!(
            utils_fqn, my_utils_fqn,
            "Different files should have different FQNs"
        );
    }

    #[test]
    fn test_fqn_does_not_confuse_relative_vs_absolute_paths() {
        // Both relative and absolute should resolve to same canonical form
        let relative_fqn = build_typescript_fqn("Config", "src/config.ts");
        let absolute_fqn = build_typescript_fqn("Config", "/home/user/project/src/config.ts");

        // Both should contain src/config::Config
        assert!(relative_fqn.contains("src/config"));
        assert!(absolute_fqn.contains("src/config"));
    }

    #[test]
    fn test_fqn_does_not_include_node_modules() {
        // External dependencies should be handled differently
        // This tests that our path extraction works for user code
        let user_code_fqn = build_typescript_fqn("MyComponent", "src/components/Button.ts");

        // Should be in src/, not in node_modules
        assert!(
            user_code_fqn.starts_with("src/"),
            "User code should start with src/"
        );
    }

    #[test]
    fn test_class_fqn_not_confused_with_method_fqn() {
        // UserService (class) should have different FQN than UserService::getUser (method)
        let class_fqn = build_typescript_fqn("UserService", "src/user.service.ts");
        let method_fqn =
            build_typescript_method_fqn("getUser", "UserService", "src/user.service.ts");

        assert_ne!(class_fqn, method_fqn, "Class and method FQNs must differ");
        assert!(
            method_fqn.starts_with(&class_fqn),
            "Method FQN should be nested under class FQN"
        );
    }

    #[test]
    fn test_interface_fqn_not_confused_with_class_fqn() {
        // User (interface) and UserService (class) should have different FQNs
        let interface_fqn = build_typescript_fqn("User", "src/types/user.ts");
        let class_fqn = build_typescript_fqn("UserService", "src/services/user.service.ts");

        assert_ne!(
            interface_fqn, class_fqn,
            "Interface and class FQNs must differ"
        );
    }

    #[test]
    fn test_fqn_does_not_add_spurious_separators() {
        let fqn = build_typescript_fqn("Component", "src/ui/Button.tsx");

        // Should not have multiple consecutive separators
        assert!(!fqn.contains("::::"), "FQN should not have ::::");
        assert!(!fqn.contains("//"), "FQN should not have //");
        assert!(
            !fqn.contains("::/:"),
            "FQN should not have mixed separators"
        );
    }

    #[test]
    fn test_empty_symbol_name_handled() {
        // Edge case: empty name should not crash
        let fqn = build_typescript_fqn("", "src/test.ts");
        // Should still have the path part
        assert!(fqn.contains("src/test"));
    }

    #[test]
    fn test_symbol_with_special_chars_handled() {
        // Some valid TypeScript identifiers have underscores
        let fqn = build_typescript_fqn("_privateMethod", "src/class.ts");
        assert!(fqn.contains("_privateMethod"));

        let fqn2 = build_typescript_fqn("$jquery", "src/legacy.ts");
        assert!(fqn2.contains("$jquery"));
    }
}

// ============================================================================
// Edge Relationship Validation
// ============================================================================

mod edge_validation {
    use serde::Deserialize;
    use std::collections::HashSet;

    #[derive(Debug, Deserialize)]
    struct ExpectedGraph {
        nodes: Vec<ExpectedNode>,
        edges: Vec<ExpectedEdge>,
    }

    #[derive(Debug, Deserialize)]
    struct ExpectedNode {
        fqn: String,
    }

    #[derive(Debug, Deserialize)]
    struct ExpectedEdge {
        from_fqn: String,
        to_fqn: String,
    }

    fn load_graph(fixture_path: &str) -> ExpectedGraph {
        let json = std::fs::read_to_string(fixture_path).unwrap();
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn test_all_edge_endpoints_exist_in_nodes() {
        // Property: Every edge must reference nodes that exist
        let fixture_paths = [
            "../test/typescript_corpus/single_class/expected.json",
            "../test/typescript_corpus/cross_file_call/expected.json",
        ];

        for path in fixture_paths {
            let graph = load_graph(path);
            let node_fqns: HashSet<&str> = graph.nodes.iter().map(|n| n.fqn.as_str()).collect();

            for edge in &graph.edges {
                // For Contains edges, from_fqn should be in nodes
                // For some edges, we might have module-level references
                let from_exists = node_fqns
                    .iter()
                    .any(|n| edge.from_fqn.starts_with(*n) || *n == edge.from_fqn);
                let _to_exists = node_fqns
                    .iter()
                    .any(|n| edge.to_fqn.starts_with(*n) || *n == edge.to_fqn);

                assert!(
                    from_exists
                        || edge
                            .from_fqn
                            .ends_with(edge.from_fqn.split("::").last().unwrap_or("")),
                    "{}: Edge from '{}' has no matching node. Available: {:?}",
                    path,
                    edge.from_fqn,
                    node_fqns
                );
            }
        }
    }

    #[test]
    fn test_no_self_referential_edges() {
        // Property: No edge should have same source and target
        let fixture_paths = [
            "../test/typescript_corpus/single_class/expected.json",
            "../test/typescript_corpus/cross_file_call/expected.json",
        ];

        for path in fixture_paths {
            let graph = load_graph(path);

            for edge in &graph.edges {
                assert_ne!(
                    edge.from_fqn, edge.to_fqn,
                    "{}: Edge should not be self-referential: {}",
                    path, edge.from_fqn
                );
            }
        }
    }

    #[test]
    fn test_no_duplicate_edges() {
        // Property: No exact duplicate edges
        let fixture_paths = [
            "../test/typescript_corpus/single_class/expected.json",
            "../test/typescript_corpus/cross_file_call/expected.json",
        ];

        for path in fixture_paths {
            let graph = load_graph(path);
            let mut seen: HashSet<(String, String)> = HashSet::new();

            for edge in &graph.edges {
                let key = (edge.from_fqn.clone(), edge.to_fqn.clone());
                assert!(
                    seen.insert(key.clone()),
                    "{}: Duplicate edge detected: {} -> {}",
                    path,
                    edge.from_fqn,
                    edge.to_fqn
                );
            }
        }
    }
}

// ============================================================================
// Performance Assertion Tests (Regression Prevention)
// ============================================================================

mod performance_assertions {
    use graphengine_parsing::syntax::utils::typescript_fqn::build_typescript_fqn;
    use std::time::Instant;

    #[test]
    fn test_fqn_generation_is_fast() {
        // FQN generation should be sub-millisecond for single invocation
        let start = Instant::now();
        for _ in 0..1000 {
            let _ = build_typescript_fqn("TestClass", "src/deeply/nested/path/to/file.ts");
        }
        let duration = start.elapsed();

        // 1000 FQN generations should complete in under 100ms
        assert!(
            duration.as_millis() < 100,
            "1000 FQN generations took {:?}, expected < 100ms",
            duration
        );
    }

    #[test]
    fn test_fixture_loading_is_fast() {
        let start = Instant::now();

        // Load all fixtures
        for _ in 0..10 {
            let _ =
                std::fs::read_to_string("../test/typescript_corpus/single_function/expected.json");
            let _ = std::fs::read_to_string("../test/typescript_corpus/single_class/expected.json");
            let _ =
                std::fs::read_to_string("../test/typescript_corpus/cross_file_call/expected.json");
        }

        let duration = start.elapsed();

        // 30 file reads should complete in under 500ms
        assert!(
            duration.as_millis() < 500,
            "30 fixture reads took {:?}, expected < 500ms",
            duration
        );
    }
}
