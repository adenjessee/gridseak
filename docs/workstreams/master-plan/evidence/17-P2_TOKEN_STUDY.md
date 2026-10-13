# 17 — P2: do code maps help a coding agent? (result)

Date: 2026-10-12. Design and pre-registered hypotheses:
`../P2_TOKEN_STUDY_DESIGN.md` (committed before the v2 run). Harness, task
file, compiler truths and every raw run: `tests/p2-token-study/`.

## Setup

- Claude Code headless (`claude -p`, `--restricted`, read-only tools plus
  `go`), fresh session per run, same prompt per task.
- Arms: **A** plain; **B** + Graphify 0.9.86 graph and CLI; **C** + GridSeak
  MCP (callers, slice, verify-claim); **D** = C told to use GridSeak first.
- Corpora: chi, zod, caddy v2.11.7 (112k LOC Go), hono v4.13.13 (66k LOC TS).
- 14 scored tasks (one more excluded, below): caller lists for methods whose
  name is defined up to 102 times; file lists for interface/class methods
  with 6–8 same-name definitions; 7 deletion questions whose truth is the
  compiler; 1 "which tests run X" task whose truth is per-test coverage.
- 2 runs per task per arm. Model Opus 5.5 (all arms) and Haiku 5.5 (A, D).

## Result

| Arm (Opus 5.5) | Fully correct | Mean cost / question | Mean wall time | Used its map |
|---|---:|---:|---:|---:|
| A plain | 27/28 (96%) | $0.085 | 18 s | — |
| B + Graphify | 28/28 (100%) | $0.076 | 16 s | 0/28 |
| C + GridSeak | 28/28 (100%) | $0.086 | 17 s | 7/28 |
| D + GridSeak, told to use it | 27/28 (96%) | $0.086 | 18 s | 21/28 |

| Arm (Haiku 5.5) | Fully correct | Mean cost / question | Used its map |
|---|---:|---:|---:|
| A plain | 27/28 (96%) | $0.172 | — |
| D + GridSeak, told to use it | 28/28 (100%) | $0.212 | 28/28 |

Total spend: pilot $1.33, Opus v2 $9.93, Haiku v2 $10.77.

**Plain reading:** on repos up to 112k lines, a frontier agent answers
structural questions — including names defined 102 times, interface
contracts and compiler-level "can I delete this?" — correctly with grep and
file reads, for about 8 cents. Neither map changed accuracy or cost. When
not told to, the agent ignored Graphify entirely and used GridSeak in 7 of
28 runs. "71.5x fewer tokens" was not reproduced: savings were ~0.

Hypotheses: H1 (maps raise recall at equal or lower cost) **not supported** —
no recall headroom existed. H2 (deletion bounded by interface contracts)
**not supported for agents** — they reasoned about contracts correctly; it
holds for GridSeak's own "zero callers" claim (20/30 caddy methods break the
build). H3 (unprompted agents rarely use the map) **supported**. H4
(Graphify gives no accuracy gain) **supported**.

The two misses: agents listed files whose only match was a doc comment
(`c.render('Hello!')`) — a grep failure GridSeak-arm runs avoided once.
Haiku 5.5 cost more per question than Opus 5.5 here (more turns and tokens;
even a one-word reply cost $0.149 vs $0.096), so "small model + map" did not
save money either.

## Truth corrections made after the run (applied to all arms equally)

- `hono-files-Router.match`: the oracle listed `matcher.ts`, which only
  assigns `this.match = match`; every agent correctly left it out. Removed.
- `hono-delete-dom.css.cx` excluded: deleting it type-checks (the `cx,`
  shorthand rebinds to the module-level export) but crashes at load
  (temporal dead zone). The question as asked was ill-posed.

## Limits

One model family; two runs per cell; repos up to 112k LOC (no
million-line monorepo); single-question tasks, not multi-step edits or long
sessions where context fills; headless, not IDE.
