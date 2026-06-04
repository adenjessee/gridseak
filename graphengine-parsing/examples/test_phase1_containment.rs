//! Test Phase 1 containment implementation with real crate
use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
use graphengine_parsing::domain::{Confidence, EdgeKind, NodeKind};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let db_path = "../function-relationship-test/rust.db";
    let root = PathBuf::from("../function-relationship-test");

    // Remove old database
    let _ = std::fs::remove_file(db_path);

    println!("=== Phase 1 Containment Test ===");
    println!("Parsing crate: {}", root.display());
    println!("Database: {}\n", db_path);

    // Create use case and parse
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        "rust".to_string(),
        Confidence::Medium,
        db_path,
    )
    .await?;

    let resolved = use_case.parse(root, "rust".to_string()).await?;

    let graph = resolved.graph();

    println!("=== Results ===");
    println!("Total nodes: {}", graph.node_count());
    println!("Total edges: {}\n", graph.edge_count());

    // Count nodes by kind
    let crates = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Crate)
        .count();
    let files = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File)
        .count();
    let folders = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Folder)
        .count();
    let modules = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Module)
        .count();
    let functions = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .count();

    println!("Nodes by kind:");
    println!("  Crate: {}", crates);
    println!("  File: {}", files);
    println!("  Folder: {}", folders);
    println!("  Module: {}", modules);
    println!("  Function: {}\n", functions);

    // Count edges by kind
    let call_edges = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Call)
        .count();
    let import_edges = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Import)
        .count();
    let contains_edges = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .count();

    println!("Edges by kind:");
    println!("  Call: {}", call_edges);
    println!("  Import: {}", import_edges);
    println!("  Contains: {}\n", contains_edges);

    // Show Crate nodes
    println!("=== Crate Nodes ===");
    for node in graph.nodes.iter().filter(|n| n.kind == NodeKind::Crate) {
        println!("  {}", node.fqn);
    }
    println!();

    // Show File nodes
    println!("=== File Nodes (first 10) ===");
    for (i, node) in graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File)
        .take(10)
        .enumerate()
    {
        println!("  {}. {}", i + 1, node.fqn);
    }
    println!();

    // Show Contains edges
    println!("=== Contains Edges (first 20) ===");
    let mut count = 0;
    for edge in graph.edges.iter().filter(|e| e.kind == EdgeKind::Contains) {
        if count >= 20 {
            break;
        }
        let from_node = graph.nodes.iter().find(|n| n.id == edge.from_id);
        let to_node = graph.nodes.iter().find(|n| n.id == edge.to_id);
        if let (Some(from), Some(to)) = (from_node, to_node) {
            println!(
                "  {} ({:?}) → {} ({:?})",
                from.fqn, from.kind, to.fqn, to.kind
            );
            count += 1;
        }
    }
    println!();

    // Count Module → Function edges
    let mod_to_func = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .filter_map(|e| {
            let from = graph.nodes.iter().find(|n| n.id == e.from_id)?;
            let to = graph.nodes.iter().find(|n| n.id == e.to_id)?;
            if from.kind == NodeKind::Module && to.kind == NodeKind::Function {
                Some(())
            } else {
                None
            }
        })
        .count();

    // Count Module → Module edges
    let mod_to_mod = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .filter_map(|e| {
            let from = graph.nodes.iter().find(|n| n.id == e.from_id)?;
            let to = graph.nodes.iter().find(|n| n.id == e.to_id)?;
            if from.kind == NodeKind::Module && to.kind == NodeKind::Module {
                Some(())
            } else {
                None
            }
        })
        .count();

    // Count Crate → File edges
    let crate_to_file = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .filter_map(|e| {
            let from = graph.nodes.iter().find(|n| n.id == e.from_id)?;
            let to = graph.nodes.iter().find(|n| n.id == e.to_id)?;
            if from.kind == NodeKind::Crate && to.kind == NodeKind::File {
                Some(())
            } else {
                None
            }
        })
        .count();

    // Count Crate → Module edges
    let crate_to_mod = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .filter_map(|e| {
            let from = graph.nodes.iter().find(|n| n.id == e.from_id)?;
            let to = graph.nodes.iter().find(|n| n.id == e.to_id)?;
            if from.kind == NodeKind::Crate && to.kind == NodeKind::Module {
                Some(())
            } else {
                None
            }
        })
        .count();

    // Count File → Module edges
    let file_to_mod = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .filter_map(|e| {
            let from = graph.nodes.iter().find(|n| n.id == e.from_id)?;
            let to = graph.nodes.iter().find(|n| n.id == e.to_id)?;
            if from.kind == NodeKind::File && to.kind == NodeKind::Module {
                Some(())
            } else {
                None
            }
        })
        .count();

    // Count Folder → File edges
    let folder_to_file = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .filter_map(|e| {
            let from = graph.nodes.iter().find(|n| n.id == e.from_id)?;
            let to = graph.nodes.iter().find(|n| n.id == e.to_id)?;
            if from.kind == NodeKind::Folder && to.kind == NodeKind::File {
                Some(())
            } else {
                None
            }
        })
        .count();

    // Count Folder → Folder edges
    let folder_to_folder = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .filter_map(|e| {
            let from = graph.nodes.iter().find(|n| n.id == e.from_id)?;
            let to = graph.nodes.iter().find(|n| n.id == e.to_id)?;
            if from.kind == NodeKind::Folder && to.kind == NodeKind::Folder {
                Some(())
            } else {
                None
            }
        })
        .count();

    println!("=== Containment Edge Breakdown ===");
    println!("  Module → Function: {}", mod_to_func);
    println!("  Module → Module: {}", mod_to_mod);
    println!("  Crate → File: {}", crate_to_file);
    println!("  Crate → Module: {}", crate_to_mod);
    println!("  File → Module: {}", file_to_mod);
    println!("  Folder → File: {}", folder_to_file);
    println!("  Folder → Folder: {}\n", folder_to_folder);

    // Verify Phase 1 requirements
    println!("=== Phase 1 Verification ===");
    let phase1_passed = crates > 0 && files > 0 && contains_edges > 0 && mod_to_func > 0;

    if phase1_passed {
        println!("✅ Phase 1 PASSED!");
        println!("  ✅ Crate nodes created: {}", crates);
        println!("  ✅ File nodes created: {}", files);
        println!("  ✅ Contains edges created: {}", contains_edges);
        println!("  ✅ Module → Function edges: {}", mod_to_func);
        if mod_to_mod > 0 {
            println!("  ✅ Module → Module edges: {}", mod_to_mod);
        }
    } else {
        println!("❌ Phase 1 FAILED!");
        if crates == 0 {
            println!("  ❌ No Crate nodes created");
        }
        if files == 0 {
            println!("  ❌ No File nodes created");
        }
        if contains_edges == 0 {
            println!("  ❌ No Contains edges created");
        }
        if mod_to_func == 0 {
            println!("  ❌ No Module → Function edges created");
        }
    }

    // Verify Phase 2 requirements
    println!("\n=== Phase 2 Verification ===");
    let phase2_passed = crate_to_file > 0 && crate_to_mod > 0 && file_to_mod > 0;

    if phase2_passed {
        println!("✅ Phase 2 PASSED!");
        println!("  ✅ Crate → File edges: {}", crate_to_file);
        println!("  ✅ Crate → Module edges: {}", crate_to_mod);
        println!("  ✅ File → Module edges: {}", file_to_mod);
    } else {
        println!("❌ Phase 2 FAILED!");
        if crate_to_file == 0 {
            println!("  ❌ No Crate → File edges created");
        }
        if crate_to_mod == 0 {
            println!("  ❌ No Crate → Module edges created");
        }
        if file_to_mod == 0 {
            println!("  ❌ No File → Module edges created");
        }
    }

    // Verify Phase 3 requirements
    println!("\n=== Phase 3 Verification ===");
    let phase3_passed = folders > 0 && folder_to_file > 0;

    if phase3_passed {
        println!("✅ Phase 3 PASSED!");
        println!("  ✅ Folder nodes created: {}", folders);
        println!("  ✅ Folder → File edges: {}", folder_to_file);
        println!("  ✅ Folder → Folder edges: {}", folder_to_folder);
    } else {
        println!("❌ Phase 3 FAILED!");
        if folders == 0 {
            println!("  ❌ No Folder nodes created");
        }
        if folder_to_file == 0 {
            println!("  ❌ No Folder → File edges created");
        }
    }

    Ok(())
}
