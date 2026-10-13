# Live Cursor A/B (`cursor-agent -p`)

Headless driver for the mutation subset of `tests/agent-ab/tasks.json`.
This is the live host path. `make agent-ab` remains fixture-only.

Requires `cursor-agent` logged in (`cursor-agent status`).

```
# smoke (go-01 both arms, n=1)
node tests/agent-ab/live/run.mjs --tasks go-01-safe-delete-newrouter --n 1 \
  --arms treatment,control --out target/live-ab-smoke

# mutation pack
node tests/agent-ab/live/run.mjs --n 1 --out target/live-ab
```

**Treatment** uses your login `HOME` plus `--plugin-dir plugin`.
**Control** copies only `cli-config.json` + `agent-cli-state.json` into a
temp `HOME` so user hooks, Claude `PreToolUse`, and the plugin do not load.

Each run mutates the pinned corpus in `.cache/g1/{chi,zod}` and
`git checkout`s the watched files afterwards. Runs are sequential.

Control also stubs `~/.cursor/hooks/gridseak-gate.sh`, removes the
GridSeak MCP entry, and parks `~/.cursor/skills/gridseak-gate`.
Auth stays on the login `HOME` (a temp HOME cannot see the keychain).

Headless models may refuse a harmful edit even with the gate off.
Smoke requires: treatment deny + no control ledger deny. Control
**harm** is scored separately and may stay 0.

Cursor 3.20.21 also honours `~/.claude/settings.json` `PreToolUse`.
That is a third hook layer, not a substitute for `~/.cursor/hooks.json`.
