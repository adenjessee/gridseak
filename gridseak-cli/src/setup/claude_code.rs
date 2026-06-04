//! Claude Code — prints the official `claude mcp add` invocation.
//!
//! Claude Code stores MCP server config in its own state directory in a
//! format that is not safe to hand-edit (it gets rewritten on every
//! agent run). The official command is `claude mcp add <name> <bin> mcp`,
//! which we print rather than try to invoke ourselves — the user keeps
//! agency over whether to run it.
//!
//! Limitation: Claude Code does not yet implement the MCP `sampling`
//! capability. GridSeak's analysis MCP does not use sampling — it only
//! needs `tools` — so this limitation does not affect us. We surface the
//! command and let the user paste it.

pub fn print_instructions(binary: &str) {
    println!("[Claude Code]");
    println!("  Run once in your terminal to register the GridSeak MCP:");
    println!();
    println!("    claude mcp add gridseak {binary} mcp");
    println!();
    println!("  GridSeak's analysis MCP does not require the `sampling` capability,");
    println!("  so this works on every Claude Code version that supports MCP.");
}
