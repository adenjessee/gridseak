# GridSeak CLI And MCP Setup

Status: draft for the local-first desktop pilot.

## What Ships

The desktop release bundle includes a `gridseak` executable alongside the app's
engine sidecars. It provides:

- terminal access to local project scan history
- `gridseak mcp` for Cursor, VS Code, and other MCP clients
- project-aware rescan, latest metrics, scan comparisons, and recommendations
- compact AI summaries written beside local reports

No source code, report JSON, or graph artifacts are uploaded by these commands
unless a future explicit sync setting is enabled.

## Two Supported Setup Flows

GridSeak ships two ways to wire the CLI + MCP into your editor. Both
produce the same `~/.cursor/mcp.json` shape; pick whichever matches
how you installed the binary.

### A. Standalone CLI (shadow-mode users)

If you installed GridSeak via the standalone installer (Stage 10:
`scripts/install/install.sh` or `install.ps1`), the binary lives at
`~/.gridseak/bin/gridseak` (or `%LOCALAPPDATA%\GridSeak\bin\gridseak.exe`).
Wire it into Cursor in one command:

```bash
gridseak setup-cursor --global         # writes ~/.cursor/mcp.json
# or
gridseak setup-cursor --workspace      # writes <repo>/.cursor/mcp.json
```

Add `--command </abs/path/to/gridseak>` if the binary is not on
`PATH`. `--dry-run` previews the edit without writing.

The command is **idempotent**: it preserves any other MCP server
entries you have configured.

### B. Desktop-Managed Setup (desktop pilot users)

The desktop app owns the recommended setup flow. Open:

```text
GridSeak Desktop -> AI Editor
```

Then click **Install CLI**. The app verifies the bundled CLI, installs a
user-owned command, shows whether the install directory is on `PATH`, and
renders the Cursor/VS Code MCP config.

Platform behavior:

- macOS/Linux: writes a `gridseak` launcher to `~/.local/bin`. The launcher
  executes the bundled CLI inside `GridSeak.app`, so sidecars and parser configs
  stay version-locked with the desktop app.
- Windows: writes a self-contained CLI bin folder to
  `%LOCALAPPDATA%\GridSeak\bin`, including `gridseak.exe`, engine sidecars, and
  parser configs.

If the app says the install directory is not on `PATH`, add the displayed
command/path and restart Cursor or VS Code so the editor inherits the updated
environment.

## Basic CLI Smoke Test

```bash
gridseak scan .                                # first-run flow: scans and renders the hero report
gridseak status .
gridseak scans list .
gridseak scan latest .
gridseak compare                                # default: latest two scans
gridseak trends --metric score --window 30d
gridseak recommendations . --limit 10
gridseak findings . --severity critical
gridseak explain <finding_id>
gridseak metrics --category graph --category composite
gridseak graph top-fan-in --limit 10
gridseak graph blast-radius "<symbol>" --depth 2
gridseak context . --budget 4000 --for-llm
gridseak report --full                          # raw HealthReport JSON dump
```

Project references accept a project id, exact display name, partial
display name, root folder path, or `.` for the current working
directory.

### Legacy `scan <subcommand>` aliases (kept for one release)

```bash
gridseak scan rescan .
gridseak scan recommendations . --limit 10
gridseak scan findings . --severity high
gridseak scan report .
gridseak scan artifacts .
gridseak export ai-summary . --format markdown
```

`gridseak scan .` upserts a project record on first run and replays
the previous scan's language set on subsequent runs. If the project
has no previous scan, it auto-detects parseable languages from the
root folder. Power users can still override with `--lang rust` or
`--languages rust,typescript`.

## Cursor MCP Config

Use stdio transport:

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

If `gridseak` is not on `PATH`, use the absolute path:

```json
{
  "mcpServers": {
    "gridseak": {
      "command": "/Applications/GridSeak.app/Contents/MacOS/gridseak",
      "args": ["mcp"]
    }
  }
}
```

## Useful Agent Prompts

```text
Before changing this repo, ask GridSeak for the latest project health, scan
history, and top recommendations. Use those findings to prioritize the work.
```

```text
After your changes, run a GridSeak rescan for this project and compare the
latest scan against the previous one. Tell me whether health improved or
regressed and why.
```

## Agent-Native Flow

For compact context, start with:

```bash
gridseak status .
gridseak scan recommendations . --limit 10
gridseak scan findings . --limit 20
```

For deeper context, request raw artifacts only when needed:

```bash
gridseak scan recommendations . --deep
gridseak scan report . --full
gridseak scan artifacts .
```

## Local Store Override

For testing only, point the CLI/MCP server at a separate data directory:

```bash
gridseak --data-dir /tmp/gridseak-test projects list
GRIDSEAK_DATA_DIR=/tmp/gridseak-test gridseak mcp
```
