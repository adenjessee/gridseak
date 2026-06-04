//! T2 end-to-end: node IDs emitted by the extraction pipeline are
//! content-based (`SHA256(fqn || normalized_body_hash)`) and are
//! invariant under purely-cosmetic source edits.
//!
//! These assertions are the contract that unblocks trend / feedback /
//! incremental re-parse going forward. If any of them regress, the T2
//! guarantee has broken and the symptoms will be invisible until a
//! customer reports "all my confirmed findings vanished after the
//! formatter ran".
//!
//! Fixture choice: Apex is the primary language the project ships with
//! an in-process extractor, and its corpus is the one the rev-9 Round 5
//! audit was run on. Testing the extractor we actually ship beats
//! testing a mock.

use graphengine_parsing::application::ports::SyntaxExtractor;
use graphengine_parsing::domain::NodeKind;
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::Path;

async fn parse_one(
    dir: &Path,
    src: &str,
) -> graphengine_parsing::application::ports::SyntaxResults {
    let path = dir.join("Fixture.cls");
    std::fs::write(&path, src).expect("write fixture");
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build extractor");
    extractor
        .extract(std::slice::from_ref(&path))
        .await
        .expect("extract fixture")
}

fn find_method_id(
    results: &graphengine_parsing::application::ports::SyntaxResults,
    method_name: &str,
) -> String {
    results
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Function && n.fqn.contains(method_name))
        .unwrap_or_else(|| {
            panic!(
                "no function node matching `{method_name}` in extracted symbols; \
                 have: {:?}",
                results
                    .symbols
                    .iter()
                    .filter(|n| n.kind == NodeKind::Function)
                    .map(|n| n.fqn.as_str())
                    .collect::<Vec<_>>()
            )
        })
        .id
        .clone()
}

#[tokio::test]
async fn cosmetic_edits_preserve_method_node_id() {
    // Source A: canonical form.
    let src_a = "\
public class Fixture {
    public Integer compute(Integer x) {
        return x + 1;
    }
}
";

    // Source B: blank line inserted above the method, trailing line
    // comment inside the body, indentation widened. Semantically
    // identical to A.
    let src_b = "\
public class Fixture {

    public Integer compute(Integer x) {
            return x + 1; // trailing note
    }
}
";

    let tmp_a = tempfile::tempdir().unwrap();
    let tmp_b = tempfile::tempdir().unwrap();

    let res_a = parse_one(tmp_a.path(), src_a).await;
    let res_b = parse_one(tmp_b.path(), src_b).await;

    let id_a = find_method_id(&res_a, "compute");
    let id_b = find_method_id(&res_b, "compute");

    assert_eq!(
        id_a, id_b,
        "T2 regression: purely cosmetic edit changed the method node ID.\n\
         source A → id={id_a}\nsource B → id={id_b}"
    );
}

#[tokio::test]
async fn semantic_edits_churn_method_node_id() {
    let src_a = "\
public class Fixture {
    public Integer compute(Integer x) {
        return x + 1;
    }
}
";
    // Source C: body changed (`+ 1` → `+ 2`). Must churn.
    let src_c = "\
public class Fixture {
    public Integer compute(Integer x) {
        return x + 2;
    }
}
";

    let tmp_a = tempfile::tempdir().unwrap();
    let tmp_c = tempfile::tempdir().unwrap();

    let res_a = parse_one(tmp_a.path(), src_a).await;
    let res_c = parse_one(tmp_c.path(), src_c).await;

    let id_a = find_method_id(&res_a, "compute");
    let id_c = find_method_id(&res_c, "compute");

    assert_ne!(
        id_a, id_c,
        "T2 regression: a semantic body change did NOT churn the method \
         ID. This would silently hide real diffs in trend analysis.\n\
         source A → id={id_a}\nsource C → id={id_c}"
    );
}

#[tokio::test]
async fn class_node_id_is_stable_under_cosmetic_edits() {
    // The class itself is a Struct node with a body (everything between
    // the braces), so it should also be content-stable across cosmetic
    // edits to its members as long as those edits normalize away.
    let src_a = "\
public class Fixture {
    public Integer compute(Integer x) {
        return x + 1;
    }
}
";
    let src_b = "\
public    class   Fixture  {
    public Integer compute(Integer x) {
        return x + 1;
    }
}
";

    let tmp_a = tempfile::tempdir().unwrap();
    let tmp_b = tempfile::tempdir().unwrap();

    let res_a = parse_one(tmp_a.path(), src_a).await;
    let res_b = parse_one(tmp_b.path(), src_b).await;

    let class_id_a = res_a
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Struct && n.fqn.ends_with("Fixture"))
        .expect("class node in A")
        .id
        .clone();
    let class_id_b = res_b
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Struct && n.fqn.ends_with("Fixture"))
        .expect("class node in B")
        .id
        .clone();

    assert_eq!(
        class_id_a, class_id_b,
        "T2 regression: extra whitespace between tokens changed the class \
         node ID. normalize_body should collapse it.\n\
         A → {class_id_a}\nB → {class_id_b}"
    );
}
