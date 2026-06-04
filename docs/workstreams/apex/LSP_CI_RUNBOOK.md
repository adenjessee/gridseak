# Apex LSP CI Runbook

Operational guide for the `apex-lsp` GitHub Actions workflow
(`.github/workflows/apex-lsp.yml`). This workflow validates that the
apex-jorje LSP stack continues to produce structural results on real
Salesforce code, so silent regressions in LSP integration cannot ship.

## What this workflow guards

- **LSP stack health** — java + apex-jorje-lsp.jar can be provisioned,
  the LSP smoke test passes against a real jar.
- **LSP recall** — on every nightly run, the dreamhouse-lwc corpus is
  scanned with `apex_baseline --enable-lsp`. The resulting
  `resolution_stats.lsp_edges` must meet or exceed the pinned minimum
  in `tests/fixtures/apex_baseline/lsp_thresholds.json`.
- **Heuristic-fallback masking** — `--enable-lsp` fails hard if the
  LSP stack isn't present or produces zero LSP edges. A silent
  heuristic-only run cannot be mistaken for a successful LSP run.

## Not guarded here

- Apex parsing of new grammar constructs — covered by per-PR
  `cargo test` in `ci.yml`.
- Structural invariants (module count, sharing, trigger synthesis) —
  covered by `tests/apex_volunteers_corpus_e2e.rs` on every PR.
- NPSP / Volunteers full-corpus recall — deliberately not scanned
  here. They take 60–100 s each, would quadruple nightly runtime, and
  would churn on legitimate upstream updates. Spot-check manually.

## First-run procedure

The workflow ships with `lsp_thresholds.json` set to zero so the first
run cannot fail. Use that first successful run to set the real pins.

1. **Land the workflow** with the default zero thresholds. Confirm
   `ci.yml` and `release.yml` pass.
2. **Trigger the workflow manually** from the Actions tab
   (`Apex LSP Validation` → `Run workflow` on `main`). Wait for it to
   succeed.
3. **Download the artifact** `dreamhouse-lwc-lsp-baseline-<run>`
   attached to the run. Open `dreamhouse-lwc.lsp.json`.
4. **Read `resolution_stats.lsp_edges`** (call it `E`) and the
   fraction `E / (E + resolution_stats.heuristic_edges)` (call it
   `F`).
5. **Pin thresholds** at 90% of observed. Edit
   `tests/fixtures/apex_baseline/lsp_thresholds.json`:

   ```json
   {
     "default": { "min_lsp_edges": 0, "min_lsp_fraction": 0.0 },
     "corpora": {
       "dreamhouse-lwc": {
         "min_lsp_edges": <floor(E * 0.9)>,
         "min_lsp_fraction": <F * 0.9 rounded to 4 decimals>
       }
     }
   }
   ```

6. **Open a PR**, merge once `ci.yml` passes. The next nightly enforces
   the pin.

## Updating the jar pin

The apex-jorje jar is pinned via `JORJE_VERSION` (currently `62.14.1`
since 2026-04-18; the prior pin `62.17.0` was pulled from the upstream
GitHub releases page so we re-baselined to the last downloadable v62
release to keep the measurement stable). The download script
(`scripts/download_apex_jorje.sh`) fetches the
`salesforcedx-vscode-apex-<version>.vsix`, extracts the jar, and caches
it. Note: the internal jar path inside the VSIX moved from
`extension/out/apex-jorje-lsp.jar` (pre-63) to
`extension/dist/apex-jorje-lsp.jar` (63+); the script now tries both.

To roll the pin:

1. Pick a new release tag from
   [forcedotcom/salesforcedx-vscode/releases](https://github.com/forcedotcom/salesforcedx-vscode/releases).
2. Update `JORJE_VERSION` in `apex-lsp.yml`.
3. Trigger the workflow manually. The jar cache key changes, so the
   jar will be re-downloaded.
4. If the new jar shifts `lsp_edges` materially, re-run the first-run
   procedure to re-pin the thresholds.
5. Record the new SHA-256 (printed by `download_apex_jorje.sh`) in
   `scripts/download_apex_jorje.sh` via the `JORJE_SHA256` env
   default, in the same PR. That locks the jar bytes so an upstream
   silent republish can't drift the recall measurement.

## Responding to a red nightly

1. **Re-run the workflow once.** Transient GitHub / network issues
   occasionally manifest as jar download failures.
2. **Read the `check_lsp_recall` step output.** It prints the observed
   `lsp_edges`, the configured minimum, the observed fraction, and the
   configured minimum fraction.
3. **Bisect recent commits on `main`** using the baseline JSON from
   the last green run. If a specific commit introduced the regression,
   revert or open a fix PR.
4. **If the drop is legitimate** (e.g. we intentionally tightened LSP
   filtering), update `lsp_thresholds.json` down in the same PR that
   introduces the change. Never bump thresholds silently.

## Integration with release

`.github/workflows/release.yml` gates on the most recent `main` run of
this workflow being green. See that file for the wiring.
