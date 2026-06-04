# Apex Framework Resolver Plan

> **Reproducing historical numbers / paths cited below.** Neither the historical baseline JSONs / calibration outputs nor the rev6.1 byte-identical regression fixture referenced in this document are tracked in git — both live as sha256-pinned GitHub release assets. Fetch on demand with `scripts/setup.sh historical-baselines` (rev3..rev11 evidence, [release](https://github.com/adenjessee/gridseak-graphengine/releases/tag/baseline-archive-2026-05-18)) and `scripts/setup.sh fixtures` (rev6.1 regression fixture, [release](https://github.com/adenjessee/gridseak-graphengine/releases/tag/regression-fixtures-2026-05-19)). All artifacts are pinned in `experiments/artifacts.lock`. The active build/test loop does not require any of them.

> **Handoff document.** Authored at the end of Wave 3.1 of the
> `truthful-scans-simplification` plan. The Apex Framework Resolver
> itself is **not built in this repo change**. This file enumerates
> the work, clusters it by dispatch idiom, fixes pre-registered
> acceptance gates per cluster, and records the north-star
> architecture (edge provenance) so later implementation does not
> drift.
>
> Scope boundary: this plan covers `graphengine-parsing/src/syntax/language/apex/`
> only. Python (Django URLconf, Celery) and JavaScript (LWC HTML
> templates, Aura `.cmp`) framework resolvers are out of scope — they
> are tracked in `FOLLOWUP_RISKS.md` R16 (Django), R25 (LWC) and
> R28 (Aura / Jest). The design principles (idiom clusters,
> authoritative vs. synthetic edges, provenance) apply verbatim to
> those languages.

## 1. Why this plan exists

Wave 2 of `truthful-scans-simplification` split the classifier by
framework so that per-node verdicts are honest. The Layer-5 audit
(`docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`) confirmed:

- **0 / 10 wrong** in `framework_annotation_unresolved` — the
  classifier *correctly* identifies 152 NPSP Apex methods as
  "dispatched by the Salesforce platform, not by an in-repo call
  edge."
- **10 / 10 wrong** in `no_callers` — every sampled FQN is a *real*
  method with a *real* caller in NPSP's source tree that the Apex
  heuristic resolver (`graphengine-parsing/src/syntax/language/apex/heuristic_resolver.rs`)
  failed to link.

Two orthogonal failure modes:

| failure mode | what is wrong | where the fix lives |
| ------------ | ------------- | ------------------- |
| *framework-dispatched but no edge* | the platform invokes the method at runtime; there is no Apex source edge that could ever exist. Today the classifier correctly *labels* this but cannot make the symbol "live." | emit a `FrameworkEntry` edge in the resolver so the dead-code detector never sees the symbol as dead. |
| *in-repo caller unlinked* | the caller exists in `.cls` / `.trigger` / `.page` / `.html` source and the parser failed to resolve the reference (constructor, typed-field, inner-class, implicit `toString()`, VF page binding). | extend the Apex resolver (this plan) and ancillary `.page` / `.html` parsers (out of scope). |

Both problems converge on the same architectural fix: every edge
carries its *source of authority* (AST-linked, framework-derived,
VF-page-parsed, etc.) and a *confidence*. See §5 — North star.

## 2. North star: edge provenance

Today the call-graph has one edge shape:

```text
Call { caller: NodeId, callee: NodeId, line: u32, ... }
```

Every edge is treated equally — the resolver either emits it or it
doesn't. The dead-code detector then makes a binary "has any inbound
edge" decision, and the classifier pastes a *reason* onto the
remaining blanks.

The destination architecture, recorded in `FOLLOWUP_RISKS.md` R24
and extended here:

```text
Edge {
    caller: Option<NodeId>,           // None when the platform is the caller
    callee: NodeId,
    source: EdgeSource,               // authoritative taxonomy, see below
    confidence: f32,                  // 0.0..=1.0
    evidence: String,                 // human-readable cite (file:line, rule id, framework tag)
}

enum EdgeSource {
    AstLinked,                        // resolved from an in-repo AST reference
    FrameworkEntry(FrameworkTag),     // platform calls this at runtime
    DeclarativeWiring(WiringKind),    // VF / LWC / Aura / Flow / Process Builder
    HeuristicName,                    // substring / naming-convention guess (last resort)
}
```

Consequences:

1. **`DeadCodeReason` becomes a label on a threshold, not a bucket.**
   "Dead" is "no incoming edge at confidence ≥ τ." Reason text is
   derived from the *highest-confidence source that didn't reach τ*.
   The whole `DeadCodeReason` enum (R27) becomes derived state.
2. **The classifier shrinks to a thresholder.** Framework-keyed
   rule sets disappear; the classifier reads the strongest inbound
   edge's `source` field to answer "why is this dead?" in a single
   step.
3. **Framework resolvers and the AST resolver compose.** Nothing
   distinguishes "parser found a call" from "framework says platform
   calls this" at the edge level; only the `source` + `confidence`
   fields differ.

This plan ships the first half of that picture: the Apex Framework
Resolver emits `FrameworkEntry(FrameworkTag)` edges at known
confidence levels. Edge-provenance migration (R24) and classifier
retirement (R27) remain deferred; this plan does not re-implement
the dead-code detector.

## 3. Dispatch matrix (R13 table, promoted)

Every dispatch idiom observed in NPSP, with resolver type, emitted
edge kind, and the on-disk evidence source the resolver reads. The
classifier reason shown is the *current* label the classifier
attaches before the resolver ships; once the resolver emits the
edge, the symbol is `live` and the reason disappears.

| # | idiom | Salesforce mechanism | resolver type | edge kind emitted | evidence source (on disk) | current classifier reason |
| - | ----- | -------------------- | ------------- | ----------------- | ------------------------- | ------------------------- |
| 1 | `Database.Batchable` | platform `Database.executeBatch(new Foo())` invokes `start` → `execute` → `finish` | **authoritative** | 3 × `FrameworkEntry(batchable)` edges (one per contract method) | `implements Database.Batchable[<T>]` + class AST | `framework_annotation_unresolved` |
| 2 | `Schedulable` | platform scheduler invokes `execute(SchedulableContext)` | authoritative | `FrameworkEntry(schedulable)` | `implements Schedulable` | `framework_annotation_unresolved` |
| 3 | `Queueable` | `System.enqueueJob(new Foo())` invokes `execute(QueueableContext)` | authoritative | `FrameworkEntry(queueable)` | `implements Queueable` | `framework_annotation_unresolved` |
| 4 | `Messaging.InboundEmailHandler` | Email services invoke `handleInboundEmail` | authoritative | `FrameworkEntry(inbound_email)` | `implements Messaging.InboundEmailHandler` | `framework_annotation_unresolved` |
| 5 | `InstallHandler` / `UninstallHandler` | install flow invokes `onInstall` / `onUninstall` | authoritative | `FrameworkEntry(install_handler)` | `implements InstallHandler` / `UninstallHandler` | `framework_annotation_unresolved` |
| 6 | `@AuraEnabled` | LWC / Aura JS calls `@wire`d Apex method | authoritative + declarative pair | `FrameworkEntry(aura_enabled)` from platform; future `DeclarativeWiring(Lwc)` from paired `.js` import when LWC resolver lands | `@AuraEnabled` annotation on method | `framework_annotation_unresolved` |
| 7 | `@InvocableMethod` | Flow / Process Builder invokes via metadata | authoritative | `FrameworkEntry(invocable)` | `@InvocableMethod` annotation | `framework_annotation_unresolved` |
| 8 | `@RestResource` + `@HttpGet/Post/Put/Delete/Patch` | Apex REST API dispatches HTTP method to tagged static | authoritative | `FrameworkEntry(rest_resource)` | `@RestResource(urlMapping=…)` on class + `@Http*` on method | `framework_annotation_unresolved` |
| 9 | `@RemoteAction` | VF JS Remoting calls the static | authoritative | `FrameworkEntry(remote_action)` | `@RemoteAction` annotation | `framework_annotation_unresolved` |
| 10 | `global` / `webservice` modifier | managed-package consumer or SOAP API calls method | authoritative (conservative — treat as entry point) | `FrameworkEntry(global_api)` | `global` / `webservice` modifier | `framework_annotation_unresolved` |
| 11 | `.trigger` file synthetic body | Salesforce runtime fires trigger events | authoritative | `FrameworkEntry(apex_trigger)` | file extension `.trigger` + `trigger … on … ( …events… )` header | `framework_annotation_unresolved` |
| 12 | TDTM handler registered in `Trigger_Handler__c` | `TDTM_DispatchConfig.getDispatchConfig()` reflects via `Type.forName(name).newInstance()` | **synthetic** (config-file evidence) | `FrameworkEntry(tdtm_handler, confidence=0.9)` per row in `TriggerHandlers.json` / custom-metadata export | scan `force-app/tdtm/**/*.json` or any file named `Trigger_Handler_*.sfdx.json` for class names | `dynamic_dispatch_target` |
| 13 | `TDTM_iTableDataGateway` / `TDTM_ObjectDataGateway` interface | typed-interface dispatch via `TDTM_TriggerHandler` field | **authoritative** (falls under AST resolver gap R23) | `Call(AstLinked)` | class declarations + typed field access in `.cls` | `dynamic_dispatch_target` |
| 14 | Visualforce page extension (`<apex:page controller="Foo">`) | VF renderer resolves controller getters / actions | **declarative** (future `.page` parser) | `DeclarativeWiring(Vf)` | pair `Foo.page` ↔ `Foo.cls`; action bindings `{!idPanel}` | `no_callers` (wrong; see R23 + this plan §4.8) |
| 15 | LWC HTML template binding (`on<event>={method}`, `{getter}`) | LWC renderer resolves JS method via `.html` attribute | declarative (future `.html` parser) | `DeclarativeWiring(Lwc)` | paired `.html` template in the LWC bundle | `declarative_wiring_unparsed` (already honest) |
| 16 | Aura `<aura:handler action="{!c.foo}">` | Aura framework resolves `Controller.foo` | declarative (future `.cmp` parser) | `DeclarativeWiring(Aura)` | paired `.cmp` / `.app` file | R28 (not yet categorised; currently `visibility_private_unused`) |
| 17 | Intra-class constructor call `new X(...)` | resolved at compile time | AST (resolver gap R23) | `Call(AstLinked)` | same-file class decl | `no_callers` |
| 18 | Cross-file constructor call | resolved at compile time | AST (resolver gap R23) | `Call(AstLinked)` | class registry (`graphengine-parsing/src/syntax/language/apex/class_registry.rs`) | `no_callers` |
| 19 | Typed-field dispatch `instanceField.method()` | resolved at compile time | AST (resolver gap R23) | `Call(AstLinked)` | field decl → type → method decl | `no_callers` |
| 20 | Inner-class constructor / method | resolved at compile time | AST (resolver gap R23) | `Call(AstLinked)` | outer-class containment graph | `no_callers` |
| 21 | Implicit `toString()` in string concatenation / `String.valueOf` | emitted by the Apex runtime | AST (resolver gap R23) | `Call(AstLinked, confidence=0.7)` | expression AST (implicit conversion site) | `no_callers` |
| 22 | Sibling overload dispatch in same class | resolved at compile time | AST (resolver gap R23) | `Call(AstLinked)` | class decl + overload set | `no_callers` |

Rows 1–11 are the "pure framework-dispatch" population the Apex
Framework Resolver removes from the `framework_annotation_unresolved`
bucket. Row 12 is the TDTM case already covered heuristically (R24)
and now upgraded to config-file-driven. Rows 13, 17–22 are resolver
gaps (R23) that must also ship for the `no_callers` hand-audit gate
to flip to PASS. Rows 14–16 are declarative-wiring resolvers owned
by separate (Python / JS) workstreams; they are enumerated here
because they share the same `EdgeSource::DeclarativeWiring` edge
kind and the acceptance gate in `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md` depends on them.

## 4. Clustered backlog with acceptance gates

Each cluster below is sized against the **rev 5 A/B-injected
analysis** (`experiments/results/NPSP/rev5/ab_report.json`). Counts
in this section are pre-registered: the resolver implementation for
cluster N is considered done when the listed bucket delta is
observed on the next canary run and the paired hand-audit resample
continues to show 0 / 10 wrong on `framework_annotation_unresolved`.

Cluster priority order reflects two criteria: (a) absolute count in
NPSP (ship the ones that move the biggest numbers first), (b)
implementation cost (authoritative rules are cheap — one AST visit
per class; declarative wiring needs a new parser per format).

### 4.1 Cluster `batchable_contract` — 72 FQNs (priority P0)

**What the runtime does.** Any class `implements Database.Batchable`
(or `Database.Batchable<T>`) is invoked by the platform as
`start(context) → execute(context, records) → finish(context)`. The
three contract methods are all entry points.

**Resolver work.** Extend `graphengine-parsing/src/syntax/language/apex/entry_points.rs::collect_interface_method_markers`
(already shipped for direct-implements) to emit `FrameworkEntry(batchable)`
Call edges from a synthetic platform-caller node into each contract
method. Current Wave 1 implementation only attaches an
`entry_point_tag`; no edge is emitted. This plan upgrades the tag
into an edge with `EdgeSource::FrameworkEntry`.

**Input precondition.** The class literally declares `implements
Database.Batchable` (with or without a type parameter). Abstract
bases whose subclass declares the `implements` are *explicitly out
of scope* for this cluster — covered by §4.13.

**Acceptance gate.** After this cluster ships:

- `framework_annotation_unresolved` cluster `batchable_contract`:
  72 → 0 (all three contract methods × 24 classes become `live`).
- `metrics.dead_code.count` delta: −72.
- Hand-audit resample (10 FQNs) on
  `framework_annotation_unresolved` keeps 0 / 10 wrong — i.e. the
  remaining items in that bucket must still all be correctly
  labelled.
- `edge_provenance_counts.FrameworkEntry_batchable`: ≥ 72 (there
  may be more — some classes implement Batchable inside test
  scope and are currently filtered).

### 4.2 Cluster `apex_trigger_body` — 26 FQNs (priority P0)

**What the runtime does.** A `.trigger` file's body runs on
insert / update / delete events. The parser already synthesises a
`__trigger__()` symbol; that symbol currently has no inbound edge.

**Resolver work.** In the `.trigger` parse path, emit a
`FrameworkEntry(apex_trigger)` edge to the synthetic `__trigger__()`
symbol, sourced from the trigger header's event list (e.g.
`before insert`, `after update`). The edge's evidence string cites
the event list verbatim — useful for downstream UIs that want to
display "fired on 4 events."

**Acceptance gate.**

- `framework_annotation_unresolved` cluster `apex_trigger_body`:
  26 → 0.
- `metrics.dead_code.count` delta: −26.

### 4.3 Cluster `schedulable_execute` — 26 FQNs (priority P0)

**What the runtime does.** `System.schedule(...)` invokes
`execute(SchedulableContext)` at the scheduled time.

**Resolver work.** `implements Schedulable` → emit
`FrameworkEntry(schedulable)` edge into `execute(SchedulableContext)`.

**Acceptance gate.**

- `framework_annotation_unresolved` cluster `schedulable_execute`:
  26 → 0.
- `metrics.dead_code.count` delta: −26.

### 4.4 Cluster `queueable_execute` — 14 FQNs (priority P0)

**What the runtime does.** `System.enqueueJob(new Foo())` invokes
`execute(QueueableContext)`.

**Resolver work.** `implements Queueable` → emit
`FrameworkEntry(queueable)` edge.

**Acceptance gate.**

- `framework_annotation_unresolved` cluster `queueable_execute`:
  14 → 0.
- `metrics.dead_code.count` delta: −14.

### 4.5 Cluster `aura_lwc_controller_method` (`@AuraEnabled`) — 5 FQNs + future growth (priority P1)

**What the runtime does.** An LWC JavaScript module or Aura
component imports an Apex method decorated with `@AuraEnabled` via
`@wire` or `@ApexMethod`. The platform dispatches the JS-originated
call to the Apex static.

**Resolver work.** Emit `FrameworkEntry(aura_enabled)` on every
`@AuraEnabled` method. This step is *authoritative* — the annotation
alone guarantees the platform can call the method.

A follow-on step (tracked against R25) emits a paired
`DeclarativeWiring(Lwc)` edge from the specific JS caller when the
LWC HTML / JS parser lands. Keeping both edges is intentional:
`FrameworkEntry` proves the method is reachable in principle;
`DeclarativeWiring` identifies the exact caller. Same pattern will
apply to Aura once R28 ships.

**Acceptance gate (resolver-only).**

- `framework_annotation_unresolved` cluster
  `aura_lwc_controller_method`: 5 → 0.
- `metrics.dead_code.count` delta: −5.

NPSP is unusually small in this cluster (many `@AuraEnabled`
methods in NPSP are *already* reachable via Apex tests which pass
the `exclude_tests` filter out). Cluster size on a
customer-production Salesforce repo is typically 30–200+ methods.
Gate text above is pre-registered relative to **NPSP**; ecosystem
benchmarks (non-NPSP) are tracked separately.

### 4.6 Cluster `install_uninstall_handler` — 2 FQNs (priority P2)

**What the runtime does.** `implements InstallHandler` → platform
invokes `onInstall(InstallContext)` on package install; ditto
`UninstallHandler.onUninstall`.

**Resolver work.** `implements (Un)InstallHandler` →
`FrameworkEntry(install_handler)` / `FrameworkEntry(uninstall_handler)`.

**Acceptance gate.**

- `framework_annotation_unresolved` cluster
  `install_uninstall_handler`: 2 → 0.

### 4.7 Cluster `tdtm_runnable_api` — 2 FQNs (priority P1)

**What the runtime does.** `TDTM_Runnable.run(...)` is the abstract
API that every TDTM handler class overrides. The `Config_API` and
`RunnableMutable` variants are dispatched by
`TDTM_TriggerHandler.processTrigger` via `Type.forName(...).newInstance().run(...)`
reflection.

**Resolver work.** Scan `force-app/tdtm/` (or any path where a
`TriggerHandlers.json` / custom-metadata export exists) for every
registered handler class name. For each, emit
`FrameworkEntry(tdtm_handler, confidence=0.9)` into that class's
`run(...)` method. This is idiom 12 in the dispatch matrix — synthetic
confidence reflects that the evidence is a config file, not the AST.

This cluster also folds in the *existing* heuristic now
implemented in `graphengine-analysis/src/health/dead_code_classifier/frameworks/tdtm.rs::looks_like_tdtm_handler`.
The heuristic goes away once the config-file resolver ships.

**Acceptance gate.**

- `framework_annotation_unresolved` cluster `tdtm_runnable_api`:
  2 → 0.
- `dynamic_dispatch_target` total: currently 47 → expected 0
  (every `_TDTM` class's `run(...)` whose class name appears in
  `TriggerHandlers.json` becomes `live`).
- The R24 over-match (`AccountAdapter::onAfterUpdate(TDTM_Runnable.DmlWrapper)`)
  disappears because the resolver no longer looks at FQN
  substrings.

### 4.8 Cluster `other_apex_framework_entry` — 5 FQNs (priority P1, mixed idioms)

These five are not homogeneous; each is its own mini-cluster:

| FQN | idiom | dispatch-matrix row |
| --- | ----- | ------------------- |
| `ADDR_Validator_REST::verifyRecord(String)` | `@RestResource` / `@HttpPost` static | 8 |
| `BDI_DataImportService::mapFieldsForDIObject(String,String,List)` | `global` modifier (managed-package consumer) | 10 |
| `TDTM_Runnable::run(List,List,Action,Schema.DescribeSObjectResult)` | abstract-method hook of an implements-Runnable class | 7 (covered in §4.7) |
| `UTIL_CustomSettings_API::getBDESettings()` | `global` modifier | 10 |
| `UTIL_RecordTypeSettingsUpdate::getInstance()` | `global` modifier | 10 |

**Resolver work.** Three rules — `@RestResource` annotation,
`global` modifier, `@InvocableMethod` (not present in NPSP but
specified to avoid re-opening the plan) — each emit their named
`FrameworkEntry` edge. `@RemoteAction` is specified for
completeness and is zero-count in NPSP.

**Acceptance gate.**

- `framework_annotation_unresolved` cluster
  `other_apex_framework_entry`: 5 → 0.
- No new false positives elsewhere. Verified via hand-audit
  resample.

### 4.9 Cluster `dynamic_dispatch_target::tdtm_data_gateway` — 2 FQNs (priority P1, covered by R23)

**What the runtime does.** `TDTM_TriggerHandler.cls:85` holds a
typed field `TDTM_iTableDataGateway dao` and invokes
`dao.isEmpty()`. The parser today does not link the field-access →
interface-method dispatch (R23); the classifier falls back to the
TDTM heuristic and slaps `dynamic_dispatch_target` on it even though
the dispatch is a plain in-source call.

**Resolver work.** This is *not* a `FrameworkEntry` case — it is a
plain `Call(AstLinked)` edge that the existing Apex resolver
should already be emitting. Fix lives in
`graphengine-parsing/src/syntax/language/apex/heuristic_resolver.rs`
and the pre-computed class registry (R23 cluster in §4.11).

**Acceptance gate (cross-links to §4.11).**

- `dynamic_dispatch_target`: −2.
- Hand-audit `dynamic_dispatch_target` remains 0 / 10 wrong.

### 4.10 Cluster `visualforce_controller` (implicit — `no_callers`) — estimated 20–40 FQNs (priority P2)

NPSP has ~65 `_CTRL` classes paired with `.page` files under
`force-app/main/default/pages/`. The rev 4 hand-audit caught
`STG_PanelOppCampaignMembers_CTRL::idPanel()` as `wrong-vf-wiring`.
A systematic sweep has not been run; the 20–40 estimate comes from
counting `*_CTRL.cls` classes that have zero incoming edges in the
current graph after Batchable / Schedulable / Queueable resolvers
ship.

**Resolver work.** Add a `.page` file reader in
`graphengine-parsing/src/syntax/language/apex/`:

- Parse `<apex:page controller="Foo" extensions="Bar, Baz">`
  declarations — emit `DeclarativeWiring(Vf)` edges on each
  controller / extension public method.
- Parse `{!method}` / `{!getter}` action and expression bindings in
  the page body — emit a `DeclarativeWiring(Vf)` edge to the named
  controller / extension method.
- Handle `<apex:commandButton action="{!idPanel}">` action
  references explicitly.

**Acceptance gate.** Pre-registered against a future rev (post-§4.1–§4.9
landing):

- `no_callers` count for `_CTRL.cls` methods: existing-count → 0
  after a followup hand-audit round (seeded) confirms 0 / 10
  wrong on a new `visualforce_bound` bucket.
- A new `DeclarativeWiring_vf` edge-provenance counter exposed in
  the HealthReport.

(This cluster is larger on customer VF-heavy orgs. NPSP is
modestly-sized here because most `_CTRL` classes have `.cls` tests
that *do* link. The resolver is still needed — the current "works
by accident via tests" is not a trustable signal.)

### 4.11 Cluster `apex_ast_resolver_gaps` (R23) — 17 regression fixtures + population-level impact (priority P0)

**What is broken.** The rev 4 and rev 5 Layer-5 hand-audits sampled
10 FQNs each from `no_callers` and found 7 + 10 = **17 distinct
failures** whose common root cause is the Apex heuristic resolver
(`graphengine-parsing/src/syntax/language/apex/heuristic_resolver.rs`)
failing to emit edges for six common idioms:

1. Intra-class constructor call `new X(...)` where `X` is declared
   in the same file. **(Phase A, TR-A.1.)**
2. Cross-file constructor call `new X(...)` where `X` is declared
   in a different file (registry lookup fails). **(Phase A, TR-A.2.)**
3. Typed-field dispatch `instanceField.method(...)`. **(Phase A,
   TR-A.3.)**
4. Inner-class constructor / method dispatch. **(Phase A, TR-A.6.)**
5. Sibling overload dispatch in the same class. **(Phase A, TR-A.4.)**
6. Implicit `toString()` call in string concatenation. **(Deferred
   to Phase D, TR-D.3; see §4.11.2 below for the rationale.)**

All five Phase-A idioms share a prerequisite: a richer **type
oracle** than today's `ApexClassRegistry` provides. Today the
registry stores only type identity (`api_name`, `kind`, `source`).
Every one of TR-A.1 through TR-A.6 needs per-class methods,
constructors, fields with declared types, inner-class references,
and inheritance links. That foundation is ticketed separately as
**TR-A.0** — landed first so each resolver ticket consumes a
known-good oracle rather than building its own. Full TR-A.0 scope
lives in `TRUTHFUL_SCANS_ROADMAP.md` §5.

**Regression fixtures** (from `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`, Rounds 2 + 3):

From Round 2 (rev 4):

- `GiftEntryProcessorQueueFinalizer::GiftEntryProcessorQueueFinalizer(GiftBatchId)` — cross-file constructor
- `UTIL_IntegrationConfig::initCallableApi()` — intra-class call
- `RD2_DataMigrationBase_BATCH.Logger::Logger(SObjectType,String,String)` — intra-file inner-class constructor
- `HouseholdMembers::HouseholdMembers(List,Map)` — cross-file constructor
- `fflib_Comparator::compare(String,String)` — intra-class overload
- `UTIL_JobProgress_CTRL.BatchJob::BatchJob(AsyncApexJob)` — intra-file inner-class constructor
- `UTIL_Permissions::canUpdate(SObjectType)` — typed-field dispatch

From Round 3 (rev 5):

- `fflib_SObjectDomain.TestSObjectDisableBehaviour::TestSObjectDisableBehaviour(List)` — intra-file inner-class constructor
- `RD_InstallScript_BATCH::RD_InstallScript_BATCH()` — cross-file zero-arg constructor
- `Gift::Gift(GiftId)` — cross-file constructor
- `GiftBatch::GiftBatch()` — cross-file default constructor
- `SfdoInstrumentationService::log(...)` — typed-field dispatch (DI)
- `Contacts::loadAccountByIdMap()` — typed-field dispatch (domain layer)
- `CRLP_Account_AccSoftCredit_BATCH::CRLP_Account_AccSoftCredit_BATCH()` — cross-file zero-arg constructor
- `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)` — inner-class dispatch
- `UTIL_OrderBy.SortableRecord::SortableRecord(sObject,FieldExpression)` — intra-file inner-class constructor
- `fflib_StringBuilder.CommaDelimitedListBuilder::toString()` — implicit `toString()` in concat

Two exemplars *correctly* classified (no resolver fix required) —
kept in the fixture set as negative controls:

- `RD2_DataMigrationBase_BATCH.Logger::addError(Exception,Id)` — genuinely unused
- `fflib_IUnitOfWorkFactory::newInstance(fflib_SObjectUnitOfWork.IDML)` — interface declaration; implementations carry the production calls

**Resolver work.** All five items below consume the TR-A.0 type
oracle; each maps 1:1 to a Phase-A ticket.

1. **Class registry consultation on every `new X(...)` (TR-A.1 +
   TR-A.2).** The registry is already built
   (`graphengine-parsing/src/syntax/language/apex/class_registry.rs`);
   the resolver simply doesn't consult it on constructor
   expressions. Add a `resolve_constructor_call(name, arg_types)`
   arm that walks `ApexClassSymbols.constructors` via TR-A.0's
   oracle. TR-A.1 covers sibling classes in the same file;
   TR-A.2 covers cross-file classes.
2. **Field-type propagation (TR-A.3).** Parse field declarations
   (already populated into `ApexClassSymbols.fields` by TR-A.0)
   and carry the declared type through `instanceField.method()`
   so method lookup uses the declared type rather than string-
   matching on the method name.
3. **Inner-class containment walking (TR-A.6).** When resolving
   `X.Inner.method()` or `new Outer.Inner(...)`, traverse
   `ApexClassSymbols.inner_classes` on the outer class. Inner-
   class method overrides resolve against the outer class's
   `parent_class` / `implemented_interfaces` chain. Load-bearing
   for the 48-node §4.11.1 revert population (≈ 40 of 48 are
   inner-class shapes).
4. **Overload resolution (TR-A.4).** When multiple methods share
   a name, use Apex's overload rules (exact-type match > widening
   match > implicit-conversion). Store the full
   `Vec<ApexParameter>` signature per method (TR-A.0 already
   does this); resolution builds a candidate set at lookup time.
   Stop short of full generic resolution — NPSP doesn't need it
   and it's a rabbit hole.
5. **Implicit `toString()` call synthesis — DEFERRED to Phase D
   (TR-D.3).** See §4.11.2 for the rationale and fixture
   carve-out.

**Acceptance gate (Phase A).**

- **16 of 17** regression fixtures (see list above) appear as
  *live* in the rev-7 baseline (their callers' edges are now
  linked). The one carved-out fixture is
  `fflib_StringBuilder.CommaDelimitedListBuilder::toString()` —
  see §4.11.2.
- **≥ 42 of 48** §4.11.1 revert-population nodes re-resolve
  live.
- `no_callers` total count drops by the population-level impact
  estimated at **−200 to −650** methods once the resolver fixes
  generalise. The exact figure is pre-registered on the first
  run of the enhanced resolver; a 20%+ over-correction or a
  3%+ under-correction triggers a blocker-level investigation.
- Hand-audit Round 5 (rev 7) on `no_callers`: **< 2 / 10 wrong**
  — this is the Phase-A closure gate.

### 4.11.2 Carve-out: implicit `toString()` synthesis → Phase D TR-D.3

Idiom 6 from §4.11 ("implicit `toString()` call in string
concatenation") is deferred out of Phase A for honesty reasons,
not scope reasons. The spec calls for
`Call(AstLinked, confidence=0.7)` — a per-edge numeric confidence
downgrade acknowledging legitimate ambiguity between an anonymous
literal's implicit `toString()` and a user-overridden
`toString()`. Today's `EdgeKind` / `EdgeProvenance` carries no
numeric confidence field; the per-edge `confidence: f32` lands in
**Phase D TR-D.1** as part of the `EdgeSource` enum. Emitting a
`toString()` synthesis against the current schema would make the
synthesis indistinguishable from AST-linked fact — the exact
R26 / R31 failure shape documented in `FOLLOWUP_RISKS.md`.

**Where it lands instead.** Phase D TR-D.3 (declarative classifier
rule engine). The rule reads naturally as data:
*if expression is non-String AND appears in string-concat context
AND target class declares `toString()` → emit `Call(EdgeSource::HeuristicName, confidence=0.7)` with evidence
"implicit toString in string concatenation".* The rule row carries
its `sampled_positives` list (the two NPSP fixtures below) and
its `sampled_negatives` list (a user-overridden `toString` call
site, and a hand-string-concat site that does not implicitly
invoke `toString`) — a Layer-2 invariant (TR-D.3 acceptance)
refuses to promote the rule if either list is empty. This closes
R31's structural failure mode for this exact idiom.

**Pre-registered Phase-D fixtures:**

- `fflib_StringBuilder.CommaDelimitedListBuilder::toString()` —
  from §8.3 Round 3. Positive sample for TR-D.3.
- `fflib_MatcherDefinitions.Eq::toString()` — from rev-6.1 Round 4
  `no_callers` sample #10. Positive sample for TR-D.3.

**Phase-A impact.** These two FQNs stay in `no_callers` through
Phase A and Phase B. They are expected; not a gate failure. The
Phase-A acceptance-gate wording ("16 of 17 fixtures live") pins
the carve-out.

### 4.11.1 Additional regression fixtures from the rev-6 → rev-6.1 TR-0.1.1 revert population (R31)

TR-0.1.1 (closing R31 on rev 6.1) reverted 48 NPSP nodes from
`dynamic_dispatch_target` back to `no_callers` — the classifier
correction exposes their true failure mode (R23 resolver gap),
so Phase A's resolver work owns them. Adding them to the Wave 3
regression fixture set ensures the resolver enhancements
explicitly cover the shapes R31 surfaced rather than absorbing
them silently into the general `no_callers` drop.

**Shape distribution (all 48 nodes live inside `*_TDTM.cls` files):**

1. Inner-class non-`run` methods invoked via
   typed-field dispatch from the outer class:
   - `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)`
   - `CON_ContactMerge_TDTM.ContactMergeProcessor::getAccounts(List)`
   - (…46 more in the same shape family)
2. Outer-class non-`run` non-ctor helper methods called by
   sibling methods or override polymorphism:
   - `CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader()`
3. Inner-class constructors invoked via `new Outer.Inner(...)`
   from the outer class:
   - `RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader::FirstCascadeUndeleteLoader()`

These align exactly with §4.11's existing three idioms
(cross-file constructor, typed-field dispatch, inner-class
containment walking), so no additional resolver work is needed
beyond what §4.11 already plans — but the acceptance gate is
extended:

**Acceptance-gate addendum.** Of the 48 rev-6 → rev-6.1 revert
nodes, **≥ 42 (87.5%)** must appear as *live* in the
post-Wave-3 rev-N baseline. The 12.5% slack covers genuinely
unused helpers (NPSP has some intentional dead code inside
TDTM files). This is a hard regression gate: if the resolver
cannot re-link the majority of them, §4.11 has missed a shape.

**rev 9 revert-population result (2026-04-19).** The exact
rev 9 live-vs-`no_callers` counts across the 48-node revert
population have **not been re-measured in PR 9**. PR 9's scope
was the R46 cross-language keyword fix in the extractor, which
does not touch the TR-A.x resolver paths that re-link these 48
nodes. TR-A.1..A.6 are Phase B scope and have not shipped. The
honest rev 9 status of this gate is therefore: **not-yet-measured,
awaiting Phase B TR-A.x**. The §4.11 fixtures (17 FQNs in §8.3)
are the right immediate oracle; the 48-node revert population
acceptance is a *post-Phase-B* gate. This contradicts the
pre-Round-5 reading that the §4.11.1 addendum was gate-ready
on rev 9; the rev 9 Round 5 audit failure on independent
shapes (R39 / R41 / R45) does not move this gate's numerator
or denominator.

See `docs/workstreams/proof-foundation-gap/REGRESSION_RESULTS.md`
§"Engine revision 6.1" and
`docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`
§"Round 4 re-scoring — Engine revision 6.1" for the full
provenance trail.

### 4.12 Abstract-base interface inheritance propagation (priority P1, covered by `FOLLOWUP_RISKS` R11)

**What the runtime does.** In NPSP, `CRLP_Batch_Base_NonSkew`
abstract-extends a class that declares `implements Database.Batchable`.
Wave 1's fix (`collect_interface_method_markers`) explicitly
limited itself to direct implements; the abstract-base case is
this cluster.

**Resolver work.** Walk the class hierarchy when computing
entry-point markers. If any ancestor class declares `implements X`,
propagate the marker down.

**Acceptance gate.** Size not yet measured — a `class_hierarchy_walk`
unit-test fixture is expected to surface 20–50 NPSP classes
belonging here. Pre-registered: 0 / 10 wrong on resample. True
count checked after §4.1 lands so the `framework_annotation_unresolved`
bucket is stable.

## 5. Consolidated delta expectations (pre-registered)

These are what the next NPSP canary run should show **after the
full Wave 3 Apex Framework Resolver lands**. Pre-registering them
here means any drift on the actual run triggers a hand-audit
round, not a silent metric change.

| metric | rev 5 | post-Wave-3 (pre-registered) | delta |
| ------ | ----- | ---------------------------- | ----- |
| `reason.framework_annotation_unresolved` | 190 | **0 ± 5** | −190 ± 5 |
| `reason.dynamic_dispatch_target`         | 47  | **0** | −47 |
| `reason.no_callers` (production)         | 1,349 | 700 – 1,150 (wide band; depends on §4.11 generalisation surface) | −200 to −650 |
| `reason.declarative_wiring_unparsed`     | 137 | 137 (unchanged — LWC / Aura / Jest resolvers are separate) | 0 |
| `reason.visibility_private_unused`       | 3   | 0–3 (Aura / Jest detection per R28 may shrink this; not strictly owned by this plan) | −0 to −3 |
| `metrics.dead_code.count` (production)   | 1,726 | 750 – 1,300 | −450 to −975 |
| `edge_provenance_counts.FrameworkEntry`  | 0   | **≥ 300** (72 + 26 + 26 + 14 + 47 + 5 + 2 + …) | +300 |
| `edge_provenance_counts.DeclarativeWiring` | 0 | 0 (deferred to VF / LWC / Aura resolver workstreams) | 0 |

Confidence bands reflect the following uncertainties:

- Abstract-base hierarchy walk (§4.12) size is unknown until run.
- `@AuraEnabled` count will rise on non-NPSP benchmarks; this
  plan's numbers are NPSP-specific.
- `no_callers` band reflects whether R23 fixes touch ~5 idioms
  (lower band) or generalise to ~8 idioms (upper band).

## 6. Testing strategy (Layer-1 through Layer-5, repeated)

Same protocol as Waves 1–2:

- **Layer 1 (unit tests).** Per rule set: one `FrameworkEntry`
  emission test per cluster. Fixture: a minimal `.cls` / `.trigger`
  file exercising the annotation / interface / convention.
- **Layer 2 (invariants).** Every new `FrameworkEntry` edge must
  carry (a) a non-empty `evidence` string citing file:line, (b)
  `confidence` in `(0, 1]`, (c) `source = FrameworkEntry(tag)` with
  `tag` in the enum registered by `graphengine-parsing/src/domain/frameworks.rs`.
- **Layer 3 (integration fixtures).** Extend `polyglot_mixed` with
  an Apex `@AuraEnabled` class actually called by an LWC file — the
  fixture's ground-truth `reason_breakdown.json` asserts the method
  becomes `live`, *not* `framework_annotation_unresolved`.
- **Layer 4 (NPSP canary).** After each cluster ships, re-run
  `experiments/run_canaries.sh` and diff against the pre-registered
  table in §5.
- **Layer 5 (hand-audit resample).** At the end of Wave 3, Round 4
  of the Layer-5 audit runs against rev 6 with seed `20260420` on
  the four gate-eligible buckets:
  - `framework_annotation_unresolved` (expected to be near-empty)
  - `dynamic_dispatch_target` (expected to be empty)
  - `no_callers` (target: < 2 / 10 wrong)
  - `visibility_private_unused` (unchanged if R28 not addressed)

  All three remaining-population buckets **must pass the < 2 / 10
  gate** before Wave 3 is considered complete.

## 7. Out-of-plan (explicit)

- **Edge-provenance migration (R24).** This plan emits
  `EdgeSource::FrameworkEntry` edges but does *not* retire the
  `DeadCodeReason` classifier. Both live side by side until R24
  lands. The classifier stops reading the heuristic TDTM rule
  (§4.7 makes it redundant), but the rest of the classifier stays
  as a belt-and-suspenders.
- **`classifier_confidence` field (R26).** Every `FrameworkEntry`
  edge *does* carry its own `confidence` field per this plan; the
  separate `DeadCodeVerdict::classifier_confidence` is still
  deferred.
- **`DeadCodeReason` enum retirement (R27).** Deferred.
- **Python Django URL routing resolver.** Scope boundary — tracked
  as R16. The dispatch-matrix rows 14–16 are specified here because
  they share `EdgeSource::DeclarativeWiring`, but their
  implementation is separate plans.
- **JavaScript LWC HTML template parser.** Scope boundary — tracked
  as R25 / R28.
- **Visualforce `.page` parser.** §4.10 specifies the shape; the
  actual implementation is a separate workstream (owner TBD).

## 8. Enumerated backlog (all FQNs)

> Source: `experiments/results/NPSP/rev5/ab_report.json`, production
> slice, A/B-injected. This is the authoritative list against which
> the acceptance gates in §4 are evaluated. Any FQN that is listed
> here but is *not* `live` after the cluster's resolver ships is a
> gate failure.

### 8.1 `framework_annotation_unresolved` — 152 FQNs

#### Cluster `batchable_contract` — 72 FQNs

- `ADDR_Seasonal_SCHED::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/ADDR_Seasonal_SCHED.cls`
- `ADDR_Seasonal_SCHED::finish(Database.BatchableContext)` — `force-app/main/default/classes/ADDR_Seasonal_SCHED.cls`
- `ADDR_Seasonal_SCHED::start(Database.BatchableContext)` — `force-app/main/default/classes/ADDR_Seasonal_SCHED.cls`
- `ALLO_MakeDefaultAllocations_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/ALLO_MakeDefaultAllocations_BATCH.cls`
- `ALLO_MakeDefaultAllocations_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/ALLO_MakeDefaultAllocations_BATCH.cls`
- `ALLO_MakeDefaultAllocations_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/ALLO_MakeDefaultAllocations_BATCH.cls`
- `BDI_DataImport_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/BDI_DataImport_BATCH.cls`
- `BDI_DataImport_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/BDI_DataImport_BATCH.cls`
- `BDI_DataImport_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/BDI_DataImport_BATCH.cls`
- `CONV_Account_Conversion_BATCH::execute(Database.BatchableContext,Sobject[])` — `force-app/main/default/classes/CONV_Account_Conversion_BATCH.cls`
- `CONV_Account_Conversion_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/CONV_Account_Conversion_BATCH.cls`
- `CONV_Account_Conversion_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/CONV_Account_Conversion_BATCH.cls`
- `CRLP_SkewDispatcher_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/CRLP_SkewDispatcher_BATCH.cls`
- `CRLP_SkewDispatcher_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/CRLP_SkewDispatcher_BATCH.cls`
- `CRLP_SkewDispatcher_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/CRLP_SkewDispatcher_BATCH.cls`
- `DeceasedBatch::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/DeceasedBatch.cls`
- `DeceasedBatch::finish(Database.BatchableContext)` — `force-app/main/default/classes/DeceasedBatch.cls`
- `DeceasedBatch::start(Database.BatchableContext)` — `force-app/main/default/classes/DeceasedBatch.cls`
- `HH_CampaignDedupe_BATCH::execute(Database.BatchableContext,Sobject[])` — `force-app/main/default/classes/HH_CampaignDedupe_BATCH.cls`
- `HH_CampaignDedupe_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/HH_CampaignDedupe_BATCH.cls`
- `HH_CampaignDedupe_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/HH_CampaignDedupe_BATCH.cls`
- `HH_HouseholdNaming_BATCH::execute(Database.BatchableContext,Sobject[])` — `force-app/main/default/classes/HH_HouseholdNaming_BATCH.cls`
- `HH_HouseholdNaming_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/HH_HouseholdNaming_BATCH.cls`
- `HH_HouseholdNaming_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/HH_HouseholdNaming_BATCH.cls`
- `LVL_LevelAssign_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/LVL_LevelAssign_BATCH.cls`
- `LVL_LevelAssign_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/LVL_LevelAssign_BATCH.cls`
- `LVL_LevelAssign_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/LVL_LevelAssign_BATCH.cls`
- `OPP_PrimaryContactRoleMerge_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/OPP_PrimaryContactRoleMerge_BATCH.cls`
- `OPP_PrimaryContactRoleMerge_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/OPP_PrimaryContactRoleMerge_BATCH.cls`
- `OPP_PrimaryContactRoleMerge_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/OPP_PrimaryContactRoleMerge_BATCH.cls`
- `OPP_PrimaryContact_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/OPP_PrimaryContact_BATCH.cls`
- `OPP_PrimaryContact_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/OPP_PrimaryContact_BATCH.cls`
- `OPP_PrimaryContact_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/OPP_PrimaryContact_BATCH.cls`
- `PMT_PaymentCreator_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/PMT_PaymentCreator_BATCH.cls`
- `PMT_PaymentCreator_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/PMT_PaymentCreator_BATCH.cls`
- `PMT_PaymentCreator_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/PMT_PaymentCreator_BATCH.cls`
- `RD2_DataMigrationBase_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/RD2_DataMigrationBase_BATCH.cls`
- `RD2_DataMigrationBase_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/RD2_DataMigrationBase_BATCH.cls`
- `RD2_DataMigrationBase_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/RD2_DataMigrationBase_BATCH.cls`
- `RD2_OpportunityEvaluation_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/RD2_OpportunityEvaluation_BATCH.cls`
- `RD2_OpportunityEvaluation_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/RD2_OpportunityEvaluation_BATCH.cls`
- `RD2_OpportunityEvaluation_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/RD2_OpportunityEvaluation_BATCH.cls`
- `RD_InstallScript_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/RD_InstallScript_BATCH.cls`
- `RD_InstallScript_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/RD_InstallScript_BATCH.cls`
- `RD_InstallScript_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/RD_InstallScript_BATCH.cls`
- `RD_RecurringDonations_BATCH::execute(Database.BatchableContext,SObject[])` — `force-app/main/default/classes/RD_RecurringDonations_BATCH.cls`
- `RD_RecurringDonations_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/RD_RecurringDonations_BATCH.cls`
- `RD_RecurringDonations_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/RD_RecurringDonations_BATCH.cls`
- `RLLP_OppAccRollup_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/RLLP_OppAccRollup_BATCH.cls`
- `RLLP_OppAccRollup_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/RLLP_OppAccRollup_BATCH.cls`
- `RLLP_OppAccRollup_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/RLLP_OppAccRollup_BATCH.cls`
- `RLLP_OppContactRollup_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/RLLP_OppContactRollup_BATCH.cls`
- `RLLP_OppContactRollup_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/RLLP_OppContactRollup_BATCH.cls`
- `RLLP_OppContactRollup_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/RLLP_OppContactRollup_BATCH.cls`
- `RLLP_OppHouseholdRollup_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/RLLP_OppHouseholdRollup_BATCH.cls`
- `RLLP_OppHouseholdRollup_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/RLLP_OppHouseholdRollup_BATCH.cls`
- `RLLP_OppHouseholdRollup_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/RLLP_OppHouseholdRollup_BATCH.cls`
- `RLLP_OppSoftCreditRollup_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/RLLP_OppSoftCreditRollup_BATCH.cls`
- `RLLP_OppSoftCreditRollup_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/RLLP_OppSoftCreditRollup_BATCH.cls`
- `RLLP_OppSoftCreditRollup_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/RLLP_OppSoftCreditRollup_BATCH.cls`
- `TEMP_ClosePledgedDonations::execute(Database.BatchableContext,List)` — `unpackaged/config/config_ldv_org_for_testing/classes/TEMP_ClosePledgedDonations.cls`
- `TEMP_ClosePledgedDonations::finish(Database.BatchableContext)` — `unpackaged/config/config_ldv_org_for_testing/classes/TEMP_ClosePledgedDonations.cls`
- `TEMP_ClosePledgedDonations::start(Database.BatchableContext)` — `unpackaged/config/config_ldv_org_for_testing/classes/TEMP_ClosePledgedDonations.cls`
- `UTIL_AbstractChunkingLDV_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/UTIL_AbstractChunkingLDV_BATCH.cls`
- `UTIL_AbstractChunkingLDV_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/UTIL_AbstractChunkingLDV_BATCH.cls`
- `UTIL_AbstractChunkingLDV_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/UTIL_AbstractChunkingLDV_BATCH.cls`
- `UTIL_OrgTelemetry_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/UTIL_OrgTelemetry_BATCH.cls`
- `UTIL_OrgTelemetry_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/UTIL_OrgTelemetry_BATCH.cls`
- `UTIL_OrgTelemetry_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/UTIL_OrgTelemetry_BATCH.cls`
- `UTIL_OrgTelemetry_SObject_BATCH::execute(Database.BatchableContext,List)` — `force-app/main/default/classes/UTIL_OrgTelemetry_SObject_BATCH.cls`
- `UTIL_OrgTelemetry_SObject_BATCH::finish(Database.BatchableContext)` — `force-app/main/default/classes/UTIL_OrgTelemetry_SObject_BATCH.cls`
- `UTIL_OrgTelemetry_SObject_BATCH::start(Database.BatchableContext)` — `force-app/main/default/classes/UTIL_OrgTelemetry_SObject_BATCH.cls`

#### Cluster `apex_trigger_body` — 26 FQNs

- `TDTM_Account::__trigger__()` — `force-app/tdtm/triggers/TDTM_Account.trigger`
- `TDTM_AccountSoftCredit::__trigger__()` — `force-app/tdtm/triggers/TDTM_AccountSoftCredit.trigger`
- `TDTM_Address::__trigger__()` — `force-app/tdtm/triggers/TDTM_Address.trigger`
- `TDTM_Affiliation::__trigger__()` — `force-app/tdtm/triggers/TDTM_Affiliation.trigger`
- `TDTM_Allocation::__trigger__()` — `force-app/tdtm/triggers/TDTM_Allocation.trigger`
- `TDTM_Campaign::__trigger__()` — `force-app/tdtm/triggers/TDTM_Campaign.trigger`
- `TDTM_CampaignMember::__trigger__()` — `force-app/tdtm/triggers/TDTM_CampaignMember.trigger`
- `TDTM_Contact::__trigger__()` — `force-app/tdtm/triggers/TDTM_Contact.trigger`
- `TDTM_DataImport::__trigger__()` — `force-app/tdtm/triggers/TDTM_DataImport.trigger`
- `TDTM_DataImportBatch::__trigger__()` — `force-app/tdtm/triggers/TDTM_DataImportBatch.trigger`
- `TDTM_EngagementPlan::__trigger__()` — `force-app/tdtm/triggers/TDTM_EngagementPlan.trigger`
- `TDTM_EngagementPlanTask::__trigger__()` — `force-app/tdtm/triggers/TDTM_EngagementPlanTask.trigger`
- `TDTM_FormTemplate::__trigger__()` — `force-app/tdtm/triggers/TDTM_FormTemplate.trigger`
- `TDTM_GeneralAccountingUnit::__trigger__()` — `force-app/tdtm/triggers/TDTM_GeneralAccountingUnit.trigger`
- `TDTM_GrantDeadline::__trigger__()` — `force-app/tdtm/triggers/TDTM_GrantDeadline.trigger`
- `TDTM_HouseholdObject::__trigger__()` — `force-app/tdtm/triggers/TDTM_HouseholdObject.trigger`
- `TDTM_Lead::__trigger__()` — `force-app/tdtm/triggers/TDTM_Lead.trigger`
- `TDTM_Level::__trigger__()` — `force-app/tdtm/triggers/TDTM_Level.trigger`
- `TDTM_Opportunity::__trigger__()` — `force-app/tdtm/triggers/TDTM_Opportunity.trigger`
- `TDTM_OpportunityContactRole::__trigger__()` — `force-app/tdtm/triggers/TDTM_OpportunityContactRole.trigger`
- `TDTM_PartialSoftCredit::__trigger__()` — `force-app/tdtm/triggers/TDTM_PartialSoftCredit.trigger`
- `TDTM_Payment::__trigger__()` — `force-app/tdtm/triggers/TDTM_Payment.trigger`
- `TDTM_RecurringDonation::__trigger__()` — `force-app/tdtm/triggers/TDTM_RecurringDonation.trigger`
- `TDTM_Relationship::__trigger__()` — `force-app/tdtm/triggers/TDTM_Relationship.trigger`
- `TDTM_Task::__trigger__()` — `force-app/tdtm/triggers/TDTM_Task.trigger`
- `TDTM_User::__trigger__()` — `force-app/tdtm/triggers/TDTM_User.trigger`

#### Cluster `schedulable_execute` — 26 FQNs

- `ADDR_Seasonal_SCHED::execute(SchedulableContext)` — `force-app/main/default/classes/ADDR_Seasonal_SCHED.cls`
- `ALLO_Rollup_SCHED::execute(SchedulableContext)` — `force-app/main/default/classes/ALLO_Rollup_SCHED.cls`
- `BDI_DataImportBatch_SCHED::execute(SchedulableContext)` — `force-app/main/default/classes/BDI_DataImportBatch_SCHED.cls`
- `CRLP_AccountSkew_AccSoftCredit_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_AccountSkew_AccSoftCredit_BATCH.cls`
- `CRLP_AccountSkew_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_AccountSkew_BATCH.cls`
- `CRLP_AccountSkew_SoftCredit_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_AccountSkew_SoftCredit_BATCH.cls`
- `CRLP_Account_AccSoftCredit_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_Account_AccSoftCredit_BATCH.cls`
- `CRLP_Account_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_Account_BATCH.cls`
- `CRLP_Account_SoftCredit_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_Account_SoftCredit_BATCH.cls`
- `CRLP_ContactSkew_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_ContactSkew_BATCH.cls`
- `CRLP_ContactSkew_SoftCredit_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_ContactSkew_SoftCredit_BATCH.cls`
- `CRLP_Contact_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_Contact_BATCH.cls`
- `CRLP_Contact_SoftCredit_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_Contact_SoftCredit_BATCH.cls`
- `CRLP_GAU_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_GAU_BATCH.cls`
- `CRLP_RDSkew_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_RDSkew_BATCH.cls`
- `CRLP_RD_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/CRLP_RD_BATCH.cls`
- `ERR_AsyncErrors_SCHED::execute(SchedulableContext)` — `force-app/main/default/classes/ERR_AsyncErrors_SCHED.cls`
- `LVL_LevelAssign_SCHED::execute(SchedulableContext)` — `force-app/main/default/classes/LVL_LevelAssign_SCHED.cls`
- `OPP_OpportunityNaming_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/OPP_OpportunityNaming_BATCH.cls`
- `RD2_OpportunityEvaluation_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/RD2_OpportunityEvaluation_BATCH.cls`
- `RD_RecurringDonations_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/RD_RecurringDonations_BATCH.cls`
- `RLLP_OppAccRollup_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/RLLP_OppAccRollup_BATCH.cls`
- `RLLP_OppContactRollup_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/RLLP_OppContactRollup_BATCH.cls`
- `RLLP_OppHouseholdRollup_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/RLLP_OppHouseholdRollup_BATCH.cls`
- `RLLP_OppSoftCreditRollup_BATCH::execute(SchedulableContext)` — `force-app/main/default/classes/RLLP_OppSoftCreditRollup_BATCH.cls`
- `UTIL_MasterSchedulable::execute(SchedulableContext)` — `force-app/main/default/classes/UTIL_MasterSchedulable.cls`

#### Cluster `queueable_execute` — 14 FQNs

- `CON_ContactMerge_TDTM.ContactMergeFixupQueueable::execute(QueueableContext)` — `force-app/main/default/classes/CON_ContactMerge_TDTM.cls`
- `CRLP_ResetRollupFieldsQueueable::execute(QueueableContext)` — `force-app/main/default/classes/CRLP_ResetRollupFieldsQueueable.cls`
- `CRLP_RollupQueueable::execute(QueueableContext)` — `force-app/main/default/classes/CRLP_RollupQueueable.cls`
- `CRLP_TEST_VALIDATE_ROLLUPS.CreateDataQueueable::execute(QueueableContext)` — `unpackaged/config/crlp_testing/classes/CRLP_TEST_VALIDATE_ROLLUPS.cls`
- `CRLP_TEST_VALIDATE_ROLLUPS.ExecuteCustomizableRollupsPart1::execute(QueueableContext)` — `unpackaged/config/crlp_testing/classes/CRLP_TEST_VALIDATE_ROLLUPS.cls`
- `CRLP_TEST_VALIDATE_ROLLUPS.ExecuteCustomizableRollupsPart2::execute(QueueableContext)` — `unpackaged/config/crlp_testing/classes/CRLP_TEST_VALIDATE_ROLLUPS.cls`
- `ERR_AsyncErrors::execute(QueueableContext)` — `force-app/main/default/classes/ERR_AsyncErrors.cls`
- `ElevateBatchCapturer::execute(QueueableContext)` — `force-app/main/default/classes/ElevateBatchCapturer.cls`
- `GiftEntryProcessorQueue::execute(QueueableContext)` — `force-app/main/default/classes/GiftEntryProcessorQueue.cls`
- `RD2_EnablementDelegate_CTRL.EnablementQueueable::execute(QueueableContext)` — `force-app/main/default/classes/RD2_EnablementDelegate_CTRL.cls`
- `RD2_QueueableService.CancelCommitmentService::execute(QueueableContext)` — `force-app/main/default/classes/RD2_QueueableService.cls`
- `RD2_QueueableService.ElevateOpportunityMatcher::execute(QueueableContext)` — `force-app/main/default/classes/RD2_QueueableService.cls`
- `RD2_QueueableService.EvaluateInstallmentOpportunities::execute(QueueableContext)` — `force-app/main/default/classes/RD2_QueueableService.cls`
- `RD2_QueueableService.OpportunityNamingService::execute(QueueableContext)` — `force-app/main/default/classes/RD2_QueueableService.cls`

#### Cluster `aura_lwc_controller_method` — 5 FQNs

- `GE_GiftEntryController::getDonationMatchingValues()` — `force-app/main/default/classes/GE_GiftEntryController.cls`
- `GE_GiftEntryController::getGiftBatchTotalsBy(String)` — `force-app/main/default/classes/GE_GiftEntryController.cls`
- `RD2_PauseForm_CTRL::getInstallments(Id,Integer)` — `force-app/main/default/classes/RD2_PauseForm_CTRL.cls`
- `RD2_VisualizeScheduleController::getInstallments(Id,Integer)` — `force-app/main/default/classes/RD2_VisualizeScheduleController.cls`
- `RD2_VisualizeScheduleController::getSchedules(Id)` — `force-app/main/default/classes/RD2_VisualizeScheduleController.cls`

#### Cluster `other_apex_framework_entry` — 5 FQNs

- `ADDR_Validator_REST::verifyRecord(String)` — `force-app/main/default/classes/ADDR_Validator_REST.cls` (idiom 8 — `@RestResource` + `@HttpPost`)
- `BDI_DataImportService::mapFieldsForDIObject(String,String,List)` — `force-app/main/default/classes/BDI_DataImportService.cls` (idiom 10 — `global` modifier)
- `TDTM_Runnable::run(List,List,Action,Schema.DescribeSObjectResult)` — `force-app/tdtm/classes/TDTM_Runnable.cls` (idiom 7 — folded into §4.7)
- `UTIL_CustomSettings_API::getBDESettings()` — `force-app/main/default/classes/UTIL_CustomSettings_API.cls` (idiom 10)
- `UTIL_RecordTypeSettingsUpdate::getInstance()` — `force-app/main/default/classes/UTIL_RecordTypeSettingsUpdate.cls` (idiom 10)

#### Cluster `install_uninstall_handler` — 2 FQNs

- `STG_InstallScript::onInstall(InstallContext)` — `force-app/main/default/classes/STG_InstallScript.cls`
- `STG_UninstallScript::onUninstall(UninstallContext)` — `force-app/main/default/classes/STG_UninstallScript.cls`

#### Cluster `tdtm_runnable_api` — 2 FQNs

- `TDTM_Config_API::run(Boolean,Boolean,Boolean,Boolean,Boolean,Boolean,List,List,Schema.DescribeSObjectResult)` — `force-app/tdtm/classes/TDTM_Config_API.cls`
- `TDTM_RunnableMutable::run(List,List,TDTM_Runnable.Action,Schema.DescribeSObjectResult,TDTM_Runnable.DmlWrapper)` — `force-app/tdtm/classes/TDTM_RunnableMutable.cls`

### 8.2 `dynamic_dispatch_target` — 2 FQNs

#### Cluster `tdtm_data_gateway` — 2 FQNs

- `TDTM_ObjectDataGateway::isEmpty()` — `force-app/tdtm/classes/TDTM_ObjectDataGateway.cls`
- `TDTM_iTableDataGateway::isEmpty()` — `force-app/tdtm/classes/TDTM_iTableDataGateway.cls`

### 8.3 `no_callers` regression fixtures — 17 FQNs

**rev 9 Round 5 annotation (2026-04-19).** The 17 fixtures below
are the *acceptance* set for the §4.11 resolver work. The rev 9
Round 5 hand-audit (see
`docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`
§"Round 5 — Engine revision 9") drew a fresh 10-FQN sample from
the rev 9 `no_callers` pool and scored **4 / 10 wrong** against
the `< 2 / 10 wrong` Phase A closure gate. The 4 wrong verdicts
are **new shapes** outside the 17-fixture set:

- **2 × R39** — property-accessor body extraction gap
  (`RD2_OpportunityMatcher::match`-family callees invoked from
  property getters). Extractor scope, not §4.11 resolver scope.
- **1 × R41** — field-initializer body extraction gap
  (`ALLO_ManageAllocations_CTRL::getMappedAllocationsForOpp` called
  from a map-literal field initializer). **Filed during PR 9;** see
  `FOLLOWUP_RISKS.md` §R41. Extractor scope.
- **1 × R45** — chained call on a call-expression return value
  (`GE_SettingsService.getInstance().getDataImportSettings()`).
  **Filed during PR 9;** see `FOLLOWUP_RISKS.md` §R45. Resolver
  receiver-typing scope — an orthogonal gap to TR-A.4
  bare-self-dispatch.

These four wrong verdicts do **not** invalidate the §4.11
acceptance (the 17 fixtures here resolve against the TR-A.x
shapes as planned); they surface architectural gaps in the
extractor and in resolver receiver-typing that Phase A
explicitly did not promise. The Phase A gate nevertheless
**fails** on the `< 2 / 10 wrong` rule because the rule
measures audit-level truthfulness of the `no_callers` bucket,
not fixture-level truthfulness of the 17 named shapes.

**Implication for the §4.11 acceptance gate.** Hold the
"16 of 17 fixtures resolve live" contract as-is; it measures
resolver correctness on the shapes §4.11 promised. Do not
relax it. The Phase A closure is separately gated on the
`< 2 / 10 wrong` sample-draw audit (§4.11 acceptance gate
item 4); that gate remains **open** pending either (a) extractor
fixes for R39 / R41 landing in an extractor-scope PR family
or (b) universal-fidelity sprint T8 (extraction-coverage-aware
classifier downgrade) honestly mitigating the false-positive
count.

See `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`
Rounds 2 and 3 for verdicts and source-line evidence on the
original 17 fixtures below. Each FQN is a wrong verdict: the
classifier applied its rule correctly but the Apex resolver
failed to emit the edge that would have kept the symbol
`live`. These fixtures feed directly into §4.11.

From Round 2 (rev 4):

1. `GiftEntryProcessorQueueFinalizer::GiftEntryProcessorQueueFinalizer(GiftBatchId)` — cross-file constructor (`GiftEntryProcessorQueue.cls:85`).
2. `UTIL_IntegrationConfig::initCallableApi()` — intra-class call (`UTIL_IntegrationConfig.cls:72`).
3. `RD2_DataMigrationBase_BATCH.Logger::Logger(SObjectType,String,String)` — intra-file inner-class constructor (`RD2_DataMigrationBase_BATCH.cls:172`).
4. `HouseholdMembers::HouseholdMembers(List,Map)` — cross-file constructor (multiple sites).
5. `fflib_Comparator::compare(String,String)` — sibling overload dispatch.
6. `UTIL_JobProgress_CTRL.BatchJob::BatchJob(AsyncApexJob)` — intra-file inner-class constructor (`UTIL_JobProgress_CTRL.cls:129`).
7. `UTIL_Permissions::canUpdate(SObjectType)` — typed-field dispatch (BGE, GE_Template, …).

From Round 3 (rev 5):

8. `fflib_SObjectDomain.TestSObjectDisableBehaviour::TestSObjectDisableBehaviour(List)` — intra-file inner-class constructor.
9. `RD_InstallScript_BATCH::RD_InstallScript_BATCH()` — cross-file zero-arg constructor.
10. `Gift::Gift(GiftId)` — cross-file constructor (`GiftService.cls:38`).
11. `GiftBatch::GiftBatch()` — cross-file default constructor.
12. `SfdoInstrumentationService::log(...)` — typed-field dispatch (DI).
13. `Contacts::loadAccountByIdMap()` — typed-field dispatch (domain layer).
14. `CRLP_Account_AccSoftCredit_BATCH::CRLP_Account_AccSoftCredit_BATCH()` — cross-file zero-arg constructor.
15. `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)` — inner-class dispatch.
16. `UTIL_OrderBy.SortableRecord::SortableRecord(sObject,FieldExpression)` — intra-file inner-class constructor.
17. `fflib_StringBuilder.CommaDelimitedListBuilder::toString()` — implicit `toString()` in string concatenation.

Negative controls (kept to detect over-correction by the resolver):

- `RD2_DataMigrationBase_BATCH.Logger::addError(Exception,Id)` — must remain in `no_callers` after the resolver ships; genuinely unused in NPSP source.
- `fflib_IUnitOfWorkFactory::newInstance(fflib_SObjectUnitOfWork.IDML)` — must remain in `no_callers`; interface declaration, implementations carry production calls.

## 9. Reproducing this backlog

```bash
python3 - <<'PY'
import json, collections, re, os
ab = json.load(open('experiments/results/NPSP/rev5/ab_report.json'))
ann = ab.get('node_annotations', {})
buckets = collections.defaultdict(list)
for nid, a in ann.items():
    if a.get('is_test') or not a.get('is_dead'):
        continue
    r = a.get('dead_code_reason')
    if r not in ('framework_annotation_unresolved', 'dynamic_dispatch_target'):
        continue
    buckets[r].append({
        'fqn': a.get('display_name') or a.get('fqn') or nid,
        'file': re.sub(r'.*/NPSP/', '', a.get('file_path') or ''),
    })
for r, items in buckets.items():
    print(r, len(items))
    for it in sorted(items, key=lambda x: x['fqn']):
        print(' ', it['fqn'], it['file'])
PY
```

Run once per engine revision before triggering the Wave 3 hand-audit
round — the enumerated lists in §8 must match for the acceptance
gates in §4 to be meaningful.
