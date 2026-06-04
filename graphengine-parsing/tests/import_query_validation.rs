//! Import query validation tests for JavaScript, Python, and Go.
//!
//! These tests verify that the tree-sitter import queries in each language config
//! correctly parse and capture the expected sub-captures from real-world import
//! syntax. Without these captures, the import extractor cannot create Import edges.
//!
//! Each test follows the pattern:
//! 1. Load the language config (YAML)
//! 2. Parse a code snippet with tree-sitter
//! 3. Run the import query
//! 4. Assert that specific captures exist (source, imported_name, etc.)

use graphengine_parsing::infrastructure::config::load_config;
use std::collections::HashMap;

// =============================================================================
// Helpers
// =============================================================================

fn run_import_query(language_name: &str, code: &str) -> Vec<HashMap<String, Vec<String>>> {
    let config = load_config(language_name)
        .unwrap_or_else(|e| panic!("Failed to load {language_name} config: {e:?}"));

    let lang = match language_name {
        "javascript" => tree_sitter_javascript::language(),
        "python" => tree_sitter_python::language(),
        "go" => tree_sitter_go::language(),
        "typescript" => tree_sitter_typescript::language_tsx(),
        _ => panic!("unsupported language: {language_name}"),
    };

    let query_str = config
        .queries
        .get("imports")
        .unwrap_or_else(|| panic!("No imports query in {language_name} config"));

    let query = tree_sitter::Query::new(lang, query_str)
        .unwrap_or_else(|e| panic!("{language_name} imports query has syntax error: {e:?}"));

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(lang).unwrap();
    let tree = parser.parse(code, None).unwrap();

    let mut cursor = tree_sitter::QueryCursor::new();
    let matches = cursor.matches(&query, tree.root_node(), code.as_bytes());

    let capture_names = query.capture_names();
    let mut results = Vec::new();

    for mat in matches {
        let mut captures: HashMap<String, Vec<String>> = HashMap::new();
        for cap in mat.captures {
            let name = capture_names
                .get(cap.index as usize)
                .map(|s| s.as_str())
                .unwrap_or("unknown");
            let text = cap
                .node
                .utf8_text(code.as_bytes())
                .unwrap_or("")
                .to_string();
            captures.entry(name.to_string()).or_default().push(text);
        }
        results.push(captures);
    }

    results
}

fn assert_has_capture(
    matches: &[HashMap<String, Vec<String>>],
    capture_name: &str,
    expected_value: &str,
) {
    let found = matches.iter().any(|m| {
        m.get(capture_name)
            .map(|vals| vals.iter().any(|v| v.contains(expected_value)))
            .unwrap_or(false)
    });
    assert!(
        found,
        "Expected capture @{capture_name} containing '{expected_value}' not found in matches: {matches:?}"
    );
}

fn assert_no_match(matches: &[HashMap<String, Vec<String>>]) {
    assert!(
        matches.is_empty(),
        "Expected no matches but found: {matches:?}"
    );
}

// =============================================================================
// JavaScript — Import Query Validation
// =============================================================================

#[test]
fn js_import_query_parses_without_syntax_error() {
    let config = load_config("javascript").unwrap();
    let lang = tree_sitter_javascript::language();
    let query_str = config.queries.get("imports").unwrap();
    let result = tree_sitter::Query::new(lang, query_str);
    assert!(
        result.is_ok(),
        "JS imports query has syntax error: {:?}",
        result.err()
    );
}

#[test]
fn js_named_import_captures_source_and_name() {
    let code = r#"import { Router } from "express";"#;
    let matches = run_import_query("javascript", code);
    assert!(!matches.is_empty(), "Should have at least one match");
    assert_has_capture(&matches, "source", "express");
    assert_has_capture(&matches, "imported_name", "Router");
}

#[test]
fn js_named_import_with_alias() {
    let code = r#"import { useState as useReactState } from "react";"#;
    let matches = run_import_query("javascript", code);
    assert_has_capture(&matches, "source", "react");
    assert_has_capture(&matches, "imported_name", "useState");
    assert_has_capture(&matches, "local_name", "useReactState");
}

#[test]
fn js_default_import_captures_source_and_default() {
    let code = r#"import React from "react";"#;
    let matches = run_import_query("javascript", code);
    assert_has_capture(&matches, "source", "react");
    assert_has_capture(&matches, "default_import", "React");
}

#[test]
fn js_namespace_import() {
    let code = r#"import * as utils from "./utils";"#;
    let matches = run_import_query("javascript", code);
    assert_has_capture(&matches, "source", "./utils");
    assert_has_capture(&matches, "namespace", "utils");
}

#[test]
fn js_side_effect_import_captures_source() {
    let code = r#"import "./polyfill";"#;
    let matches = run_import_query("javascript", code);
    assert_has_capture(&matches, "source", "./polyfill");
}

#[test]
fn js_relative_import_captures_path() {
    let code = r#"import { handler } from "../middleware/auth";"#;
    let matches = run_import_query("javascript", code);
    assert_has_capture(&matches, "source", "../middleware/auth");
    assert_has_capture(&matches, "imported_name", "handler");
}

#[test]
fn js_multiple_named_imports() {
    let code = r#"import { get, post, put } from "./http";"#;
    let matches = run_import_query("javascript", code);
    assert_has_capture(&matches, "source", "./http");
    assert_has_capture(&matches, "imported_name", "get");
    assert_has_capture(&matches, "imported_name", "post");
    assert_has_capture(&matches, "imported_name", "put");
}

// =============================================================================
// Python — Import Query Validation
// =============================================================================

#[test]
fn py_import_query_parses_without_syntax_error() {
    let config = load_config("python").unwrap();
    let lang = tree_sitter_python::language();
    let query_str = config.queries.get("imports").unwrap();
    let result = tree_sitter::Query::new(lang, query_str);
    assert!(
        result.is_ok(),
        "Python imports query has syntax error: {:?}",
        result.err()
    );
}

#[test]
fn py_simple_import_captures_module() {
    let code = "import os";
    let matches = run_import_query("python", code);
    assert!(!matches.is_empty(), "Should have at least one match");
    assert_has_capture(&matches, "module", "os");
}

#[test]
fn py_dotted_import_captures_full_module() {
    let code = "import os.path";
    let matches = run_import_query("python", code);
    assert_has_capture(&matches, "module", "os.path");
}

#[test]
fn py_from_import_captures_module_and_name() {
    let code = "from os import path";
    let matches = run_import_query("python", code);
    assert_has_capture(&matches, "module", "os");
    assert_has_capture(&matches, "imported_name", "path");
}

#[test]
fn py_from_import_with_alias() {
    let code = "from os import path as p";
    let matches = run_import_query("python", code);
    assert_has_capture(&matches, "module", "os");
    assert_has_capture(&matches, "imported_name", "path");
    assert_has_capture(&matches, "local_name", "p");
}

#[test]
fn py_import_with_alias() {
    let code = "import numpy as np";
    let matches = run_import_query("python", code);
    assert_has_capture(&matches, "module", "numpy");
    assert_has_capture(&matches, "local_name", "np");
}

#[test]
fn py_from_import_wildcard() {
    let code = "from os.path import *";
    let matches = run_import_query("python", code);
    assert_has_capture(&matches, "module", "os.path");
    assert_has_capture(&matches, "glob", "*");
}

#[test]
fn py_relative_import() {
    let code = "from . import utils";
    let matches = run_import_query("python", code);
    assert!(!matches.is_empty(), "Should match relative import");
    assert_has_capture(&matches, "imported_name", "utils");
}

#[test]
fn py_deep_relative_import() {
    let code = "from ..utils import helper";
    let matches = run_import_query("python", code);
    assert!(!matches.is_empty(), "Should match deep relative import");
    assert_has_capture(&matches, "imported_name", "helper");
}

#[test]
fn py_non_import_code_no_match() {
    let code = "x = os.path.join('a', 'b')";
    let matches = run_import_query("python", code);
    assert_no_match(&matches);
}

// =============================================================================
// Go — Import Query Validation
// =============================================================================

#[test]
fn go_import_query_parses_without_syntax_error() {
    let config = load_config("go").unwrap();
    let lang = tree_sitter_go::language();
    let query_str = config.queries.get("imports").unwrap();
    let result = tree_sitter::Query::new(lang, query_str);
    assert!(
        result.is_ok(),
        "Go imports query has syntax error: {:?}",
        result.err()
    );
}

#[test]
fn go_single_import_captures_path() {
    let code = r#"package main
import "fmt"
"#;
    let matches = run_import_query("go", code);
    assert!(!matches.is_empty(), "Should have at least one match");
    assert_has_capture(&matches, "path", "\"fmt\"");
}

#[test]
fn go_grouped_imports_capture_paths() {
    let code = r#"package main
import (
    "fmt"
    "net/http"
    "os"
)
"#;
    let matches = run_import_query("go", code);
    assert_has_capture(&matches, "path", "\"fmt\"");
    assert_has_capture(&matches, "path", "\"net/http\"");
    assert_has_capture(&matches, "path", "\"os\"");
}

#[test]
fn go_aliased_import_captures_alias_and_path() {
    let code = r#"package main
import myfmt "fmt"
"#;
    let matches = run_import_query("go", code);
    assert_has_capture(&matches, "path", "\"fmt\"");
    assert_has_capture(&matches, "local_name", "myfmt");
}

#[test]
fn go_dot_import() {
    let code = r#"package main
import . "fmt"
"#;
    let matches = run_import_query("go", code);
    assert_has_capture(&matches, "path", "\"fmt\"");
    assert_has_capture(&matches, "glob", ".");
}

#[test]
fn go_blank_import() {
    let code = r#"package main
import _ "net/http/pprof"
"#;
    let matches = run_import_query("go", code);
    assert_has_capture(&matches, "path", "\"net/http/pprof\"");
}

#[test]
fn go_third_party_import_path() {
    let code = r#"package main
import "github.com/go-chi/chi/v5"
"#;
    let matches = run_import_query("go", code);
    assert_has_capture(&matches, "path", "\"github.com/go-chi/chi/v5\"");
}

#[test]
fn go_non_import_code_no_path_capture() {
    let code = r#"package main
func main() { fmt.Println("hello") }
"#;
    let matches = run_import_query("go", code);
    let has_path = matches.iter().any(|m| m.contains_key("path"));
    assert!(!has_path, "Should not capture 'path' from non-import code");
}

// =============================================================================
// JavaScript — CommonJS require() Validation
// =============================================================================

#[test]
fn js_require_default_captures_source_and_name() {
    let code = r#"const express = require("express");"#;
    let matches = run_import_query("javascript", code);
    assert!(
        !matches.is_empty(),
        "require() should produce at least one match"
    );
    assert_has_capture(&matches, "source", "express");
    assert_has_capture(&matches, "default_import", "express");
}

#[test]
fn js_require_destructured_captures_imported_names() {
    let code = r#"const { Router, json } = require("express");"#;
    let matches = run_import_query("javascript", code);
    assert!(!matches.is_empty(), "destructured require() should match");
    assert_has_capture(&matches, "source", "express");
    assert_has_capture(&matches, "imported_name", "Router");
}

#[test]
fn js_require_relative_path() {
    let code = r#"const utils = require("./utils");"#;
    let matches = run_import_query("javascript", code);
    assert_has_capture(&matches, "source", "./utils");
    assert_has_capture(&matches, "default_import", "utils");
}

#[test]
fn js_var_require_captures_source() {
    let code = r#"var path = require("path");"#;
    let matches = run_import_query("javascript", code);
    assert!(!matches.is_empty(), "var require() should match");
    assert_has_capture(&matches, "source", "path");
    assert_has_capture(&matches, "default_import", "path");
}

#[test]
fn js_var_destructured_require() {
    let code = r#"var { readFileSync } = require("fs");"#;
    let matches = run_import_query("javascript", code);
    assert!(
        !matches.is_empty(),
        "var destructured require() should match"
    );
    assert_has_capture(&matches, "source", "fs");
    assert_has_capture(&matches, "imported_name", "readFileSync");
}

// =============================================================================
// Cross-language consistency: all languages have @source or @module or @path
// =============================================================================

#[test]
fn all_supported_languages_have_import_source_capture() {
    for lang in &["javascript", "typescript", "python", "go"] {
        let config =
            load_config(lang).unwrap_or_else(|e| panic!("Failed to load {lang} config: {e:?}"));
        let query_str = config.queries.get("imports");
        assert!(query_str.is_some(), "{lang} config missing 'imports' query");
        let qs = query_str.unwrap();
        let has_source_capture =
            qs.contains("@source") || qs.contains("@module") || qs.contains("@path");
        assert!(
            has_source_capture,
            "{lang} import query lacks a source/module/path capture — Import edges cannot be created without knowing WHAT is being imported"
        );
    }
}
