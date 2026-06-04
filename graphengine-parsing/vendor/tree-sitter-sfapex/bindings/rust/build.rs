//! Build script for the vendored Apex/SOQL/SOSL tree-sitter grammars.
//!
//! Each parser is self-contained (no external scanner), so we just compile the
//! three `parser.c` files into a single native static library. The grammar
//! sources are ABI version 14, compatible with tree-sitter 0.20.

use std::path::PathBuf;

fn main() {
    // Crate root is `<repo>/graphengine-parsing/vendor/tree-sitter-sfapex`.
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let grammars: [(&str, PathBuf); 3] = [
        ("apex", crate_dir.join("apex").join("src")),
        ("soql", crate_dir.join("soql").join("src")),
        ("sosl", crate_dir.join("sosl").join("src")),
    ];

    let mut build = cc::Build::new();
    build
        .flag_if_supported("-std=c11")
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .flag_if_supported("-Wno-trigraphs");

    // Each grammar ships its own `tree_sitter/parser.h` — use per-file includes
    // so the three parsers do not collide on header search paths.
    for (_name, src_dir) in &grammars {
        let parser = src_dir.join("parser.c");
        build.file(&parser);
        build.include(src_dir);
        println!("cargo:rerun-if-changed={}", parser.display());
    }

    build.compile("tree_sitter_sfapex_vendored");
}
