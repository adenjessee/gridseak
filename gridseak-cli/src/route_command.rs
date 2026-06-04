//! `gridseak route "question"` — deterministic symptom → tool debug CLI.

use anyhow::Result;
use clap::Args;

use crate::intent_router::{route, routing_table, RouteInput};

#[derive(Args, Debug, Clone)]
pub struct RouteArgs {
    /// Plain-language question to route (quoted).
    pub question: String,

    #[arg(long)]
    pub file: Option<String>,

    #[arg(long)]
    pub symbol: Option<String>,
}

pub fn run_route(args: RouteArgs) -> Result<()> {
    let decision = route(RouteInput {
        question: &args.question,
        file_hint: args.file.as_deref(),
        symbol_hint: args.symbol.as_deref(),
    });
    println!("recommended_tool: {}", decision.tool.mcp_name());
    println!("matched_symptom: {}", decision.matched_symptom);
    println!("preconditions: {:?}", decision.preconditions);
    println!();
    println!("routing_table:");
    for (symptom, tool, pre) in routing_table() {
        println!("  - {symptom} → {} ({pre:?})", tool.mcp_name());
    }
    Ok(())
}
