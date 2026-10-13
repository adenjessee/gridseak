//! Diagnostics: print every occurrence in one document of a SCIP index.
//!
//! `cargo run -p graphengine-scip-adapter --example scip_dump -- <index.scip> <relative/path>`

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let index = graphengine_scip_adapter::ScipIndex::from_file(&args[1]).expect("load index");
    for (symbol, roles, r) in index.occurrences_in(&args[2]) {
        println!(
            "{}:{}-{}:{} roles={} {}",
            r.start_line + 1,
            r.start_character,
            r.end_line + 1,
            r.end_character,
            roles,
            symbol
        );
        if roles & 1 != 0 {
            for iface in index.interfaces_implemented_by(&symbol) {
                println!("      implements {iface}");
            }
        }
    }
}
