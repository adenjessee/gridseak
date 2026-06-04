use graphengine_parsing::application::ports::SyntaxResults;
use graphengine_parsing::domain::{Node, Range};
use graphengine_parsing::infrastructure::lsp::call_resolver::CallResolver;
use graphengine_parsing::module_resolution::ModuleResolver;

fn make_function(fqn: &str, file: &str, start: u32, end: u32) -> Node {
    Node::function(
        fqn.to_string(),
        Range::with_file(start, 0, end, 1, file.to_string()),
    )
}

#[test]
fn resolves_simple_name_using_module_similarity() {
    let mut syntax = SyntaxResults::new();

    let caller = make_function(
        "crate::service::controller::run",
        "src/service/controller.rs",
        10,
        40,
    );
    let target_same_module = make_function(
        "crate::service::helper::process",
        "src/service/helper.rs",
        5,
        30,
    );
    let target_other_module = make_function(
        "crate::analytics::helper::process",
        "src/analytics/helper.rs",
        8,
        28,
    );

    syntax.add_symbol(caller.clone());
    syntax.add_symbol(target_same_module.clone());
    syntax.add_symbol(target_other_module);

    syntax.add_call_site(
        Range::with_file(20, 4, 20, 18, "src/service/controller.rs"),
        "process".to_string(),
    );

    let mut call_resolver = CallResolver::new();
    call_resolver.prepare(&syntax);
    let module_resolver = ModuleResolver::from_syntax(&syntax);

    let edges = call_resolver
        .resolve_calls(&syntax, &module_resolver)
        .expect("heuristic resolution should succeed");

    assert_eq!(edges.len(), 1, "expected single resolved edge");
    let edge = &edges[0];
    assert_eq!(edge.from_id, caller.id);
    assert_eq!(edge.to_id, target_same_module.id);
}

#[test]
fn resolves_constructor_calls_with_type_fallback() {
    let mut syntax = SyntaxResults::new();

    let caller = make_function(
        "crate::ui::builder::make_widget",
        "src/ui/builder.rs",
        1,
        20,
    );
    let target_primary = make_function(
        "crate::ui::components::Widget::new",
        "src/ui/components/widget.rs",
        3,
        12,
    );
    let target_secondary = make_function(
        "crate::common::widgets::Widget::new",
        "src/common/widgets/widget.rs",
        5,
        16,
    );

    syntax.add_symbol(caller.clone());
    syntax.add_symbol(target_primary.clone());
    syntax.add_symbol(target_secondary);

    syntax.add_call_site(
        Range::with_file(8, 2, 8, 18, "src/ui/builder.rs"),
        "constructor_call:crate::ui::components::Widget::new()".to_string(),
    );

    let mut call_resolver = CallResolver::new();
    call_resolver.prepare(&syntax);
    let module_resolver = ModuleResolver::from_syntax(&syntax);

    let edges = call_resolver
        .resolve_calls(&syntax, &module_resolver)
        .expect("heuristic resolution should succeed");

    assert_eq!(edges.len(), 1, "expected single resolved edge");
    let edge = &edges[0];
    assert_eq!(edge.from_id, caller.id);
    assert_eq!(edge.to_id, target_primary.id);
}

#[test]
fn resolves_cross_module_function_call() {
    let mut syntax = SyntaxResults::new();

    let caller = make_function("crate::mod_a::a4_cross_b1", "src/mod_a.rs", 10, 30);
    let local_target = make_function("crate::mod_a::a1_base", "src/mod_a.rs", 1, 8);
    let cross_target = make_function("crate::mod_b::b1_base", "src/mod_b.rs", 1, 8);

    syntax.add_symbol(caller.clone());
    syntax.add_symbol(local_target);
    syntax.add_symbol(cross_target.clone());

    syntax.add_call_site(
        Range::with_file(18, 4, 18, 25, "src/mod_a.rs"),
        "mod_b::b1_base".to_string(),
    );

    let mut call_resolver = CallResolver::new();
    call_resolver.prepare(&syntax);
    let module_resolver = ModuleResolver::from_syntax(&syntax);

    let edges = call_resolver
        .resolve_calls(&syntax, &module_resolver)
        .expect("heuristic resolution should succeed");

    assert!(edges
        .iter()
        .any(|edge| edge.from_id == caller.id && edge.to_id == cross_target.id));
}
