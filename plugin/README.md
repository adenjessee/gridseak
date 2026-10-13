# GridSeak plugin

One plugin tree: slim MCP + host hooks that call `gridseak gate`.

## Install the binary first

```sh
curl -fsSL https://raw.githubusercontent.com/adenjessee/gridseak-graphengine/main/scripts/install.sh | bash
export PATH="$HOME/.gridseak/bin:$PATH"
gridseak --version
gridseak setup
```

## What the plugin loads

- MCP: `gridseak mcp --slim` — `route`, `verify_claim`, `blast_radius`
  (symbol or file), `diff_impact`, `gate_status`.
- Hooks, three layers (Cursor 3.20.21 runs all of them):
  1. `~/.cursor/hooks.json` — `preToolUse`
     (Write/StrReplace/Delete/Shell, `failClosed`) plus
     `beforeShellExecution` for `rm` / `git rm`.
  2. this plugin's `plugin.json` / `.cursor-plugin/plugin.json`
  3. `~/.claude/settings.json` `PreToolUse` (`gridseak-gate.sh claude`)
  Cursor `preToolUse` does not enforce `ask` — the `cursor-tool`
  host maps ask → deny. `setup --verify` fails if Claude
  `PreToolUse` is an empty array.
- Compaction card: `gridseak gate --card`.

The other MCP verbs stay on the full CLI (`gridseak mcp` without `--slim`).
They are not in the always-on schema.

Marketplace steps: `docs/workstreams/master-plan/evidence/12-P3-MARKETPLACE.md`.
