//! One-shot helper to regenerate `tests/fixtures/python/index.scip`.

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
    doc.language = "python".into();
    doc.relative_path = "main.py".into();
    doc.occurrences.push(Occurrence {
        range: vec![1, 4, 10],
        symbol: "scip-python python . main.callee().".into(),
        symbol_roles: SymbolRole::Definition.value(),
        ..Default::default()
    });
    doc.occurrences.push(Occurrence {
        range: vec![4, 4, 10],
        symbol: "scip-python python . main.callee().".into(),
        symbol_roles: 0,
        ..Default::default()
    });
    index.documents.push(doc);

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python/index.scip");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    scip::write_message_to_file(&out, index).expect("write fixture");
    println!("wrote {}", out.display());
}
