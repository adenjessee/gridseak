# 03 — DISTRIBUTION UNBLOCKERS: installable, findable, credible

Status: DRAFT v1 (2026-07-01)
Depends on: nothing. Run in parallel with 01/02. Exit = Gate G2.
Executor profile: agent with shell + CI/YAML competence; some tasks need
the owner's accounts (flagged OWNER below).

## Context

GridSeak's adoption channel is (a) agents choosing MCP servers from
registries by machine-readable quality signals and (b) developers reading
third-party comparison articles. Both are blocked by mechanical issues,
verified 2026-07-01:

- Release workflow (`.github/workflows/release.yml`) builds Linux x86_64
  and macOS x86_64 ONLY — no Apple Silicon (aarch64) binary, excluded
  over a `ring`/tree-sitter build issue. In 2026 most target developers
  are on ARM Macs. This alone disqualifies adoption.
- Core crates are `publish = false`; no Homebrew tap, no npm wrapper.
- Not listed on the MCP official registry, Glama, mcp.so, or any
  awesome-mcp list. Directory quality scores penalize missing LICENSE /
  SECURITY.md / CI / imprecise tool descriptions.
- Public web footprint describes GridSeak as a bootcamp visualization
  platform — contradicting the agent-first engine. Brand confusion kills
  the first impression the few visitors get.
- Install exists (`scripts/install/install.sh` → GitHub release
  manifest → `~/.gridseak/bin`) and `gridseak setup` auto-wires Cursor /
  Claude Code / Codex / Windsurf — these are good; build on them.

## Objective

A stranger on an Apple Silicon Mac (or Linux/Windows) goes from zero to
an MCP-connected scan in under 5 minutes, and both machines and reviewers
can find and score the project without anyone knowing the founder.

## Workstream A — Apple Silicon + release matrix

1. Reproduce the aarch64-apple-darwin build failure:
   `cargo +1.91 build -p gridseak-cli --release --target aarch64-apple-darwin`
   (on the local ARM Mac this is a native build; capture exact errors).
2. Known suspects: the `ring` crate (older versions lack aarch64-darwin
   prebuilt asm — fix by upgrading the dependency that pulls `ring`, or
   replacing rustls-with-ring with `rustls` + `aws-lc-rs` or native-tls)
   and tree-sitter grammar C compilation flags (fix via `cc` crate env:
   ensure `CFLAGS`/`--target` propagate; vendored `tree-sitter-sfapex`
   may need its build.rs updated).
3. Add `aarch64-apple-darwin` (and evaluate `aarch64-unknown-linux-gnu`)
   to the release matrix; update `cli-manifest.json` generation so
   `install.sh` picks the right artifact by `uname -m`.
4. Acceptance: fresh ARM Mac (no dev tools beyond curl) completes
   `curl … | bash && gridseak scan .` successfully.

## Workstream B — Registry and directory presence

Preparation (repo hygiene — these are literal scoring inputs):
1. LICENSE at root (verify), SECURITY.md, CONTRIBUTING.md, CODE_OF_CONDUCT.md.
2. A public CI workflow that runs fmt+clippy+test on PRs (the current
   pre-push-hook-only model reads as "no CI" to scorers). Keep it cheap:
   one Linux job, cache cargo.
3. Tool descriptions audit: every one of the 14 MCP tools in
   `gridseak-cli/src/main.rs` must state (a) exactly when to call it,
   (b) input constraints, (c) output shape summary. They are already
   good; tighten to directory-scoring standards (explicit trigger
   phrases, no vague verbs).
4. `server.json` / MCP manifest for the official MCP registry format.

Submissions (OWNER accounts needed, agent drafts everything):
5. MCP official registry; Glama (claim server, fix score deductions);
   mcp.so; Smithery.
6. PRs to `punkpeye/awesome-mcp-servers` and 2–3 other relevant lists —
   one server per PR, follow each list's contribution guide exactly.
7. GitHub topics on the repo: `mcp`, `model-context-protocol`,
   `ai-agents`, `code-analysis`, `static-analysis`, `call-graph`,
   `tree-sitter`, `developer-tools`.

## Workstream C — Package managers

1. Homebrew tap (`gridseak/homebrew-tap`) with a formula pulling release
   artifacts; test on ARM+Intel Mac.
2. npm wrapper package `gridseak` (postinstall downloads the right
   binary — same pattern as esbuild/biome). This matters because MCP
   users habitually `npx` servers.
3. Defer crates.io (workspace has path deps and vendored grammars;
   publishing is real work with low adoption payoff vs A–C).

## Workstream D — Brand separation and the front door

1. Decide canonical public home: recommend `github.com/gridseak/gridseak`
   as THE public repo (audit found a public-mirror overlay already
   exists in `scripts/release/public-overlay/`). One canonical README.
2. README rewrite (agent drafts): first screen = what it is in one
   sentence ("deterministic structural ground truth for AI agents —
   every edge carries evidence and calibrated confidence"), a 30-second
   install block, a real terminal GIF, the tier model in one diagram,
   an honest language-support table copied from LIMITATIONS.md (kill the
   parity implication), link to benchmark (plan 04) when live.
3. Separate the personal-brand/bootcamp story from the engine: the
   engine's site/README never mentions the bootcamp; the personal site
   links out to the engine, not the reverse.
4. OWNER: a plain `gridseak.dev` (or similar) landing page can wait
   until plan 05's observatory needs a home; do not block G2 on it.

## Workstream E — 5-minute first-run experience

1. `gridseak setup` already wires agent clients; add a final verification
   step that performs one MCP round-trip and prints "your agent can now
   ask: what breaks if I change X".
2. First-scan UX: progress output, then the report card summary (plan 06
   will make it beautiful; here it must at least be correct and honest —
   including the plan-01 disclosure lines).
3. Time the full journey on a clean machine; record it in
   `docs/workstreams/master-plan/evidence/03-first-run-timing.md`.

## Acceptance criteria (binding — this is Gate G2)

- [ ] ARM Mac one-command install → successful MCP-connected scan < 5 min.
- [ ] Release workflow produces aarch64-apple-darwin artifacts; manifest
      serves them; install.sh selects correctly.
- [ ] Listed live on >= 3 of: MCP official registry, Glama, mcp.so,
      Smithery, awesome-mcp-servers.
- [ ] Public repo has LICENSE, SECURITY.md, CONTRIBUTING.md, PR CI.
- [ ] README opens with the engine story; language table matches
      LIMITATIONS.md; no bootcamp branding.

## Sequencing note

Do Workstream A first (it gates everything), then B-prep, then C/E in
parallel, then B-submissions last so every directory reviewer who clicks
through sees the finished front door.
