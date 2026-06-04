use tree_sitter::Parser;
use tree_sitter_rust::language;

fn main() {
    let mut parser = Parser::new();
    parser.set_language(language()).unwrap();

    let code = r#"
pub trait LayoutAlgorithm {
    fn apply_layout(&self);
    fn apply_layout_with_connections(&self) {
        self.apply_layout();
    }
}

impl LayoutAlgorithm for MyType {
    fn apply_layout(&self) {}
    fn apply_layout_with_connections(&self) {}
}
"#;

    let tree = parser.parse(code, None).unwrap();
    let root_node = tree.root_node();

    println!("Tree-sitter Rust AST for trait/impl:");
    print_node(&root_node, 0, code);
}

fn print_node(node: &tree_sitter::Node, depth: usize, code: &str) {
    let indent = "  ".repeat(depth);
    let kind = node.kind();
    let text = node.utf8_text(code.as_bytes()).unwrap_or("");
    let text_preview = if text.len() > 50 {
        format!("{}...", &text[..50])
    } else {
        text.to_string()
    };

    println!(
        "{}{} [{}:{}] '{}'",
        indent,
        kind,
        node.start_byte(),
        node.end_byte(),
        text_preview.replace('\n', "\\n")
    );

    // Check if this is a function_item and show its parents
    if kind == "function_item" {
        println!("{}  [FUNCTION PARENTS:]", indent);
        let mut current = node.parent();
        let mut parent_depth = 0;
        while let Some(parent) = current {
            parent_depth += 1;
            let parent_text = parent.utf8_text(code.as_bytes()).unwrap_or("");
            let parent_preview = if parent_text.len() > 30 {
                format!("{}...", &parent_text[..30])
            } else {
                parent_text.to_string()
            };
            println!(
                "{}    parent[{}]: {} '{}'",
                indent,
                parent_depth,
                parent.kind(),
                parent_preview.replace('\n', "\\n")
            );
            if parent_depth > 10 {
                break;
            }
            current = parent.parent();
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            print_node(&child, depth + 1, code);
        }
    }
}
