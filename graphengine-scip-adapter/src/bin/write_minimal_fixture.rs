//! One-shot helper to regenerate `tests/fixtures/minimal/index.scip`.
//! Run: `cargo run -p graphengine-scip-adapter --bin write_minimal_fixture`

use std::path::PathBuf;

use protobuf::Enum;
use scip::types::{Document, Index, Metadata, Occurrence, SymbolRole};

fn main() {
    let mut index = Index::new();
    index.metadata = protobuf::MessageField::some(Metadata {
        project_root: "/fixture".into(),
        ..Default::default()
    });
    let mut doc = Document::new();
    doc.language = "typescript".into();
    doc.relative_path = "src/index.ts".into();
    doc.occurrences.push(Occurrence {
        range: vec![1, 16, 22],
        symbol: "scip-typescript npm . `index`.callee().".into(),
        symbol_roles: SymbolRole::Definition.value(),
        ..Default::default()
    });
    doc.occurrences.push(Occurrence {
        range: vec![4, 9, 15],
        symbol: "scip-typescript npm . `index`.callee().".into(),
        symbol_roles: 0,
        ..Default::default()
    });
    index.documents.push(doc);

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal/index.scip");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    scip::write_message_to_file(&out, index).expect("write fixture");
    println!("wrote {}", out.display());
}
