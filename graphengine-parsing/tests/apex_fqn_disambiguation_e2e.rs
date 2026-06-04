//! End-to-end validation of Sprint E.2: FQN disambiguation.
//!
//! Pins the two regressions that surfaced in dreamhouse-lwc / apex-recipes:
//!
//! 1. Overloaded constructors in the same class must produce distinct FQNs.
//! 2. Sibling inner classes that each declare a method with the same simple
//!    name (think `GeocodingServiceTest.MockSuccess.respond` vs
//!    `GeocodingServiceTest.MockFailure.respond`) must produce distinct FQNs.
//!
//! A third assertion guards against accidental non-Apex regressions by
//! confirming the shared path-based FQN shape stays unchanged when the
//! override hook returns `None`.

use graphengine_parsing::application::ports::SyntaxExtractor;
use graphengine_parsing::domain::NodeKind;
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::collections::HashMap;
use std::path::PathBuf;

struct TempFixture {
    root: PathBuf,
}

impl TempFixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("gridseak_apex_fqn_e2e_{name}"));
        if root.exists() {
            std::fs::remove_dir_all(&root).expect("cleanup previous fixture");
        }
        std::fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir -p");
        }
        std::fs::write(&path, contents).expect("write fixture");
        path
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn overloaded_constructors_and_sibling_inner_methods_are_unique() {
    let fx = TempFixture::new("overloaded_and_siblings");

    // Overloaded-constructor case (the `MetadataTriggerHandler` shape).
    let handler_path = fx.write(
        "force-app/main/default/classes/Handler.cls",
        r#"
public class Handler {
    public Handler() {}
    public Handler(Integer x) {}
    public Handler(Integer x, String y) {}
}
"#,
    );

    // Sibling-inner-class case (the `GeocodingServiceTest` shape).
    let sibling_path = fx.write(
        "force-app/main/default/classes/GeocodingServiceTest.cls",
        r#"
@IsTest
public class GeocodingServiceTest {
    public class MockSuccess {
        public HttpResponse respond(HttpRequest req) { return null; }
    }
    public class MockFailure {
        public HttpResponse respond(HttpRequest req) { return null; }
    }
}
"#,
    );

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    let results = extractor
        .extract(&[handler_path.clone(), sibling_path.clone()])
        .await
        .expect("extract fixture");

    // ---- Overloaded constructors must diverge on parameter signature ----
    let handler_ctors: Vec<&String> = results
        .symbols
        .iter()
        .filter(|n| {
            matches!(n.kind, NodeKind::Function) && n.location.file.ends_with("Handler.cls")
        })
        .map(|n| &n.fqn)
        .collect();
    assert_eq!(
        handler_ctors.len(),
        3,
        "all three Handler constructors must be extracted; got {handler_ctors:?}",
    );
    let mut dedup: Vec<&String> = handler_ctors.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(
        dedup.len(),
        handler_ctors.len(),
        "overloaded constructors must get distinct FQNs: {handler_ctors:?}",
    );
    assert!(
        handler_ctors
            .iter()
            .any(|f| f.ends_with("::Handler::Handler()")),
        "zero-arg constructor must encode empty signature: {handler_ctors:?}",
    );
    assert!(
        handler_ctors
            .iter()
            .any(|f| f.ends_with("::Handler::Handler(Integer)")),
        "1-arg constructor must include parameter type: {handler_ctors:?}",
    );
    assert!(
        handler_ctors
            .iter()
            .any(|f| f.ends_with("::Handler::Handler(Integer,String)")),
        "2-arg constructor must include both parameter types: {handler_ctors:?}",
    );

    // ---- Sibling-inner-class `respond` methods must use distinct outer paths ----
    let respond_methods: Vec<&String> = results
        .symbols
        .iter()
        .filter(|n| {
            matches!(n.kind, NodeKind::Function)
                && n.location.file.ends_with("GeocodingServiceTest.cls")
                && n.fqn.contains("::respond(")
        })
        .map(|n| &n.fqn)
        .collect();
    assert_eq!(
        respond_methods.len(),
        2,
        "both inner-class `respond` methods must survive extraction; got {respond_methods:?}",
    );
    let mut respond_dedup: Vec<&String> = respond_methods.clone();
    respond_dedup.sort();
    respond_dedup.dedup();
    assert_eq!(
        respond_dedup.len(),
        2,
        "sibling inner-class methods must diverge on outer-class path: {respond_methods:?}",
    );
    assert!(
        respond_methods
            .iter()
            .any(|f| f.contains("::GeocodingServiceTest.MockSuccess::respond(")),
        "MockSuccess.respond must carry the outer.inner path: {respond_methods:?}",
    );
    assert!(
        respond_methods
            .iter()
            .any(|f| f.contains("::GeocodingServiceTest.MockFailure::respond(")),
        "MockFailure.respond must carry the outer.inner path: {respond_methods:?}",
    );

    // ---- Inner classes themselves get the dotted shape ----
    let inner_classes: Vec<&String> = results
        .symbols
        .iter()
        .filter(|n| {
            matches!(n.kind, NodeKind::Struct)
                && n.location.file.ends_with("GeocodingServiceTest.cls")
        })
        .map(|n| &n.fqn)
        .collect();
    assert!(
        inner_classes
            .iter()
            .any(|f| f.ends_with("::GeocodingServiceTest.MockSuccess")),
        "MockSuccess class FQN must use dotted outer path: {inner_classes:?}",
    );
    assert!(
        inner_classes
            .iter()
            .any(|f| f.ends_with("::GeocodingServiceTest.MockFailure")),
        "MockFailure class FQN must use dotted outer path: {inner_classes:?}",
    );

    // ---- Global uniqueness across the scan ----
    let mut counts: HashMap<&String, usize> = HashMap::new();
    for n in &results.symbols {
        *counts.entry(&n.fqn).or_insert(0) += 1;
    }
    let dupes: Vec<(&&String, &usize)> = counts.iter().filter(|(_, c)| **c > 1).collect();
    assert!(
        dupes.is_empty(),
        "FQN dedup key must be unique across all extracted symbols: {dupes:?}",
    );
}

#[tokio::test]
async fn non_apex_languages_retain_default_fqn_shape() {
    // Sanity guard: the new override hook must not alter FQNs for
    // languages whose extractor returns `None`. We use TypeScript as a
    // representative non-Apex target — it exercises the same shared
    // `build_simple_fqn` path that Rust/Python/Go all rely on.
    let fx = TempFixture::new("ts_baseline");
    let ts_path = fx.write(
        "src/demo.ts",
        r#"
export function hello(name: string): string {
    return "hi " + name;
}
"#,
    );

    let config = load_config("typescript").expect("load ts config");
    let extractor = TreeSitterExtractor::new(config).expect("build ts extractor");
    let results = extractor
        .extract(std::slice::from_ref(&ts_path))
        .await
        .expect("extract fixture");

    let hello = results
        .symbols
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Function) && n.fqn.ends_with("hello"))
        .expect("hello function must be extracted");
    // TypeScript FQN must NOT include a `(sig)` tail — that would
    // indicate the Apex override accidentally fired for a non-Apex
    // language.
    assert!(
        !hello.fqn.contains('('),
        "non-Apex FQN must not carry a method signature: {}",
        hello.fqn,
    );
}
