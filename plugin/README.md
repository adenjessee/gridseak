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
- Claim check (Claude `Stop`): `gridseak gate --stop`. When the agent's
  final message claims completeness ("updated all callers", "no other
  call sites"), the hook diffs the working tree against `HEAD`, finds
  functions that were removed, renamed or re-signatured, and checks each
  graph caller: it must be in a changed file, and a caller of a removed
  name must no longer mention it.
  - A compiler/LSP-witnessed miss blocks the stop and hands the agent
    the list of callers.
  - A tree-sitter-only miss is shown to you without blocking.
  - Languages: Go, Rust, TypeScript/JavaScript and Python. Module-level
    entries that are only import lines are not counted as callers for
    signature changes.
  - A `git grep -w` fallback also lists references the graph has no
    edge for (Go `_examples/`, build-tagged files, untyped Python calls):
    any leftover mention of a removed name, and mentions of a re-signatured
    name in files the agent didn't touch. These text matches are labeled
    heuristic and never block on their own. Skipped for names under 4 characters,
    names another function still defines, and names with more than 50
    matches.
  - It never blocks twice in a row (`stop_hook_active`).
  - No claim, no scan, or any error → `{}`: the hook fails open.

  Limits: the diff is against `HEAD`, so edits made before the session
  count too. A changed file counts as an updated caller for signature
  changes; the hook doesn't check the call site itself. Callers come
  from the last scan, so rescan after big refactors.

The other MCP verbs stay on the full CLI (`gridseak mcp` without `--slim`).
They are not in the always-on schema.

Marketplace steps: `docs/workstreams/master-plan/evidence/12-P3-MARKETPLACE.md`.
