# GridSeak plugin

One plugin tree: slim MCP + host hooks that call `gridseak gate`, plus the
`audit-my-agent` skill and `/gridseak:audit` command for Claude Code.

## Install the binary first

```sh
curl -fsSL https://gridseak.com/install.sh | sh
export PATH="$HOME/.gridseak/bin:$PATH"
gridseak --version
gridseak scan .
gridseak setup
```

## Install the plugin (Claude Code)

```text
/plugin marketplace add adenjessee/gridseak
/plugin install gridseak@gridseak
```

The marketplace manifest is `.claude-plugin/marketplace.json` at the repo
root; it points at this directory.

## Audit your agent

In Claude Code, in your repo, say *"audit my agent on this repo"* (or run
`/gridseak:audit`, optionally with flags such as `--tasks 3 --arms plain`).
The `audit-my-agent` skill ([`skills/audit-my-agent/SKILL.md`](skills/audit-my-agent/SKILL.md))
then:

1. checks `gridseak` is installed (otherwise shows the install command and waits),
2. runs `gridseak scan .` if needed,
3. runs `gridseak audit --plan-only` and shows the tasks and a cost estimate,
4. asks you to confirm, then runs `gridseak audit`,
5. opens `report.html` and summarizes it.

The card shows, per arm (`plain` = agent alone, `gridseak` = agent + GridSeak
MCP): exact answers, silent misses (an incomplete caller list presented as
complete), flagged misses, minutes, cost and tokens. Files land in
`.gridseak/audit/<timestamp>/` (`report.html`, `report.md`, `results.json`).

Cost: real money through your Claude Code login, about $0.25-3.50 per task
run at API prices; 6 tasks x 2 arms is typically $8-25, capped by
`--max-usd-per-task` (default $4) per run.

Languages: Go today. TypeScript works when the scan is fully
compiler-indexed. Any scan with non-compiler call edges is refused.

Other agents (Cursor, Codex, ...) can follow `SKILL.md` directly: it is plain
Markdown with the same steps.

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
