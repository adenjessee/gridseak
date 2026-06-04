//! Cursor rule body — single source of truth.
//!
//! The rule body is the markdown that teaches the agent when to call each
//! GridSeak MCP tool. It lives in `.cursor/rules/gridseak.mdc` at the
//! workspace root so this repo's own Cursor agent reads the same body
//! `gridseak setup` writes to user homes. `include_str!` embeds it into
//! the binary at compile time — no filesystem dependency at runtime.
//!
//! When you edit the rule, edit `.cursor/rules/gridseak.mdc`. This module
//! re-exports it.

pub const CURSOR_RULE_BODY: &str = include_str!("../../../.cursor/rules/gridseak.mdc");
