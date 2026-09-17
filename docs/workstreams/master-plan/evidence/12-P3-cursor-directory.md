# cursor.directory listing draft — NOT SUBMITTED

Do not treat this file as a live listing. OWNER pastes it after a
second-machine `curl | sh` smoke.

## Name

GridSeak

## One sentence

Deterministic structural ground truth for AI agents: every edit-boundary
verdict is deny / ask / allow with a named evidence tier.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/adenjessee/gridseak/main/scripts/install/install.sh | bash
export PATH="$HOME/.gridseak/bin:$PATH"
gridseak --version   # must print 0.1.1
gridseak gate --help # must exist; 0.1.0 does not have this
gridseak setup
gridseak setup --verify
```

Public release `cli-v0.1.1` is live. Do **not** use the private
`gridseak-graphengine` URL or `scripts/install.sh` (that path 404s).

## Hooks (three layers on Cursor 3.20.21)

1. `~/.cursor/hooks.json` — `preToolUse` (`Write|StrReplace|Delete|Shell`)
   plus `beforeShellExecution` for `rm` / `git rm`
2. plugin `plugin.json` / `.cursor-plugin/plugin.json`
3. `~/.claude/settings.json` `PreToolUse` (`gridseak-gate.sh claude`)

Cursor `preToolUse` does not enforce `ask` (deny-with-reason).

## MCP

`gridseak mcp --slim`: route, verify_claim, blast_radius, diff_impact,
gate_status.

## Pin

Plugin tree SHA: fill at submit time. Require `gridseak` on PATH from
`scripts/install.sh`.
