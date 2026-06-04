//! Extractor-level integration coverage for constructor call sites.
//!
//! Locks the Phase A Commit 1 extractor contract for constructor-shaped
//! call sites across all five languages that ship a `@constructor`
//! capture (Apex, Java, C#, JavaScript, TypeScript) plus the Apex-only
//! explicit-constructor-invocation plumbing (`this(...)` / `super(...)`)
//! and the `CallSite.arg_types` population hook.
//!
//! Source strings are written to tempfiles and fed through
//! `TreeSitterExtractor::extract` end-to-end (config load → query
//! execution → per-language extractor). No resolver runs here; the
//! resolver-side innermost-enclosing-type-wins behaviour that consumes
//! these call sites is locked by the unit tests in
//! `syntax::language::apex::heuristic_resolver` (`super_ctor_*`,
//! `this_ctor_*`). The inner-class `super(...)` test below pins the
//! *shape* the resolver depends on — that the call site's range lies
//! inside the inner class declaration, so the resolver's
//! `SymbolIndex::find_enclosing_type` picks the inner, not the outer.
//!
//! Coverage map (nine tests, per
//! `docs/workstreams/proof-foundation-gap/PHASE_A_EXECUTION_PLAN.md` §8.4):
//!
//! | #   | Test                                        | Closes           |
//! |-----|---------------------------------------------|------------------|
//! | 1–5 | `new X(...)` for apex/java/csharp/js/ts     | R33              |
//! | 6   | Apex `this(...)` explicit ctor              | §8.3 self path   |
//! | 7   | Apex `super(...)` explicit ctor             | §8.3 super path  |
//! | 8   | Apex inner-class `super(...)` shape         | §8.3 inner-super |
//! | 9   | Apex `arg_types` literal correctness        | TR-A.1 overload  |

use graphengine_parsing::application::ports::{CallSite, SyntaxExtractor, SyntaxResults};
use graphengine_parsing::domain::apex::class_symbols::ApexTypeRef;
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::PathBuf;
use tempfile::TempDir;

/// Write `source` to a tempfile with the given `extension`, run the
/// language's `TreeSitterExtractor`, and return the resulting
/// `SyntaxResults` together with the owning `TempDir` (kept alive by
/// the caller so the file path the extractor recorded stays valid for
/// range comparisons).
async fn extract_source(language: &str, extension: &str, source: &str) -> (SyntaxResults, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path: PathBuf = dir.path().join(format!("Fixture{}", extension));
    std::fs::write(&file_path, source).expect("write fixture source");

    let config = load_config(language).unwrap_or_else(|e| panic!("load {language} config: {e}"));
    let extractor =
        TreeSitterExtractor::new(config).unwrap_or_else(|e| panic!("build {language}: {e}"));
    let results = extractor
        .extract(&[file_path])
        .await
        .unwrap_or_else(|e| panic!("extract {language}: {e}"));
    (results, dir)
}

/// Return all call sites whose synthesised function name matches the
/// given `constructor_call:<name>` shape. The shared extractor emits
/// this prefix whenever `@constructor` or `@chained_ctor_keyword`
/// fires (see `call_site_extractor::CONSTRUCTOR_CALL_PREFIX`).
fn ctor_sites<'a>(results: &'a SyntaxResults, expected: &str) -> Vec<&'a CallSite> {
    let wanted = format!("constructor_call:{expected}");
    results
        .iter_all_call_sites()
        .filter(|s| s.function_name == wanted)
        .collect()
}

// ---------------------------------------------------------------------------
// 1–5 — `new X(...)` across the five R33-affected languages.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn apex_new_x_emits_constructor_call_site() {
    // Real NPSP-style caller → `new HouseholdMembers(...)`. Matches the
    // §8.4 row literally so the test doubles as documentation of the
    // R33 regression scenario.
    let source = r#"
public class Caller {
    public void run() {
        HouseholdMembers hm = new HouseholdMembers(new List<Id>{ '001000000000000AAA' });
    }
}
"#;
    let (results, _dir) = extract_source("apex", ".cls", source).await;
    let sites = ctor_sites(&results, "HouseholdMembers::new");
    assert_eq!(
        sites.len(),
        1,
        "expected a single `new HouseholdMembers(...)` call site; got: {:#?}",
        results
            .iter_all_call_sites()
            .map(|s| &s.function_name)
            .collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn java_new_x_emits_constructor_call_site() {
    let source = r#"
public class Caller {
    public void run() {
        Foo f = new Foo();
    }
}
"#;
    let (results, _dir) = extract_source("java", ".java", source).await;
    assert_eq!(
        ctor_sites(&results, "Foo::new").len(),
        1,
        "expected a single `new Foo()` call site; got: {:#?}",
        results
            .iter_all_call_sites()
            .map(|s| &s.function_name)
            .collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn csharp_new_x_emits_constructor_call_site() {
    let source = r#"
public class Caller {
    public void Run() {
        var f = new Foo();
    }
}
"#;
    let (results, _dir) = extract_source("csharp", ".cs", source).await;
    assert_eq!(
        ctor_sites(&results, "Foo::new").len(),
        1,
        "expected a single `new Foo()` call site; got: {:#?}",
        results
            .iter_all_call_sites()
            .map(|s| &s.function_name)
            .collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn javascript_new_x_emits_constructor_call_site() {
    let source = r#"
function run() {
    const f = new Foo();
    return f;
}
"#;
    let (results, _dir) = extract_source("javascript", ".js", source).await;
    assert_eq!(
        ctor_sites(&results, "Foo::new").len(),
        1,
        "expected a single `new Foo()` call site; got: {:#?}",
        results
            .iter_all_call_sites()
            .map(|s| &s.function_name)
            .collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn typescript_new_x_emits_constructor_call_site() {
    let source = r#"
function run(): Foo {
    const f = new Foo();
    return f;
}
"#;
    let (results, _dir) = extract_source("typescript", ".ts", source).await;
    assert_eq!(
        ctor_sites(&results, "Foo::new").len(),
        1,
        "expected a single `new Foo()` call site; got: {:#?}",
        results
            .iter_all_call_sites()
            .map(|s| &s.function_name)
            .collect::<Vec<_>>(),
    );
}

// ---------------------------------------------------------------------------
// 6 — Apex `this(...)` explicit-constructor-invocation extraction.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn apex_this_ctor_emits_self_sentinel_with_arg_types() {
    // Sibling-ctor delegation in a single class, two ctors. Asserts the
    // shared extractor recognises `this(...)` as
    // `constructor_call:__self::new`, flips `call_type` accordingly,
    // and populates `arg_types` with an Integer primitive from the
    // literal discriminator. The resolver arm consumes these three
    // signals as a triple.
    let source = r#"
public class Simple {
    public Simple() {
        this(42);
    }
    public Simple(Integer n) {
    }
}
"#;
    let (results, _dir) = extract_source("apex", ".cls", source).await;
    let sites = ctor_sites(&results, "__self::new");
    assert_eq!(
        sites.len(),
        1,
        "expected a single `this(...)` call site; got: {:#?}",
        results
            .iter_all_call_sites()
            .map(|s| &s.function_name)
            .collect::<Vec<_>>(),
    );
    let site = sites[0];
    assert_eq!(
        site.arg_types,
        vec![ApexTypeRef::Primitive {
            name: "Integer".to_string(),
        }],
        "literal `42` should infer to Primitive(\"Integer\") — the resolver uses this \
         as the overload discriminator in §3.2's `r23_a1_overloaded_ctors` fixture shape",
    );
}

// ---------------------------------------------------------------------------
// 7 — Apex `super(...)` explicit-constructor-invocation extraction.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn apex_super_ctor_emits_super_sentinel_with_arg_types() {
    // Subclass ctor delegating to its declared parent. The file
    // deliberately declares both parent and child in the same source
    // file so the fixture is self-contained; the extractor does not
    // need cross-file symbol resolution for this assertion — only the
    // sentinel-plus-arg-types shape.
    let source = r#"
public class Parent {
    public Parent(String s) { }
}
public class Child extends Parent {
    public Child() {
        super('hello');
    }
}
"#;
    let (results, _dir) = extract_source("apex", ".cls", source).await;
    let sites = ctor_sites(&results, "__super::new");
    assert_eq!(
        sites.len(),
        1,
        "expected a single `super(...)` call site; got: {:#?}",
        results
            .iter_all_call_sites()
            .map(|s| &s.function_name)
            .collect::<Vec<_>>(),
    );
    let site = sites[0];
    assert_eq!(
        site.arg_types,
        vec![ApexTypeRef::Primitive {
            name: "String".to_string(),
        }],
        "literal `'hello'` should infer to Primitive(\"String\")",
    );
}

// ---------------------------------------------------------------------------
// 8 — Apex inner-class `super(...)` range-shape (resolver dependency).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn apex_inner_super_ctor_site_falls_inside_inner_class_range() {
    // Outer has no ctor / no `extends`; Inner extends a separate Parent
    // and calls `super(...)`. The resolver-side `find_enclosing_type`
    // picks the innermost type containing the call site — which only
    // works if the extractor records the call site's location inside
    // the inner class's declaration range. That range relationship is
    // what this test pins.
    //
    // We do **not** re-test innermost-wins here (that is covered by
    // `super_ctor_delegates_to_parent_class_ctor` in the resolver unit
    // tests). The goal is to lock the extractor-emitted range shape the
    // resolver depends on.
    let source = r#"
public class Parent {
    public Parent() { }
}
public class Outer {
    public class Inner extends Parent {
        public Inner() {
            super();
        }
    }
}
"#;
    let (results, _dir) = extract_source("apex", ".cls", source).await;
    let sites = ctor_sites(&results, "__super::new");
    assert_eq!(
        sites.len(),
        1,
        "expected a single `super(...)` call site inside Inner; got: {:#?}",
        results
            .iter_all_call_sites()
            .map(|s| &s.function_name)
            .collect::<Vec<_>>(),
    );

    // The Inner class is the second declared class and contains the
    // only `super()`. Hardcoding the line is brittle across source
    // edits — instead, find both class nodes the symbol extractor
    // emitted and assert the call site's range is strictly inside the
    // inner one's range and inside the outer one's range (an inner
    // class is by definition contained in its outer). Strict-inside
    // here means "inner's declaration starts at or after outer's and
    // the call site sits within both".
    let inner = results
        .symbols
        .iter()
        .find(|n| n.fqn.ends_with("Outer.Inner") || n.fqn.ends_with("Inner"))
        .filter(|n| n.fqn.contains("Inner"))
        .expect("Inner class symbol should be emitted by the Apex symbol extractor");
    let outer = results
        .symbols
        .iter()
        .find(|n| {
            (n.fqn.ends_with("Outer") || n.fqn.contains("::Outer"))
                && !n.fqn.contains("Inner")
                && !n.fqn.contains("Parent")
        })
        .expect("Outer class symbol should be emitted");

    let site = sites[0];
    let call_line = site.location.start_line;

    assert!(
        call_line >= inner.location.start_line && call_line <= inner.location.end_line,
        "super() call site line {} must lie inside Inner range {}..={}; \
         otherwise the resolver's `find_enclosing_type` cannot pick Inner over Outer. \
         Inner.fqn={}, call site={:?}",
        call_line,
        inner.location.start_line,
        inner.location.end_line,
        inner.fqn,
        site.location,
    );
    assert!(
        call_line >= outer.location.start_line && call_line <= outer.location.end_line,
        "super() call site line {} must also lie inside Outer range {}..={} \
         (inner-class containment is a basic source-position invariant). \
         Outer.fqn={}, call site={:?}",
        call_line,
        outer.location.start_line,
        outer.location.end_line,
        outer.fqn,
        site.location,
    );

    // The innermost-wins tiebreaker requires Inner to be **strictly
    // narrower** than Outer — otherwise `find_enclosing_type` can tie.
    // Sanity-check the source really declares that nesting.
    let inner_span = inner.location.end_line - inner.location.start_line;
    let outer_span = outer.location.end_line - outer.location.start_line;
    assert!(
        inner_span < outer_span,
        "Inner span ({inner_span} lines) must be narrower than Outer span \
         ({outer_span} lines) for the resolver's span-based innermost-wins \
         tiebreaker to pick Inner over Outer",
    );
}

// ---------------------------------------------------------------------------
// 9 — Apex `arg_types` population correctness.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn apex_new_logger_populates_arg_types_with_string_and_integer() {
    // Verbatim §8.4 row: `new Logger('tag', 42)` →
    // arg_types == [Primitive("String"), Primitive("Integer")].
    // The resolver arm in `signature_matcher` consumes this exact
    // shape for overload disambiguation.
    let source = r#"
public class Caller {
    public void run() {
        Logger l = new Logger('tag', 42);
    }
}
"#;
    let (results, _dir) = extract_source("apex", ".cls", source).await;
    let sites = ctor_sites(&results, "Logger::new");
    assert_eq!(
        sites.len(),
        1,
        "expected a single `new Logger(...)` call site"
    );
    let site = sites[0];
    assert_eq!(
        site.arg_types,
        vec![
            ApexTypeRef::Primitive {
                name: "String".to_string(),
            },
            ApexTypeRef::Primitive {
                name: "Integer".to_string(),
            },
        ],
        "`new Logger('tag', 42)` must infer to [String, Integer] — this is the \
         overload-discriminator shape TR-A.1's `signature_matcher::rank_candidates` \
         expects from `CallSite.arg_types`",
    );
}
