# Batch H — Distribution (held)

Date: 2026-09-16. Updated after G2 stranger-proof install (fresh HOME).

## What is true

- G1 (High Call / all Call) **is met** on the pinned scans:
  - chi `73abec07-12fd-47e9-a2e8-fb3c69fdf971` → 0.777
  - zod `4cb6e82a-e4d7-458f-aecb-09c3f8d87452` → 0.911
  Evidence: `10-B-results.md`, `11-AGENT_AB-results.md`.
- The shippable binary is `gridseak` (not `ge-mcp`). `release.yml`
  now copies `target/.../release/gridseak` as a **required** artifact
  for `aarch64-apple-darwin`, `x86_64-apple-darwin`, and linux triples.
- Install path: `scripts/install.sh` → `~/.gridseak/bin` + PATH prepend
  instructions. The older manifest installer
  (`scripts/install/install.sh`) remains for SHA-verified sidecar
  bundles.

## S1 local pack (this machine, 2026-09-16)

- `scripts/pack-local-tarball.sh local` produced
  `target/local-release/gridseak-local-aarch64-apple-darwin.tar.gz`
  with layout `gridseak-local-aarch64-apple-darwin/gridseak` (matches
  `release.yml`: `gridseak-<tag>-<triple>/gridseak`).
- `GRIDSEAK_TARBALL=… GRIDSEAK_HOME=<tmpdir> bash scripts/install.sh`
  extracted that archive and `gridseak --version` printed `gridseak 0.1.0`.
- Plugin hook-key tests pass: Cursor `preToolUse`
  (`Write|StrReplace|Delete|Shell`, `failClosed`) plus
  `beforeShellExecution`; Claude `PreToolUse` + `Edit|Write|MultiEdit`.
- **Clean-Mac `curl | sh` from a public GitHub release: unknown.**
  No `v*` tag was cut. Do not claim a clean-laptop install until
  someone runs `curl -fsSL …/scripts/install.sh | bash` against a
  real release.

## G2 fresh-HOME install + setup --verify (this machine, 2026-09-16)

Ran on a throwaway `HOME=/tmp/gs-g2-fresh-LQ2dVF` so it did not
overwrite the developer `~/.cursor`.

1. `scripts/pack-local-tarball.sh g2-fresh` wrote
   `target/local-release/gridseak-g2-fresh-aarch64-apple-darwin.tar.gz`.
2. `GRIDSEAK_TARBALL=… GRIDSEAK_HOME=$HOME/.gridseak bash scripts/install.sh`
   installed `gridseak 0.1.0` at
   `/tmp/gs-g2-fresh-LQ2dVF/.gridseak/bin/gridseak`.
3. That binary `setup --command <itself>` wrote absolute paths:
   - `~/.cursor/mcp.json` → `/private/tmp/gs-g2-fresh-LQ2dVF/.gridseak/bin/gridseak`
   - `~/.cursor/hooks.json` `preToolUse` matcher
     `Write|StrReplace|Delete|Shell` via
     `/bin/bash ~/.cursor/hooks/gridseak-gate.sh cursor-tool`
   - `beforeShellExecution` via the same wrapper (`cursor` format)
   - `~/.claude/settings.json` `PreToolUse` via the same wrapper
     (`claude` format). Cursor 3.20.21 also runs this layer; emptying
     only `hooks.json` is not a control. `setup --verify` now rejects
     an empty `PreToolUse: []` even if SessionStart `gate --card` remains.
4. `setup --verify` exited 0:
   - MCP command absolute
   - hook binary absolute and has `gate`
   - wrapper returns JSON for a `{"command":"pwd"}` payload
   - NOTE: this machine's login `PATH` still resolves
     `~/.cargo/bin/gridseak` (no `gate`). That is the documented
     footgun; verify passed because the *written* command is absolute.
5. `gridseak doctor` hook section: `status=ok`, `absolute=true`,
   `has_gate=true`, `wrapper_json=true`. Sidecars resolved from the
   developer workspace (not bundled in the CLI-only tarball). Doctor
   also reported `PATH: different_binary` for `~/.cargo/bin/gridseak`.

`setup --verify` unit tests (`setup_verify_g2`) fail a non-absolute
hook command, a gateless binary, and a wrapper that exits 1 with no
JSON.

**Clean-Mac `curl | sh` remains unknown.** This proof is a local
tarball + fresh HOME, not a second machine.

## What is held (do not start)

- 50-repo observatory — not stood up.
- Flagship video — not recorded. Do not film the policy A/B harness
  as a live bakeoff.
- Homebrew / npm / official marketplace — after P1 fixtures + P2
  live transcripts (or an explicit `ABANDON`), not before.
- Apex god-file splits, type→vector docs, video engine.

G3 *slice* ECE 0.08 in `10-C-results.md` still carries the
SCIP-as-oracle contamination disclosure. That is **not** a P4
calibration start.
