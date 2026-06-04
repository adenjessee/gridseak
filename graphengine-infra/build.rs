fn main() {
    // Skip tree-sitter compilation entirely - use pre-compiled libraries from crates
    // The tree-sitter-rust and tree-sitter-python crates already include the compiled libraries
    println!("cargo:warning=Using pre-compiled tree-sitter libraries from crates.io");
    println!("cargo:rerun-if-changed=build.rs");
}
