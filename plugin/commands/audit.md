---
description: Grade your coding agent on this repo against compiler truth (gridseak audit). Shows the plan and cost first.
argument-hint: "[extra gridseak audit flags, e.g. --tasks 3 --arms plain]"
---

Audit my coding agent on this repository with `gridseak audit`, following
the `audit-my-agent` skill from the GridSeak plugin step by step:

1. Check `gridseak --version` and `claude --version`. If `gridseak` is
   missing, show `curl -fsSL https://gridseak.com/install.sh | sh` and stop
   until I approve.
2. Run `gridseak scan .` if there is no scan, or if the audit says the call
   graph is not fully compiler-indexed (then `gridseak doctor --provision
   <lang>` and rescan).
3. Run `gridseak audit --plan-only $ARGUMENTS` and show me the planned tasks.
4. Estimate the cost: tasks x arms runs at about $0.25-3.50 each (6 tasks x 2
   arms is typically $8-25; the hard cap is runs x `--max-usd-per-task`,
   default $4). Ask me to confirm. Do not run the full audit until I say yes.
5. Run `gridseak audit $ARGUMENTS` in the background (it can take over an
   hour) and poll it.
6. Open `report.html` and summarize `report.md` per arm: exact answers,
   silent misses, minutes, cost, tokens. Explain that a silent miss is an
   incomplete caller list the agent presented as complete.
