//! Codex (OpenAI Codex CLI) — prints the TOML snippet to paste into
//! `~/.codex/config.toml`.
//!
//! Codex's config is a hand-edited TOML file. We could write to it
//! programmatically with the `toml` crate, but the file may contain
//! user comments and ordering that round-tripping would mangle, so we
//! print the snippet and let the user paste. The MCP block is small
//! (3 lines) and the location is well-documented.

pub fn print_instructions(binary: &str) {
    println!("[Codex]");
    println!("  Append to ~/.codex/config.toml:");
    println!();
    println!("    [mcp_servers.gridseak]");
    println!("    command = \"{binary}\"");
    println!("    args = [\"mcp\"]");
}
