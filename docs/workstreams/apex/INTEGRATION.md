# Apex / Salesforce Integration

Authoritative reference for how the gridseak graphengine parses and analyses
Salesforce Apex codebases. Explains the architectural decisions, what is
implemented today, what is deliberately out of scope for Phase 1, and the
Phase 2 roadmap.

Audience: engineers adding features, support answering customer questions,
and anyone evaluating accuracy claims.

---

## 1. Why Apex is a first-class language

Salesforce implementations are among the most structurally-damaged
codebases that exist — not because Salesforce developers are worse, but
because the platform produces architecturally hazardous patterns by
default: declarative automation, multiple triggers per SObject,
managed-package black boxes, metadata XML dependencies, and "Flow
spaghetti" that no IDE surfaces. These are exactly the problems
gridseak exists to diagnose, and treating Apex as a generic C-like
language misses every one of them.

Every architectural choice below is deliberate: extract the
relationships a senior Salesforce architect would care about, not just
the generic call graph.

---

## 2. Accuracy posture: LSP-first, heuristic as safety net

Every other supported language goes through Tree-sitter + an LSP where
available. Apex is held to a stricter bar: the LSP is the **primary**
resolver and the heuristic is a narrow fallback, not the default.

**Reason**: Apex's type system is rich (virtual/abstract classes,
sharing modifiers, inheritance, inner classes, generics, SObject field
types) and single-file heuristics miss most of it. The official
`apex-jorje-lsp.jar` is the same engine that powers the Salesforce
Extension Pack in VS Code and knows every subtlety of the language. If
we ship heuristic-first, we'd be knowingly shipping worse accuracy
than the free VS Code extension — which is untenable for a diagnostic
product.

### Tier dispatcher

[`ApexResolverDispatcher`](../../../graphengine-parsing/src/syntax/language/apex/resolver_dispatch.rs)
probes the primary LSP at startup and selects between three modes:

| Mode | Trigger | Behaviour |
|---|---|---|
| **Auto** (default) | LSP available → primary | Full run via LSP, heuristic gap-fills any edges LSP didn't produce for files it errored on |
| **Auto** | LSP unavailable | Falls back to heuristic for the whole run, warning emitted |
| **LSP-only** | `GRAPHENGINE_APEX_RESOLVER=lsp` | Hard-fails if LSP is unavailable. For "I refuse to ship degraded output" scenarios |
| **Heuristic-only** | `GRAPHENGINE_APEX_RESOLVER=heuristic` | Skips LSP entirely. For CI reproducibility and support audits |

Gap-filled edges retain their original `Provenance::Heuristic`; LSP
edges keep `Provenance::Lsp`. Primary always wins on endpoint
collisions — heuristic can never downgrade a real semantic edge.

### LSP command resolution

Apex LSP isn't a standalone binary — it's a Java JAR. The
[`command_locator`](../../../graphengine-parsing/src/infrastructure/lsp/command_locator.rs)
module resolves the command at runtime in this order:

1. `GRAPHENGINE_JAVA_HOME` + `GRAPHENGINE_APEX_JORJE_JAR` env vars (CI
   / power-user override).
2. Bundled JRE + bundled `apex-jorje-lsp.jar` (desktop installer
   deployment — see `docs/05-deployment/DEPLOYMENT_ARCHITECTURE.md`).
3. System `java` on `PATH` + `apex-jorje-lsp.jar` on
   `APEX_LSP_JORJE_PATH`.

The `apex.yaml` config uses the placeholder token
`APEX_JORJE_JAR_PLACEHOLDER` in its `lsp_args`, which `command_locator`
rewrites at launch.

---

## 3. Repository layout discovery

Salesforce repos come in three flavours:

| Layout | Detection signal |
|---|---|
| **SFDX** (modern) | `sfdx-project.json` at root or any parent |
| **force-app heuristic** | `force-app/` or `force-app/main/default/` directory tree |
| **MDAPI** (classic) | Top-level `classes/`, `triggers/`, `objects/` directories |
| **File-extension scan** (fallback) | Neither of the above — scan for `.cls` / `.trigger` files everywhere |

See
[`sfdx_layout`](../../../graphengine-parsing/src/syntax/language/apex/sfdx_layout.rs).
The detector returns a `SfdxLayout` carrying both the classified files
and a single-line summary that ships in the telemetry block (so users
can sanity-check which layout we thought they had).

---

## 4. Metadata XML integration

Salesforce metadata lives in XML sidecar files, not the Apex source:

- `*.cls-meta.xml` — per-class API version, status, label
- `*.trigger-meta.xml` — per-trigger API version + status (active/inactive)
- `*.object-meta.xml` — per-SObject settings, sharing model

These are parsed streaming via
[`quick-xml`](https://docs.rs/quick-xml) in
[`metadata_reader`](../../../graphengine-parsing/src/syntax/language/apex/metadata_reader.rs)
to keep memory flat on repos with thousands of SObjects. Strict
end-tag checking is enabled so malformed XML fails loudly rather than
silently producing wrong attribution.

**What we extract today:**
- API version and deprecation status (fuels "is this class deprecated?"
  signals downstream).
- SObject API name + custom/standard flag (derived from the filename,
  since that's the canonical source — the file *name* `Account__c.object-meta.xml`
  is the SObject name, and the XML body just configures it).
- Class label / display name.

**Not yet parsed** (Phase 2 candidate): `<fields>`, `<validationRules>`,
`<listViews>`. These are high-value for schema-aware coupling but not
required for the Phase 1 meeting-ready metrics.

---

## 5. Apex type registry

[`ApexClassRegistry`](../../../graphengine-parsing/src/syntax/language/apex/class_registry.rs)
is a case-insensitive type registry preloaded with:

- ~80 standard SObjects (`Account`, `Contact`, `Opportunity`, …).
- Core system types (`System`, `Database`, `Test`, `Limits`, `Schema`,
  `Trigger`, etc.).
- User-declared classes, interfaces, enums, triggers — registered as
  they're discovered.
- Custom SObjects from `object-meta.xml` parsing.
- Managed-package external references detected via `namespace__ApiName`
  patterns (tracked but marked as external — never confused with local
  types).

Case-insensitive because **Apex itself is case-insensitive** and
pretending otherwise produces silent false-negatives during type
resolution. Inner-class fallback is implemented: a lookup for
`OuterClass.InnerClass` succeeds even if only `OuterClass` is
registered (pending richer inner-class indexing).

---

## 6. SOQL / SOSL edge extraction

`[SELECT Id FROM Account]` and `[FIND 'x' IN ALL FIELDS RETURNING
Contact(Id)]` are parsed as part of the **same tree** as the
containing Apex class — the vendored `tree-sitter-sfapex` grammar
embeds `soql_query_body` and `sosl_query_body` as children of
`query_expression`, so we get the SOQL/SOSL AST "for free".

[`soql_sosl`](../../../graphengine-parsing/src/syntax/language/apex/soql_sosl.rs)
walks the tree and emits one `QueryReference` per query literal:

- **SObject** the query targets (or the child-relationship name for
  subqueries).
- **Fields** explicitly listed (including aliased, function-wrapped,
  and relationship-dotted fields like `Account.Name`).
- `uses_fields_expansion` boolean for `FIELDS(ALL|CUSTOM|STANDARD)` —
  these imply "every field" and can't be enumerated from source.
- Subqueries are marked `is_child_relationship: true` — critical
  distinction, because SOQL subquery `FROM` names the *relationship*
  (`Contacts`) not the SObject (`Contact`).

**Deliberately not supported** (would produce false data):

- `Database.query('SELECT Id FROM ' + objName)` dynamic queries. The
  extractor silently skips them rather than emit partial refs.
- Runtime-computed field expansions inside `FIELDS()`.

---

## 7. Trigger-framework detection

Naive "≥ 2 triggers on SObject X" is a false-positive generator on
mature Salesforce repos, because the industry-standard pattern is
"one trigger that delegates to N handler classes via a framework". See
[`trigger_framework`](../../../graphengine-parsing/src/syntax/language/apex/trigger_framework.rs).

We detect the following frameworks via structural signals:

| Framework | Signal |
|---|---|
| **Kevin O'Hara's `sfdc-trigger-framework`** | `ITrigger` interface + `TriggerHandler` class |
| **`fflib-apex-common`** | `fflib_SObjectDomain` / `fflib_SObjectSelector` / `fflib_SObjectUnitOfWork` class presence |
| **NPSP TDTM** | `TDTM_Runnable` / `TDTM_Config` / `TDTM_TriggerHandler` class presence |
| **Generic** | Handler interface + trigger bodies that delegate rather than contain imperative logic |

When a framework is detected, the
[`MultipleTriggersPerSObject`](../../../graphengine-analysis/src/health/multiple_triggers_per_sobject.rs)
finding downgrades severity from Warning → Info and rewrites the
recommendation to "verify each trigger uses the handler dispatch
pattern" rather than "consolidate".

**Intentionally conservative**: detection requires strong evidence.
False positives on the finding are a minor annoyance; false negatives
on framework detection would silently hide real architectural
problems.

---

## 8. `MultipleTriggersPerSObject` finding

Severity ladder:

| Condition | Severity |
|---|---|
| 2–3 triggers on one SObject, no framework | Warning |
| ≥ 4 triggers on one SObject, no framework | High |
| Any count, framework detected | Info |

Finding id is stable across runs: `mtps-<sobject-lowercase>`. This
lets users permanently suppress the finding for known-intentional
cases via the triage override system.

---

## 9. Language config & grammar vendoring

- `graphengine-parsing/configs/apex.yaml` defines file extensions
  (`.cls`, `.trigger`, `.apxc`), LSP invocation, Tree-sitter queries,
  and kind mappings. It extends `LanguageConfig` with
  `lsp_initialization_options` so Apex can pass
  `enableSemanticErrors: true` without breaking every other
  language's LSP startup path.
- The Apex / SOQL / SOSL grammars are **vendored** at
  `graphengine-parsing/vendor/tree-sitter-sfapex/` rather than pulled
  from crates.io. Reason: the upstream published crate
  (`tree-sitter-sfapex` v2.4.0) depends on `tree-sitter 0.22+`, while
  the rest of this workspace is pinned to `tree-sitter 0.20` to keep
  all eight language grammars on a single version. The vendored crate
  compiles the upstream C sources against `tree-sitter 0.20` — the
  parsers are ABI-14 and self-contained, so this compiles cleanly.

  Upgrade path: bumping the whole workspace to `tree-sitter 0.22+` in
  a future PR should let us delete the vendored crate and take the
  public one. Tracked in `docs/00-strategy/FUTURE_PLAN.md`.

---

## 10. What is intentionally NOT in Phase 1

Listed explicitly so the scope is reviewable:

- **No Salesforce authentication.** No OAuth, no Tooling API, no
  network calls. Source-only analysis from a repo on disk.
- **No Flow / Process Builder / Validation Rule parsing.** These are
  metadata, not source; they require the Tooling API for useful
  analysis (their XML is mostly GUID references).
- **No managed-package content analysis.** We detect
  `namespace__Api` references and emit them as external nodes, but we
  do not unpack or analyse the managed package itself.
- **No deployment graph.** Change sets, package manifests, dependency
  order — all Phase 2.
- **No runtime profiling.** Governor limit analysis, trigger execution
  cost estimates, CPU-time budgets — future work.

---

## 11. Phase 2 roadmap

Post-pilot, org-connected upgrade. Positioned as a premium tier —
source analysis remains the foundation; org connection enriches it.

### 11.1 Org connection

- OAuth 2.0 via Salesforce Connected App. Credentials stored in
  the desktop OS keychain, never on disk in plaintext.
- `sfdx auth` token reuse when available so users don't re-auth.

### 11.2 Tooling API enrichment

- `MetadataComponentDependency` query to pull cross-component
  references that are invisible in source (e.g., a Flow that invokes
  an Apex class by FQN — the source just has
  `@InvocableMethod`, not the call site).
- `ApexCodeCoverage` + `ApexCodeCoverageAggregate` to layer real
  production test coverage on the graph — lets us flag "untested hot
  path" cases deterministically instead of guessing from source.
- `SetupAuditTrail` correlation for "most-churned configuration" —
  the Salesforce equivalent of temporal coupling.

### 11.3 Schema-aware coupling

- Parse `<fields>` / `<relationships>` sections of `object-meta.xml`
  to build a true schema graph.
- Overlay SOQL field edges on the schema graph — detect "querying a
  deprecated field" and "high churn on a heavily-referenced field".

### 11.4 Deployment & release

- Consume `package.xml` manifests to compute deploy-set impact
  ("this change will force re-deploy of N unrelated components").
- Change-set drift detection against the last deployed sha.

### 11.5 Flow graph

- Parse Flow XML (one of the ugliest Salesforce metadata formats —
  dataflow is a DAG of GUID-identified elements) to emit
  Flow → Apex → SObject edges.
- Requires a dedicated Flow parser; deferred because Flow XML is
  large and the parse isn't trivially vendorable as a Tree-sitter
  grammar.

### 11.6 Governor-limit projection

- Static analysis of SOQL query counts, DML counts, and callout
  counts per transaction entry point. Flag cases likely to hit
  `System.LimitException` at scale.

### 11.7 Richer test detection

- Wire `apex_test_detector.rs` into `symbol_extractor` and
  `trait_context_detector` so `@IsTest` classes and `testMethod`s are
  automatically excluded from production metrics.
- Requires integration with the shared extractor pipeline which
  spans all languages.

### 11.8 Entry-point coverage

- Full annotation-based entry-point detection: `@AuraEnabled`,
  `@InvocableMethod`, `@HttpGet/Post/Put/Delete/Patch`, `@RestResource`,
  `global`/`webservice` methods, `@Future`, `Schedulable.execute`,
  `Batchable.execute`, `Queueable.execute`.
- Required to stop the existing dead-code detector from flagging
  public LWC-callable methods as unreachable.

---

## 12. Environment variables reference

| Variable | Purpose |
|---|---|
| `GRAPHENGINE_JAVA_HOME` | Path to JRE for Apex LSP |
| `GRAPHENGINE_APEX_JORJE_JAR` | Path to `apex-jorje-lsp.jar` |
| `APEX_LSP_JORJE_PATH` | Alternate jar path (system PATH fallback) |
| `GRAPHENGINE_APEX_RESOLVER` | `auto` \| `lsp` \| `heuristic` — see §2 |

---

## 13. Verification environment (Sprint A.1)

The Apex LSP path is verified end-to-end by `tests/apex_lsp_smoke.rs`
(gated on the `lsp-tests` cargo feature). The harness needs a Java 17
JRE and the Salesforce `apex-jorje-lsp.jar`. Both are pinned to
specific upstream releases so the verification result is reproducible
across machines.

### 13.1 Pinned versions

| Component | Version | Source | SHA-256 |
|---|---|---|---|
| Eclipse Temurin JRE (macOS aarch64) | `17.0.18+8` | [adoptium/temurin17-binaries](https://github.com/adoptium/temurin17-binaries/releases/tag/jdk-17.0.18%2B8), asset `OpenJDK17U-jre_aarch64_mac_hotspot_17.0.18_8.tar.gz` | `6853987fa37340b157d7e8e895db0148efa13c3b2d6e6f3b289aac42e437d32e` |
| `apex-jorje-lsp.jar` | bundled in `salesforcedx-vscode-apex-66.5.4.vsix` | [forcedotcom/salesforcedx-vscode v66.5.4](https://github.com/forcedotcom/salesforcedx-vscode/releases/tag/v66.5.4), path `extension/dist/apex-jorje-lsp.jar` | `8742a3d5254228c72cfc630a11b06447d4c4209e9ad33a7da803ebceea822882` |

The Salesforce VS Code extension is the canonical distribution channel
for the jar; Salesforce does not publish it as a standalone artefact.
Bumping the pin means downloading the next release's `.vsix`,
extracting the jar, recording the new SHA-256 here, and re-running
`apex_lsp_smoke`.

### 13.2 Recommended on-disk layout

```text
~/.gridseak/
├── runtime/
│   ├── jdk-17.0.18+8-jre/             # tarball extraction
│   ├── jre  ->  jdk-17.0.18+8-jre/Contents/Home
│   ├── jre.tar.gz                     # original archive (keep for re-verify)
│   └── jre.sha256.txt                 # upstream sha256 file
└── lsp/
    ├── apex-jorje-lsp.jar
    └── apex-jorje-lsp.jar.sha256
```

This layout matches the bundled-desktop install model in
`docs/05-deployment/DEPLOYMENT_ARCHITECTURE.md`, so the same paths work for both
local verification and the eventual installer.

### 13.3 Provisioning recipe (macOS arm64)

```bash
mkdir -p ~/.gridseak/runtime ~/.gridseak/lsp

# Temurin 17 JRE
curl -sL -o ~/.gridseak/runtime/jre.tar.gz \
  "https://github.com/adoptium/temurin17-binaries/releases/download/jdk-17.0.18%2B8/OpenJDK17U-jre_aarch64_mac_hotspot_17.0.18_8.tar.gz"
shasum -a 256 -c <(echo "6853987fa37340b157d7e8e895db0148efa13c3b2d6e6f3b289aac42e437d32e  $HOME/.gridseak/runtime/jre.tar.gz")
tar -xzf ~/.gridseak/runtime/jre.tar.gz -C ~/.gridseak/runtime
ln -sfn ~/.gridseak/runtime/jdk-17.0.18+8-jre/Contents/Home ~/.gridseak/runtime/jre

# apex-jorje-lsp.jar (extract from the official VS Code extension VSIX)
curl -sL -o /tmp/sfdx-apex.vsix \
  "https://github.com/forcedotcom/salesforcedx-vscode/releases/download/v66.5.4/salesforcedx-vscode-apex-66.5.4.vsix"
unzip -p /tmp/sfdx-apex.vsix extension/dist/apex-jorje-lsp.jar > ~/.gridseak/lsp/apex-jorje-lsp.jar
shasum -a 256 -c <(echo "8742a3d5254228c72cfc630a11b06447d4c4209e9ad33a7da803ebceea822882  $HOME/.gridseak/lsp/apex-jorje-lsp.jar")
rm /tmp/sfdx-apex.vsix
```

For Linux x64, swap the Temurin asset name to
`OpenJDK17U-jre_x64_linux_hotspot_17.0.18_8.tar.gz` and use the
matching SHA-256 from the same Adoptium release page. CI runners follow
the same recipe in their setup step.

### 13.4 Activation

```bash
export GRAPHENGINE_JAVA_HOME=$HOME/.gridseak/runtime/jre
export GRAPHENGINE_APEX_JORJE_JAR=$HOME/.gridseak/lsp/apex-jorje-lsp.jar
```

Sanity check:

```bash
"$GRAPHENGINE_JAVA_HOME/bin/java" -version
# openjdk version "17.0.18" 2026-01-20
# OpenJDK Runtime Environment Temurin-17.0.18+8 (build 17.0.18+8)
test -f "$GRAPHENGINE_APEX_JORJE_JAR" && echo "jar present"
```

After activation, `cargo test -p graphengine-parsing --features
lsp-tests --test apex_lsp_smoke -- --nocapture` should run the live
handshake instead of skipping.

---

## 14. Telemetry and quality surfacing

Every Apex parse run emits:

- Resolver tier actually used (`lsp_primary` / `heuristic` / `merged`).
- Detected SFDX layout kind.
- Trigger-framework detection result (if any).
- LSP recall percentage when available (Phase 1: structural only;
  Phase 2 will compare against Tooling API ground truth).

This data is surfaced in the health report's resolution-quality block
so users can see *what* we produced, not just *what we claim*.

---

## 15. Validation status (post-Sprint H, 2026-04-16)

Phase 1 structural analysis is validated against four public SFDX
corpora plus a CI-pinned Volunteers subset:

| Corpus | Files | Nodes | Edges | `module == file` | Result |
|---|---|---|---|---|---|
| `trailheadapps/dreamhouse-lwc` | 9 | 56 | 105 | 9/9 | ✅ baseline (unchanged by H — no cross-class short-name collisions) |
| `trailheadapps/apex-recipes` | 142 | 1231 | 6321 | 142/142 | ✅ baseline (small Low→Medium migration from H.1) |
| `SFDO-Community/Volunteers-for-Salesforce` | 79 | 759 | ↓ from 4486 (H.1 collapsed cross-class `getNamespace` fanout) | 79/79 | ✅ baseline |
| `SalesforceFoundation/NPSP` | 1071 | 16 356 | ↓ from 288 034 (H.1 collapsed many `Low` cross-class calls to `Medium` same-class) | 1071/1071 | ✅ baseline; `npe01` / `npe03` / `npe04` / `npe05` / `npo02` carry H.2 registry metadata |
| `tests/fixtures/volunteers-mini/` (H.3 CI pin) | 4 (3 `.cls` + 1 `.trigger`) | — | — | 4/4 | ✅ runs on every PR via `cargo test --workspace` (0.56 s) |

All four external baselines are committed under `tests/fixtures/apex_baseline/`
and are the regression gate; `tests/fixtures/apex_baseline/inspections/{volunteers,npsp}/`
carries the G.2 per-class manual-comparison output for six specific
classes. The `volunteers-mini` fixture adds a per-PR regression test
that runs without Java and pins the structural invariants that the
full Volunteers baseline also covers.

### What Sprint G closed

- **In-memory node deduplication** in `GraphBuilder::add_nodes` —
  `graph.nodes` was keeping every duplicate from the managed-package
  Module synthesizer, even though SQLite was upserting them by id.
  NPSP module count dropped 2752 → 1075 after the fix; Volunteers is
  byte-identical (no managed packages). Regression-pinned by
  `pipeline::graph_building::tests::add_nodes_dedupes_by_stable_id`
  and `add_containment_skips_ids_already_on_graph`.
- **Apex inspection helper** (`apex_inspect` binary) — thin wrapper
  that re-parses a repo once and dumps matched-node / outgoing /
  incoming edge JSON per `--match` substring, so manual predictions
  have a cheap, reproducible comparison surface.
- **Capability confidence scorecard** in
  `docs/workstreams/apex/VALIDATION_RESULTS.md` — one row per capability,
  confidence level, evidence.

### What Sprint H closed

- **H.1 Same-class preference in `ApexHeuristicResolver`.** When
  multiple candidates share a short name and one or more live in the
  caller's class, the heuristic collapses to that same-class subset
  at `Medium` confidence and suppresses the cross-class fanout.
  `Low`-confidence fanout still fires when no same-class candidate
  exists.
- **H.2 Curated managed-package registry** in
  `graphengine-parsing/src/syntax/language/apex/managed_package_registry.rs`.
  ~25 well-known Salesforce ecosystem namespaces (NPSP, SBQQ,
  Vlocity/Industries, Pardot, Docusign, etc.) resolve to
  `display_name`, `vendor`, `category`, and `is_known_ecosystem_package=true`.
  Unknown namespaces are explicitly tagged
  `is_known_ecosystem_package=false` so consumers can distinguish
  "unknown" from "field missing".
- **H.3 Volunteers-mini CI fixture** + `apex_volunteers_corpus_e2e.rs`
  (5 tests) pin the structural invariants that Volunteers-for-Salesforce
  exercises — without needing to clone the full repo on every PR.
- **H.4 Nightly LSP validation workflow** (`.github/workflows/apex-lsp.yml`)
  runs `apex_baseline --enable-lsp` on `dreamhouse-lwc` under a real
  Java + jorje install every night, fails on any of:
  (a) JAR download or checksum mismatch,
  (b) `resolve_lsp_command` returning an error (Java missing / JAR
  path wrong),
  (c) `lsp_edges == 0` post-scan (session started but nothing
  resolved),
  (d) recall below pinned thresholds in
  `tests/fixtures/apex_baseline/lsp_thresholds.json`.
  Release builds are blocked until the last nightly on `main` is
  green (via the `apex-lsp-gate` job in `release.yml`). Operational
  runbook: `docs/workstreams/apex/LSP_CI_RUNBOOK.md`.

### Single outstanding validation gate

**LSP recall thresholds still need to be tuned from real runs.** The
nightly workflow's code path is complete and release-gated, but the
thresholds in `tests/fixtures/apex_baseline/lsp_thresholds.json` are intentionally
set to zero for the first run so the initial success doesn't block on
a number that was never observed. First nightly on `main` sets the
numeric ceiling; the runbook's "new_threshold = actual * 0.9" procedure
is then how drift in jorje recall is caught going forward.
