//! TypeScript FQN (Fully Qualified Name) Domain Tests
//!
//! These tests define the expected FQN generation rules for TypeScript symbols.
//! Following TDD: tests are written BEFORE implementation.
//!
//! FQN Format for TypeScript:
//! - `{relative_path_without_extension}::{symbol_name}`
//! - For nested symbols: `{path}::{class_name}::{method_name}`
//!
//! Examples:
//! - Class `AuthService` in `src/auth/auth.service.ts` → `src/auth/auth.service::AuthService`
//! - Method `login` in class `AuthService` → `src/auth/auth.service::AuthService::login`

use graphengine_parsing::syntax::utils::typescript_fqn::{
    build_typescript_fqn, build_typescript_method_fqn,
};

/// Test FQN generation for a TypeScript class
#[test]
fn test_typescript_class_fqn_in_nested_path() {
    // Input: Class `AuthService` in file `src/auth/auth.service.ts`
    // Expected FQN: `src/auth/auth.service::AuthService`
    let fqn = build_typescript_fqn("AuthService", "src/auth/auth.service.ts");
    assert_eq!(fqn, "src/auth/auth.service::AuthService");
}

/// Test FQN generation for a TypeScript class in root src directory
#[test]
fn test_typescript_class_fqn_in_src_root() {
    let fqn = build_typescript_fqn("AppController", "src/app.controller.ts");
    assert_eq!(fqn, "src/app.controller::AppController");
}

/// Test FQN generation for a TypeScript method
#[test]
fn test_typescript_method_fqn() {
    // Input: Method `login` in class `AuthService` in file `src/auth/auth.service.ts`
    // Expected FQN: `src/auth/auth.service::AuthService::login`
    let fqn = build_typescript_method_fqn("login", "AuthService", "src/auth/auth.service.ts");
    assert_eq!(fqn, "src/auth/auth.service::AuthService::login");
}

/// Test FQN generation for a TypeScript interface
#[test]
fn test_typescript_interface_fqn() {
    // Input: Interface `User` in file `src/types/user.ts`
    // Expected FQN: `src/types/user::User`
    let fqn = build_typescript_fqn("User", "src/types/user.ts");
    assert_eq!(fqn, "src/types/user::User");
}

/// Test FQN generation for a standalone function
#[test]
fn test_typescript_function_fqn() {
    let fqn = build_typescript_fqn("calculateTotal", "src/utils/math.ts");
    assert_eq!(fqn, "src/utils/math::calculateTotal");
}

/// Test FQN generation handles .tsx files correctly
#[test]
fn test_typescript_tsx_file_fqn() {
    let fqn = build_typescript_fqn("UserProfile", "src/components/UserProfile.tsx");
    assert_eq!(fqn, "src/components/UserProfile__tsx::UserProfile");
}

/// Test FQN generation for index files
#[test]
fn test_typescript_index_file_fqn() {
    let fqn = build_typescript_fqn("main", "src/index.ts");
    assert_eq!(fqn, "src/index::main");
}

/// Test FQN generation handles deeply nested paths
#[test]
fn test_typescript_deeply_nested_fqn() {
    let fqn = build_typescript_fqn(
        "DatabaseConnection",
        "src/infrastructure/persistence/database/connection.ts",
    );
    assert_eq!(
        fqn,
        "src/infrastructure/persistence/database/connection::DatabaseConnection"
    );
}

/// Test FQN generation strips project root prefix if present
#[test]
fn test_typescript_absolute_path_normalization() {
    // When given an absolute path, it should still produce relative FQN
    let fqn = build_typescript_fqn("Service", "/home/user/project/src/services/api.ts");
    // Should handle gracefully - extract from src/ onward
    assert!(fqn.contains("Service"));
    assert!(fqn.contains("src/services/api"));
}

/// Test FQN generation for constructor methods
#[test]
fn test_typescript_constructor_fqn() {
    let fqn = build_typescript_method_fqn("constructor", "UserService", "src/user.service.ts");
    assert_eq!(fqn, "src/user.service::UserService::constructor");
}

/// Test FQN generation for static methods
#[test]
fn test_typescript_static_method_fqn() {
    let fqn = build_typescript_method_fqn("create", "Factory", "src/factory.ts");
    assert_eq!(fqn, "src/factory::Factory::create");
}

/// Test FQN is deterministic (same input produces same output)
#[test]
fn test_typescript_fqn_deterministic() {
    let fqn1 = build_typescript_fqn("Test", "src/test.ts");
    let fqn2 = build_typescript_fqn("Test", "src/test.ts");
    assert_eq!(fqn1, fqn2, "FQN generation must be deterministic");
}

/// Test FQN handles paths without src/ prefix
#[test]
fn test_typescript_no_src_prefix() {
    let fqn = build_typescript_fqn("Config", "lib/config.ts");
    assert_eq!(fqn, "lib/config::Config");
}

/// Test FQN handles Windows-style paths
#[test]
fn test_typescript_windows_path_normalization() {
    let fqn = build_typescript_fqn("Service", "src\\services\\api.ts");
    // Should normalize to forward slashes
    assert_eq!(fqn, "src/services/api::Service");
}
