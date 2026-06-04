# Cursor / Claude Code / Codex / Windsurf MCP setup

`gridseak setup` writes the MCP server config and the agent rule
that teaches Cursor when to call each of the fourteen GridSeak tools.
Most users will never need this document. It exists for the cases
where setup didn't work and you want to understand or fix the wiring
by hand.

## What `gridseak setup` does

Defaults (no flags):

- **Cursor** — writes `~/.cursor/mcp.json` (adds `mcpServers.gridseak`,
  preserves any other entries) and `~/.cursor/rules/gridseak.mdc`
  (the agent rule body).
- **Windsurf** — writes `~/.codeium/windsurf/mcp_config.json` if
  Windsurf is installed.
- **Claude Code** — prints the `claude mcp add` command (Claude
  Code's MCP state is managed by `claude` itself).
- **Codex** — prints the TOML block to paste into `~/.codex/config.toml`.

Useful flags:

- `--verify` — post-install sanity check.
- `--no-rule` — skip the Cursor rule file.
- `--workspace` — write Cursor's mcp.json to `<cwd>/.cursor/mcp.json`
  instead of the home-scope path.
- `--only cursor` (repeatable) — restrict to one target.
- `--dry-run` — show what would change without writing.
- `--print-rule` — print the rule body to stdout, useful for piping
  into a custom location.

## What the Cursor mcp.json entry looks like

```json
{
  "mcpServers": {
    "gridseak": {
      "command": "gridseak",
      "args": ["mcp"]
    }
  }
}
```

If `gridseak` is not on `$PATH`, replace `"command": "gridseak"` with
the absolute path (e.g. `"/Users/me/.gridseak/bin/gridseak"`). The
`--command <path>` flag on `gridseak setup` does this for you.

## The Cursor rule body

The Cursor rule (`~/.cursor/rules/gridseak.mdc`) teaches the agent:

- When to call each of the fourteen MCP tools (trigger phrases per tool).
- The tier-honest evidence model (Tier 0 tree-sitter, Tier 1 grep,
  Tier 3 LSP) and the directive to quote the tier when stating a
  structural fact.
- When *not* to call any GridSeak tool (trivial edits, pure-string
  refactors).

You can preview it with `gridseak setup --print-rule`. The source
of truth is `.cursor/rules/gridseak.mdc` in this repo; `gridseak
setup` embeds it at compile time and writes the embedded copy.

## If the agent isn't calling GridSeak

Run `gridseak setup --verify`. The verifier checks:

- `~/.cursor/mcp.json` exists and has `mcpServers.gridseak` with a
  resolvable `command`.
- `~/.cursor/rules/gridseak.mdc` exists and is non-empty.
- The `gridseak` binary on `$PATH` matches the one referenced in
  mcp.json.

Common failure modes:

- **Two installs on `$PATH`** — the one in mcp.json may not be the
  one your shell resolves. The verifier prints a `NOTE` line if it
  detects this. Pick one and remove the other.
- **Cursor not restarted** — MCP servers don't hot-reload. Restart
  Cursor after running `gridseak setup`.
- **Rule file missing** — if you passed `--no-rule`, re-run setup
  without it. The agent still works without the rule, but it calls
  GridSeak tools much less reliably.

## Cleaning up old entries

If you had an earlier version wired up under a different name (e.g.
`user-gridseak`, `gridseak-dashboard`), open `~/.cursor/mcp.json`
and remove the stale entry. `gridseak setup` only manages the
`mcpServers.gridseak` key; it preserves all other entries.

## What if I don't want any of this?

You can run the MCP server manually:

```sh
gridseak mcp
```

It speaks JSON-RPC over stdio. You can point any MCP-compatible
client at it without going through `gridseak setup`.
