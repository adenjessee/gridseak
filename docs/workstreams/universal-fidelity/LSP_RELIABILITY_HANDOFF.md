# LSP Reliability Handoff

## Purpose

This document gives a developer full context for the current LSP reliability investigation, what was already proven, what was fixed on the current branch, and what remains to fully solve the problem.

The product goal is not "use LSP when it is easy." The goal is for GridSeak to use language servers whenever they are the right source of truth, across small and large repositories, and to be honest when it cannot reach authoritative resolution. GridSeak should be able to take more time, resume partial work, and report why resolution degraded instead of silently falling back to weaker heuristics.

## Current Branch

Work was done in:

```text
lsp-patient-mode-probe
```

This branch was created from the current local `main` branch in `/Users/me/gridseak-graphengine`.

Important working-tree note: before this work began, unrelated changes were already present:

```text
A  docs/04-architecture/GRIDSEAK_SELF_SCAN_REFACTOR_MAP.md
A  docs/04-architecture/GRIDSEAK_SELF_SCAN_REFACTOR_MAP.png
?? graphengine-parsing/tests/doc_linkage_c4_metadata.rs
```

Those files were not part of this LSP investigation and were intentionally left alone.

## How We Got Here

The observatory running on the remote `gridrunner` host had scanned many repositories, but only a very small set reached LSP-level fidelity (`authoritative` or `heuristic_primary`). The user wanted to know whether this was because:

1. The `gridrunner` machine was too weak.
2. LSP packages were missing or misconfigured.
3. GridSeak itself was falling back too quickly or incorrectly.
4. GridSeak lacked the patient, resumable, large-repo LSP strategy needed for the product vision.

The initial remote work verified that LSP packages were installed on `gridrunner`, but high-fidelity results were still rare. The remote machine was then inspected and found to be extremely constrained:

```text
CPU: 1 core, Intel Pentium 4 2.66GHz
RAM: 1.2GiB total, about 82MiB free at inspection time
Swap: 2.0GiB, already in use
Architecture: i686 / 32-bit
```

That host is useful for runner robustness and small smoke tests, but it is not a suitable proof machine for large-repo LSP reliability.

## Observatory Data Export And Stats

The remote SQLite database was about `3.1G`, and broad aggregate queries were too slow on the `gridrunner` host. Instead, a narrow run-level export was produced locally:

```text
/Users/me/gridrunner-observatory/analysis_exports/observatory_analysis_export_20260616.tar.gz
```

Contents:

```text
observatory_run_stats.csv
observatory_stats_summary.json
observatory_stats_summary.md
lsp_level_repos_clean.md
lsp_diagnosis.md
report_inventory.csv
watchtower_schema.sql
```

Dataset summary:

```text
895 total runs
192 unique repos
778 ok
115 error
2 running at export time
```

Run-level fidelity:

```text
750 syntactic_only
18 authoritative
11 heuristic_primary
6 unknown
110 null/no report
```

Best-per-repo fidelity:

```text
114 syntactic_only
3 authoritative
4 heuristic_primary
1 unknown
70 null/no report
```

Statistically significant observations after deduping by repo:

```text
LSP-level scans had much higher high-confidence call ratio:
  median 0.585 vs 0.00055
  p ~= 9.43e-06

LSP-level scans reported far fewer edges:
  median 346 vs 7033
  p ~= 0.0008

LSP-level scans reported far fewer functions:
  median 151 vs 1474.5
  p ~= 0.000784

LSP-level scans had shorter analysis duration:
  median 801ms vs 3400ms
  p ~= 0.00433
```

Repo size by bytes/files/lines was not statistically significant after deduplication. The better interpretation is:

> LSP succeeds when the analyzed language slice is small enough and properly configured, not strictly when the whole Git checkout is tiny.

## Source Review Findings

Relevant source locations:

```text
graphengine-parsing/src/infrastructure/lsp/resolver.rs
graphengine-parsing/src/infrastructure/lsp/simple_client.rs
graphengine-parsing/src/infrastructure/lsp/session.rs
graphengine-parsing/src/infrastructure/lsp/definition_provider.rs
graphengine-parsing/src/infrastructure/lsp/utils/document_sync.rs
graphengine-parsing/src/infrastructure/lsp/resolvers/call_resolver_lsp.rs
graphengine-parsing/src/infrastructure/lsp/resolvers/import_resolver.rs
graphengine-parsing/src/infrastructure/lsp/resolvers/type_resolver.rs
graphengine-parsing/configs/python.yaml
graphengine-parsing/configs/typescript.yaml
graphengine-parsing/configs/java.yaml
graphengine-parsing/configs/go.yaml
```

What GridSeak already had:

```text
LspResolver runs call/import/type phases with tokio::join!.
Each LSP phase processes references/imports/types in chunks.
SimpleLspClient supports concurrent in-flight requests through request IDs and a semaphore.
Apex has a better readiness model through ProgressAndProbe.
There is a fallback path from LSP to heuristic resolution.
```

What was weak:

```text
Non-Apex languages use Immediate readiness after initialize.
The old default document indexing wait was only 5 seconds.
Python and Go request timeouts were only 1 second.
TypeScript request timeout was 2 seconds.
Java request timeout was 5 seconds.
Chunk size was fixed at 32.
The system favored fast fallback over patient authoritative resolution.
```

One important nuance: the local proof runs showed timing is not the only problem. Some failures were caused by configuration and server validation bugs before timing even mattered.

## Fixes Already Made

### 1. Python LSP command fixed

File:

```text
graphengine-parsing/configs/python.yaml
```

Before:

```yaml
lsp_command: "pyright"
lsp_args:
  - "--stdio"
```

After:

```yaml
lsp_command: "pyright-langserver"
lsp_args:
  - "--stdio"
```

Why this matters:

`pyright` is the type-checking CLI. It is not the LSP server and does not accept `--stdio` as an LSP transport. The correct LSP executable from the Pyright package is `pyright-langserver --stdio`.

This directly explains Python LSP unreliability. On local runs, the old config launched `pyright --stdio`, which exited and caused GridSeak to fall back.

### 2. LSP availability check fixed

File:

```text
graphengine-parsing/src/infrastructure/lsp/simple_client.rs
```

Old behavior:

```text
GridSeak checked availability by running:
  <server> --version
or:
  <server> --help

If those exited non-zero, the server was declared unavailable.
```

Why this was wrong:

Some valid LSP servers do not behave like normal CLI tools. `pyright-langserver`, for example, exits non-zero unless launched with a transport option such as `--stdio`. This caused GridSeak to reject a valid installed LSP server before starting it correctly.

New behavior:

```text
If command is a path, check that it exists.
If command is a bare executable name, check that `which` can resolve it.
Do not require --help or --version to succeed.
```

This is a higher-signal availability test for language servers.

### 3. Patient LSP runtime knobs added

Files:

```text
graphengine-parsing/src/infrastructure/lsp/simple_client.rs
graphengine-parsing/src/infrastructure/lsp/utils/document_sync.rs
graphengine-parsing/src/infrastructure/lsp/resolvers/call_resolver_lsp.rs
graphengine-parsing/src/infrastructure/lsp/resolvers/import_resolver.rs
graphengine-parsing/src/infrastructure/lsp/resolvers/type_resolver.rs
```

New environment knobs:

```text
GRIDSEAK_LSP_PROFILE=patient
GRAPHENGINE_LSP_REQUEST_TIMEOUT_MS
GRAPHENGINE_LSP_MAX_CONCURRENT_REQUESTS
GRAPHENGINE_LSP_INDEX_WAIT_MS
GRAPHENGINE_LSP_DOCUMENT_SETTLE_MS
GRAPHENGINE_LSP_CHUNK_SIZE
```

Language-specific timeout/concurrency overrides also work:

```text
GRAPHENGINE_LSP_PYTHON_REQUEST_TIMEOUT_MS
GRAPHENGINE_LSP_PYTHON_MAX_CONCURRENT_REQUESTS
GRAPHENGINE_LSP_TYPESCRIPT_REQUEST_TIMEOUT_MS
GRAPHENGINE_LSP_TYPESCRIPT_MAX_CONCURRENT_REQUESTS
...
```

`GRIDSEAK_LSP_PROFILE=patient` defaults:

```text
request timeout: 30_000ms
max concurrent requests: 4
index wait: 30_000ms
document settle: 5_000ms
chunk size: 4
```

Default behavior is unchanged when these env vars are not set.

## Validation Completed

Commands that passed:

```bash
cargo +1.91 check --manifest-path /Users/me/gridseak-graphengine/Cargo.toml -p graphengine-parsing
cargo +1.91 build --manifest-path /Users/me/gridseak-graphengine/Cargo.toml -p gridseak-cli
cargo +1.91 build --manifest-path /Users/me/gridseak-graphengine/Cargo.toml -p graphengine-parsing
git -C /Users/me/gridseak-graphengine diff --check
```

IDE lints reported no errors for touched files.

Important compile note:

The repo is pinned to Rust `1.91` in `rust-toolchain.toml`. The default active shell had Rust `1.90`, so the correct command uses:

```bash
cargo +1.91 ...
```

## Local Probe Corpus

Local probe repos were cloned into:

```text
/Users/me/gridrunner-observatory/lsp_probe_repos
```

Repos:

```text
autosize
libevent
build-extra
```

Probe outputs were written under:

```text
/Users/me/gridrunner-observatory/lsp_probe_runs
```

The local machine had:

```text
typescript-language-server
pyright
pyright-langserver
```

No local `jdtls`, `gopls`, or equivalent Java/Go server was available during this pass.

## Proof Run Results

### Autosize

Command shape:

```bash
gridseak scan <autosize> --languages javascript --format json
```

Results:

```text
baseline tier: heuristic_primary
patient tier: heuristic_primary
baseline high_ratio_on_calls: 0.7777777777777778
patient high_ratio_on_calls: 0.7777777777777778
baseline call confidence: high=7, low=2
patient call confidence: high=7, low=2
```

Interpretation:

Patient mode did not change `autosize`. The remaining misses are likely extraction, location, or symbol-mapping issues rather than timeout.

### Libevent Python Slice

Old behavior before sidecar rebuild/fixes:

```text
pyright --stdio exited.
GridSeak fell back to heuristics.
```

After fixing Python config and rebuilding the `graphengine-parsing` sidecar:

```text
tier: authoritative
high_ratio_on_calls: 0.9539473684210527
call confidence: high=145, medium=2, low=5
summary edges/functions: 347 / 151
```

Compared to the earlier local run:

```text
previous call confidence: high=144, medium=2, low=5
fixed call confidence: high=145, medium=2, low=5
```

Interpretation:

The Python LSP path is now actually using the correct language server and no longer crashes/rejects the server. This fixed a concrete reliability issue. Patient mode did not add more edges on this small slice, so further improvements likely require better extraction/mapping/definition handling.

### Build-extra JavaScript Slice

Result:

```text
tier: authoritative
high_ratio_on_calls: 1.0
call confidence: high=5, medium=0, low=0
summary edges/functions: 14 / 3
```

Interpretation:

This repo's JavaScript slice is tiny locally: only one JS source file was discovered. It is useful as a smoke test, not a large-repo stress test.

## Important Finding: Why It Looked Like GridSeak "Ignored" LSP

The answer is mixed:

```text
On gridrunner, the machine is underpowered enough that LSP can time out or crash under load.
But GridSeak also had real code/config problems that caused valid LSP usage to fail or be rejected.
```

Concrete examples:

```text
Python config launched the wrong binary.
The availability check rejected valid LSP servers that do not support --help/--version.
The sidecar binary had to be rebuilt; rebuilding only gridseak-cli was not enough.
```

This means the problem was not just hardware. Hardware made the behavior worse, but GridSeak itself needed fixes.

## What Is Not Done

The branch does not fully solve large-repo authoritative LSP.

Remaining gaps:

```text
No Java/JDTLS proof run was completed locally.
No Go/gopls proof run was completed locally.
No large TypeScript repo stress test was completed locally.
Patient mode is currently an env-driven probe, not a first-class CLI/API policy.
Non-Apex readiness is still mostly Immediate at session startup.
Reports still lack enough LSP telemetry to explain exactly why each reference missed.
The remaining low-confidence edges on small slices likely need extractor/location/symbol-index fixes.
The observatory should be rerun on better hardware after these fixes.
```

## Recommended Full Plan

### Phase 1: Land the small fixes

Goal: remove known false-negative LSP startup failures.

Tasks:

```text
1. Add/adjust tests for Python config using `pyright-langserver`.
2. Add a unit test for LSP availability check:
   - valid executable path returns true
   - valid bare command found by PATH returns true
   - no dependency on --help or --version success
3. Build graphengine-parsing and gridseak-cli with Rust 1.91.
4. Run autosize/libevent/build-extra smoke tests.
5. Commit only scoped LSP files, not unrelated staged docs.
```

### Phase 2: Add LSP telemetry to reports

Goal: make every fallback explainable.

Add report fields for:

```text
server command
server command source
server start attempts
successful starts
failed starts
last LSP error
notifications received
stderr lines observed
indexing messages seen
request successes
request timeouts
definition hits
definition nulls
definition errors
fallback count by reason
per-phase LSP duration
per-phase heuristic fallback duration
```

Important: distinguish these cases:

```text
server missing
server rejected by availability check
server crashed
request timed out
server returned null
definition returned a location but GridSeak could not map it to a symbol
extractor did not produce a usable call-site location
heuristic fallback produced an edge
```

Without this telemetry, we cannot reliably tell "LSP was not needed" from "LSP failed silently."

### Phase 3: Build a repeatable local LSP benchmark corpus

Goal: compare before/after behavior without depending on the remote observatory.

Use repos from the observatory and add expected language slices:

```text
autosize: JavaScript, small
libevent: Python slice, small/medium
build-extra: JavaScript slice, small analyzed graph despite larger checkout
FFmpeg: Python slice, larger checkout
joern: Java slice, requires JDTLS
a Go repo from the collected dataset, requires gopls
a TypeScript repo with many files, requires typescript-language-server
```

For each run, capture:

```text
scan command
GridSeak commit
sidecar binary timestamp
LSP server versions
machine specs
duration
fidelity tier
high_ratio_on_calls
call_edges_by_confidence
all_edges_by_confidence
LSP telemetry
fallback reasons
```

### Phase 4: Make patient mode first-class

Goal: replace env-only probing with a clear product policy.

Possible CLI/API:

```bash
gridseak scan . --lsp-policy fast
gridseak scan . --lsp-policy patient
gridseak scan . --lsp-policy exhaustive
```

Policy meaning:

```text
fast:
  current behavior, bounded and quick

patient:
  wait for language-server readiness
  lower concurrency
  longer request timeout
  explicit report telemetry

exhaustive:
  maximum practical LSP effort
  resumable work queue
  no silent fallback without recording why
```

### Phase 5: Generalize readiness beyond Apex

Goal: prevent early nulls caused by querying before indexing completes.

Current state:

```text
Apex has ProgressAndProbe readiness.
Most other languages still use Immediate readiness.
```

Needed:

```text
TypeScript: documentSymbol or workspaceSymbol canary after initialize.
Python/Pyright: documentSymbol canary and/or diagnostics/indexing quiet period.
Java/JDTLS: workspace/project import readiness; likely longer initialization budget.
Go/gopls: module load readiness and controlled concurrency.
Rust: separate ra_ap_ide path already exists, but subprocess fallback still needs telemetry.
```

### Phase 6: Fix extraction and mapping misses

Goal: resolve the cases where LSP returns useful data but GridSeak cannot use it.

Likely areas:

```text
call-site range accuracy
UTF-16 column conversion
definition range to symbol lookup
module/import capture gaps
dynamic/member-call receiver extraction
Python generated-code/string false positives
JavaScript config warnings for missing module/type queries
```

The local runs already showed warnings like:

```text
Query 'structs' for language 'javascript' does not contain capture groups (@)
Query 'modules' for language 'javascript' does not contain capture groups (@)
Query 'type_refs' for language 'javascript' does not contain capture groups (@)
No kind mapping found for common node type 'function_item'
No kind mapping found for common node type 'struct_item'
No kind mapping found for common node type 'mod_item'
```

Some warnings may be harmless, but they should be reviewed because missing import/module structure directly affects coupling, dead-code, and cross-file confidence.

### Phase 7: Rerun observatory on proper hardware

Goal: validate the product claim under realistic conditions.

Do not use the current `gridrunner` host as the proof machine for large repos.

Recommended minimum:

```text
64-bit OS
multi-core CPU
8-16GiB RAM minimum
enough disk for repo cache + DB
installed LSP servers for all target languages
```

Then rerun the same repo list and compare:

```text
before/after fidelity tier
before/after high_ratio_on_calls
fallback reasons
LSP crash/timeout rate
scan duration
per-language success/failure
```

## Highest-Leverage Next Developer Tasks

1. Add tests for the two concrete fixes:

```text
python.yaml uses pyright-langserver
LSP availability is based on command resolution, not --help/--version
```

2. Add LSP telemetry to the full report.

3. Convert env-only patient mode into a real scan policy.

4. Run local corpus with Java/Go servers installed.

5. Investigate remaining low-confidence edges in `autosize` and `libevent` using definition trace logs.

6. Only after those are measured, move to larger repos and the remote observatory.

## Bottom Line

This pass did more than troubleshoot. It found and fixed two concrete GridSeak-side LSP reliability bugs:

```text
Python LSP used the wrong executable.
LSP availability probing rejected valid language servers.
```

It also added runtime controls for patient LSP experiments.

However, the full vision is not complete. Large-repo authoritative LSP requires telemetry, first-class LSP policy, readiness per language, resumable work, and language-specific extraction/mapping fixes. The next planning session should start from these concrete fixes and build the full reliability plan around measurable fallback reasons rather than guesses.

