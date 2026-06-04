use tree_sitter::Parser;
use tree_sitter_rust::language;

fn main() {
    let mut parser = Parser::new();
    parser.set_language(language()).unwrap();

    let code = r#"
mod grandparent {
    pub mod parent {
        pub mod child {
            pub fn target() {}
            pub fn helper() {}
        }

        pub use self::child::target as alias_target;
        pub use self::child::{target as re_alias, helper};
        pub use crate::grandparent::parent::child::*;
        pub use crate::grandparent::parent::{self as parent_alias, child as child_alias};
    }
}

use crate::grandparent::parent::child::target;
use crate::grandparent::parent::child::{target as top_alias, helper};
use crate::grandparent::parent::child::*;
use crate::grandparent::parent::{self as parent_mod, child as child_mod};

fn main() {}
"#;

    let tree = parser.parse(code, None).unwrap();
    let root_node = tree.root_node();

    println!("Tree-sitter Rust AST:");
    print_node(&root_node, 0);
}

fn print_node(node: &tree_sitter::Node, depth: usize) {
    let indent = "  ".repeat(depth);
    println!(
        "{}{} [{}:{}]",
        indent,
        node.kind(),
        node.start_byte(),
        node.end_byte()
    );

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            print_node(&child, depth + 1);
        }
    }
}
