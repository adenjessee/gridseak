//! GraphEngine Template Query CLI
//!
//! Purpose: expose filtered raw graph relationships (nodes/edges) from the SQLite DB
//! using the existing TOML template format consumed by `TemplateService`.
//!
//! This is intended as an Unreal-sidecar friendly binary: it prints JSON to stdout.

use clap::{Parser, Subcommand};
use graphengine_infra::services::template_service::TemplateService;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ge-template")]
#[command(about = "Query GraphEngine SQLite DB using TOML templates and output JSON", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Pretty-print JSON output (human-readable)
    #[arg(long)]
    pretty: bool,

    /// Include execution plan details in metadata.explain (machine-readable)
    #[arg(long)]
    explain: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a template query against a SQLite DB and print JSON
    Query {
        /// SQLite database path produced by GraphEngine parsing
        #[arg(short, long)]
        db: PathBuf,

        /// TOML template path
        #[arg(short, long)]
        template: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Query { db, template } => {
            let service = TemplateService::new(&db.to_string_lossy())?;
            let json = service.get_custom_graph_with_explain(&template, cli.explain)?;

            if cli.pretty {
                let value: serde_json::Value = serde_json::from_str(&json)?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                println!("{}", json);
            }
        }
    }

    Ok(())
}
