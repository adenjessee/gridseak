# P3 — plugin marketplace submit checklist

Submit **only after** `make gate` is green **and** a headless live
table exists under `target/live-ab/report.md` with treatment harm
strictly below control. Official Cursor/Anthropic stores still wait
for a second-machine smoke. Homebrew is after the plugin works — do
not invert.

n=1 IDE rows are in `12-LIVE_GATE-results.md`. That is not enough
to submit. This file is still a checklist, not a submission receipt.

## Manifests (this repo)

| path | host |
|---|---|
| `plugin/plugin.json` | Agent Plugins generic |
| `plugin/.cursor-plugin/plugin.json` | Cursor |
| `plugin/.claude-plugin/plugin.json` | Claude Code |
| `plugin/hooks/gridseak-gate.sh` | calls `gridseak gate` |

Always-on MCP: `gridseak mcp --slim` → router + `verify_claim` +
`blast_radius` (file or symbol) + `diff_impact` + `gate_status`.

## Community first (SHA-pinned)

1. **cursor.directory** — draft listing, pin commit SHA of the plugin
   tree, require `gridseak` on PATH (`scripts/install.sh`).
2. **Claude community marketplace** — same SHA, document
   `PreToolUse` matcher `Edit|Write|MultiEdit` + `Bash`.
   Cursor listing must name `preToolUse` (`Write|StrReplace|Delete|Shell`)
   plus `beforeShellExecution` for `rm` / `git rm`. Note that Cursor
   `preToolUse` does not enforce `ask`.
3. Second machine smoke:
   - install plugin
   - `gridseak scan` on a tiny fixture
   - reproduce P1c NewRouter deny (`make gate`)
4. Official Cursor / Anthropic marketplaces: after step 3 **and**
   either P2c rows or a recorded fixture deny (not a live bakeoff lie).

## Do not

- Ship Homebrew before a stranger can reproduce a denial.
- Advertise fourteen always-on MCP tools in the plugin schema.
- Claim live A/B wins from `make agent-ab`.
