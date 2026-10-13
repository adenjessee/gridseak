---
name: audit-my-agent
description: Grade the user's coding agent on this repository against compiler truth with `gridseak audit`, then summarize the report card. Use when the user asks to "audit my agent", "grade my agent on this repo", "can I trust my agent here", or wants to measure how often their agent misses callers. Spends real money on agent runs, so it always shows the plan and cost and asks before running.
---

# Audit my agent on this repo

`gridseak audit` asks a headless Claude Code agent "which functions
transitively call X?" for a handful of targets in this repository, grades each
answer against the compiler-indexed call graph, and writes a shareable report
card. It runs each question twice by default: the agent alone (`plain`) and the
agent with GridSeak's MCP tools (`gridseak`).

It spends real money. Never start the full audit without the user's explicit
yes in this conversation.

## Steps

Run every command from the repository root.

1. **Check the binary.** Run `gridseak --version`. If it is not found, show
   the user this and stop until they approve or install it themselves:

   ```sh
   curl -fsSL https://gridseak.com/install.sh | sh
   export PATH="$HOME/.gridseak/bin:$PATH"
   ```

   Also check `claude --version`: the audit drives Claude Code headless
   (`claude -p`), so it must be installed and logged in.

2. **Scan if needed.** If the repository has no scan yet, or the user changed
   a lot of code since, run `gridseak scan .`. The audit refuses to run unless
   every call edge in the scan comes from the compiler index. If it says
   "only N of M call edges ... come from the compiler index", tell the user,
   run `gridseak doctor --provision <lang>`, rescan, and retry.

3. **Plan first (free).** Run:

   ```sh
   gridseak audit --plan-only
   ```

   It prints one line per task (band, target function, number of callers,
   hops) and writes `tasks.json`. No agent runs, nothing is spent.

4. **Show the plan and cost, then ask.** Show the task list and this estimate:

   - Runs = tasks x arms (default 6 x 2 = 12).
   - Each run costs about $0.25-3.50 at API prices; 6 tasks x 2 arms is
     typically **$8-25**.
   - Hard cap: runs x `--max-usd-per-task` (default $4, so $48 for 12 runs).
   - Time: runs are sequential, up to `--timeout-min` (default 30) each;
     expect tens of minutes to over an hour.
   - Costs go to the account Claude Code is logged in with.

   Offer cheaper options: `--tasks 3`, `--arms plain`, a lower
   `--max-usd-per-task`, or `--model <id>`. Then ask: "Run the audit for
   about $X-Y?" Wait for a clear yes. If the user says no, stop.

5. **Run.** With the flags the user agreed to, for example:

   ```sh
   gridseak audit --tasks 6 --arms plain,gridseak
   ```

   It takes longer than a normal tool-call timeout. Run it in the
   background and poll its output, or ask the user to run it in their own
   terminal. Each finished run prints `PASS`, `MISS (silent)`,
   `MISS (flagged)` or `ERROR` with recall, precision, minutes and cost.
   Results are saved after every run under `.gridseak/audit/<timestamp>/`.

6. **Report.** The last line prints the path to `report.html`. Open it for
   the user (`open` on macOS, `xdg-open` on Linux) and read `report.md` in
   the same directory to summarize. Give, per arm:

   - **Exact answers**: runs with recall >= 0.95 and precision >= 0.9, out
     of the total.
   - **Silent misses**: wrong answers presented as complete.
   - **Minutes**, **cost** (USD at API prices) and **tokens**.

   Lead with the headline, e.g. "Alone, your agent was exact on 2 of 6.
   With GridSeak, 6 of 6. It missed silently 3 times." Tell the user
   `report.html` is a self-contained card they can share, and that
   `gridseak audit --report <dir>` re-renders it from `results.json`.

## What a silent miss is

The agent returned an incomplete or wrong caller list and did not say it
might be incomplete. That is the dangerous failure: a refactor built on it
breaks callers nobody checked. A **flagged** miss is also wrong, but the
agent warned the list could be incomplete.

## Languages

Go is supported today (the scan is compiler-indexed via `scip-go`).
TypeScript works when the scan is fully compiler-indexed (`scip-typescript`
indexed every call edge). Other languages are refused until their scan is
100% compiler-indexed.

## Do not

- Do not run `gridseak audit` without `--plan-only` before the user agrees
  to the cost.
- Do not edit repository files; the audit only writes under `.gridseak/`.
- Do not report numbers you did not read from the output or `report.md`.
