//! Sprint E.5 — Inner-class property propagation end-to-end.
//!
//! Two separate propagation rules had silent gaps before this sprint:
//! 1. `apex_sharing` was never set on inner-class Struct nodes because
//!    the classifier returned `None` for anything nested inside a
//!    `class_body`. Downstream security reports treated every inner
//!    class as "modifier absent", erasing the outer class's posture.
//! 2. `is_test` was correctly set on methods inside `@IsTest` classes
//!    (via the method-level Tier-2 walk), but NOT on the inner-class
//!    struct node itself. A query like
//!    `WHERE kind='Struct' AND properties->>'is_test'='true'`
//!    missed mock/helper classes even when every method inside them
//!    was tagged.
//!
//! Fixing both required touching the language-specific detectors
//! (`sharing.rs`, `apex_test_detector.rs`). This file pins the
//! end-to-end contract: parse real Apex source, resolve through the
//! shared symbol_extractor path, and assert the struct-level
//! properties are right.

use graphengine_parsing::application::ports::SyntaxExtractor;
use graphengine_parsing::domain::{Node, NodeKind};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::PathBuf;
use tempfile::TempDir;

fn write_fixture(dir: &std::path::Path, filename: &str, src: &str) -> PathBuf {
    let p = dir.join(filename);
    std::fs::write(&p, src).expect("write");
    p
}

async fn parse(files: Vec<PathBuf>) -> graphengine_parsing::application::ports::SyntaxResults {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("extractor");
    extractor.extract(&files).await.expect("parse")
}

fn find_struct_by_name<'a>(symbols: &'a [Node], simple_name: &str) -> Option<&'a Node> {
    symbols.iter().find(|n| {
        n.kind == NodeKind::Struct
            && n.fqn
                .rsplit("::")
                .next()
                .map(|last| last.ends_with(simple_name) || last == simple_name)
                .unwrap_or(false)
    })
}

#[tokio::test]
async fn inner_class_inherits_outer_with_sharing_property() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(
        tmp.path(),
        "Outer.cls",
        "\
public with sharing class Outer {
    public class Inner {
        public void run() {}
    }
    public void go() {}
}
",
    );
    let hints = parse(vec![path]).await;

    let outer = find_struct_by_name(&hints.symbols, "Outer").expect("Outer struct");
    let inner_fqn_tail = "Outer.Inner";
    let inner = hints
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Struct && n.fqn.ends_with(inner_fqn_tail))
        .unwrap_or_else(|| {
            panic!(
                "Inner Struct missing; symbols: {:?}",
                hints
                    .symbols
                    .iter()
                    .filter(|n| n.kind == NodeKind::Struct)
                    .map(|n| n.fqn.as_str())
                    .collect::<Vec<_>>()
            )
        });

    assert_eq!(
        outer
            .properties
            .get("apex_sharing")
            .and_then(|v| v.as_str()),
        Some("with_sharing"),
        "outer class must declare with_sharing"
    );
    assert_eq!(
        inner
            .properties
            .get("apex_sharing")
            .and_then(|v| v.as_str()),
        Some("with_sharing"),
        "inner class must inherit outer's with_sharing; props: {:?}",
        inner.properties
    );
}

#[tokio::test]
async fn inner_class_overrides_outer_sharing_when_declared_locally() {
    // Defensive: if an inner class declares its own modifier (newer
    // grammars tolerate it), the inner's own modifier must win.
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(
        tmp.path(),
        "Svc.cls",
        "\
public with sharing class Svc {
    public without sharing class Escalated {
        public void run() {}
    }
}
",
    );
    let hints = parse(vec![path]).await;
    let inner = hints
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Struct && n.fqn.ends_with("Svc.Escalated"))
        .expect("inner struct");
    assert_eq!(
        inner
            .properties
            .get("apex_sharing")
            .and_then(|v| v.as_str()),
        Some("without_sharing"),
        "inner's own modifier must win over outer's"
    );
}

#[tokio::test]
async fn inner_class_inside_istest_outer_is_tagged_is_test() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(
        tmp.path(),
        "GeocodingServiceTest.cls",
        "\
@IsTest
public class GeocodingServiceTest {
    public class MockSuccess {
        public void respond() {}
    }

    public class MockFailure {
        public void respond() {}
    }
}
",
    );
    let hints = parse(vec![path]).await;

    for inner_tail in [
        "GeocodingServiceTest.MockSuccess",
        "GeocodingServiceTest.MockFailure",
    ] {
        let inner = hints
            .symbols
            .iter()
            .find(|n| n.kind == NodeKind::Struct && n.fqn.ends_with(inner_tail))
            .unwrap_or_else(|| panic!("struct {inner_tail} missing"));
        let is_test = inner
            .properties
            .get("is_test")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(
            is_test,
            "{inner_tail} should be is_test=true by inheritance; props={:?}",
            inner.properties
        );
    }
}

#[tokio::test]
async fn inner_class_in_non_test_outer_does_not_pick_up_is_test() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(
        tmp.path(),
        "Plain.cls",
        "\
public class Plain {
    public class Nested {
        public void doIt() {}
    }
}
",
    );
    let hints = parse(vec![path]).await;
    let nested = hints
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Struct && n.fqn.ends_with("Plain.Nested"))
        .expect("Nested struct");
    assert!(
        nested
            .properties
            .get("is_test")
            .and_then(|v| v.as_bool())
            .is_none(),
        "Nested in a non-test class must not carry is_test; props={:?}",
        nested.properties
    );
}
