# Discovery stub — Rust Layer 2 adapter per-variant error histogram

> **Source follow-up.** [UF-FU-012 item (c)](../FOLLOWUPS.md).
> **Status.** Stub — no per-variant counter wired yet.
> **Priority rationale.** Lowest of UF-FU-012's three — the 0.9 %
> adapter-error rate is small in absolute terms (896 errors out
> of 101 343 call refs on `gridseak-self`). But the rate is
> currently only aggregate-counted, which means *any* upstream
> `ra_ap_ide` regression that shifts error shapes would be
> invisible. This stub is primarily an observability improvement.

---

## 1. The measured shape

From the same 2026-04-21 Gate 1.2 dogfood run:

- `adapter_errors = 896` (0.9 % of `call_refs = 101 343`).
- `ResolveSnapshot` only carries the aggregate count; the per-variant
  `SemanticResolverError` breakdown is discarded.

The error enum is declared in
`graphengine-parsing/src/infrastructure/semantic/` (adapter crate
seam). Candidate variants — the exact list needs verification
when the PR lands:

- `FileNotInProjectModel { file_path: PathBuf }` — the file is not
  registered with `ra_ap_load_cargo::load_workspace_at`'s VFS.
  Known failure mode during crate-boundary transitions.
- `LineIndexMissing { file_id: FileId }` — the file is in the
  project model but its line index was not built (rust-analyzer
  internal).
- `VfsReadFailed { file_path: PathBuf, error: String }` — the
  VFS panicked trying to read the file (permissions, transient
  filesystem error).
- `GotoDefinitionPanic { recovered: bool }` — caught a panic inside
  the `ra_ap_ide::Analysis::goto_definition` call. Should be rare;
  any nonzero count is actionable.
- `RangeOutOfBounds { requested_range: Range, file_len: u64 }` —
  tree-sitter and rust-analyzer disagree on file length (encoding,
  CRLF, BOM).
- `Other(String)` — catch-all for enum variants we do not know
  about yet.

## 2. Why this matters

The 0.9 % rate is small today, but:

- Error composition is a leading indicator. A shift from 10 %
  `FileNotInProjectModel` to 80 % `FileNotInProjectModel` signals
  a VFS-registration regression that would otherwise look like
  a stable 0.9 % "noise floor".
- Upstream `ra_ap_ide` releases can change the error surface
  without changing the rate. Without per-variant counters, a
  silent semantic change in what the adapter reports as `Err`
  vs `Ok(None)` is invisible.
- A future mitigation PR (retry on `VfsReadFailed`, skip
  `GotoDefinitionPanic` + log) needs per-variant data to
  sequence its targets.

## 3. Proposed investigation

**Observability PR.** Land a diff that:

1. Adds a `HashMap<SemanticResolverErrorDiscriminant, u32>` field
   to `ResolveSnapshot` (already the right seam — same struct
   that carries the aggregate `adapter_errors` count).
2. Populates the map in the existing error-count code path.
   Existing aggregate count stays for backward compatibility.
3. Prints the histogram at the end of the `dump_report` example
   harness and at the end of the dogfood-rollup binary.
4. Adds an explicit `SemanticResolverErrorDiscriminant` enum
   whose `Display` impl produces stable string keys (these become
   JSON keys in downstream artefacts — breakage-sensitive).

**Zero behavioural change** in the adapter itself. Pure
observability.

## 4. Success criteria for the stub

- The 896-error total on `gridseak-self` decomposes into a
  histogram with ≥ 99 % of errors classified (≤ 1 % in `Other`).
- Each non-zero variant has a named disposition in the PR body:
  "keep, benign" / "watch, may need retry" / "needs fix".
- The histogram is re-runnable as part of any future dogfood —
  not a one-shot measurement.

## 5. Non-goals

- Not a fix for any error variant. Measurement-only PR.
- Not a new error-handling policy. If the histogram reveals a
  variant that should be retried rather than counted, that
  becomes its own ticket.
- Not cross-language. The `SemanticResolverError` enum is Rust
  Layer 2-specific; the generic seam is `Box<dyn Error>` at the
  resolver trait boundary and does not benefit from this breakdown.

## 6. Graduation path

Unlike the other two UF-FU-012 stubs, this one does not
necessarily produce a follow-on engineering ticket. If the
histogram shows a clean distribution — say 95 % `FileNotInProjectModel`
on crate-boundary files, 5 % benign — it may simply close with
the observation that the error rate is well-characterised and
stable. The observability itself is the deliverable.

If a variant reveals a fix opportunity (e.g. `VfsReadFailed`
with > 5 % share), graduate to `../tasks/T6.b-adapter-error-handling.md`
and update UF-FU-012 (c)'s `Recommended next step`.
