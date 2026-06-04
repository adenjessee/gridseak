# Edge Taxonomy — planned vs shipped

**Status.** Authoritative enumeration of every `EdgeKind` variant, past, present, and planned. The invariant this file enforces is:

> Code (`graphengine-parsing/src/domain/edge.rs` + the analysis-side mirror) ships **only variants with live emitters**. Markdown (this file) enumerates the planned taxonomy a sprint into the future, so "someone thought about the shape" and "an emitter runs in production" are two different checkboxes.

Adding a planned variant to the table below does NOT add it to `EdgeKind`. The two events are coupled in the opposite direction: adding a variant to `EdgeKind` requires flipping "ships in code?" to Yes here in the same PR as the emitter lands.

See [`docs/workstreams/universal-fidelity/NEW_ENGINEER_PRIMER.md`](../workstreams/universal-fidelity/NEW_ENGINEER_PRIMER.md) §8 Decision 6 for the reasoning.

## Legend

- **Variant**: fully-qualified Rust path as it would appear (or does appear) under `graphengine_parsing::domain::edge::EdgeKind`.
- **Family**: which parent variant this sub-enum belongs to (`Framework` or `Declarative`), or "core" for top-level unit variants.
- **Emitter exists?**: Is there a call site in the codebase that produces this variant today?
- **Tests exist?**: Is there an integration test or fixture asserting this variant is emitted for a realistic input?
- **Ships in code?**: Is the variant currently present in the `EdgeKind` enum? (Yes = live; No = planned only.)
- **Notes**: short rationale or blocking dependency.

## Core variants (top-level unit)

| Variant | Emitter exists? | Tests exist? | Ships in code? | Notes |
| --- | --- | --- | --- | --- |
| `EdgeKind::Call` | Yes | Yes | Yes | Normal function-call invocation. Produced by every resolver. |
| `EdgeKind::Contains` | Yes | Yes | Yes | Structural parenthood (module→function, file→class). |
| `EdgeKind::Import` | Yes | Yes | Yes | `import`/`use` statement resolution. |
| `EdgeKind::Extends` | Yes | Yes | Yes | Class-inheritance clause. |
| `EdgeKind::Implements` | Yes | Yes | Yes | Interface-implementation clause. |
| `EdgeKind::Type` | Yes | Yes | Yes | Type reference in signature / field / generic. |
| `EdgeKind::Uses` | Yes | Yes | Yes | Identifier reference that does not fit other buckets. |

## `EdgeKind::Framework(FrameworkKind)` — source-bound framework dispatch

Framework edges are dispatches invoked by a framework runtime at a declarative binding site *inside source code* (a template attribute, a component binding, an annotation). Distinct from `Declarative` because a source-file parser can see the binding.

| Variant | Emitter exists? | Tests exist? | Ships in code? | Notes |
| --- | --- | --- | --- | --- |
| `FrameworkKind::VisualforcePage` | Yes | Yes (`apex_resolver_r23_a5_vf_fixtures.rs`) | Yes | `{!expr}` binding on a `.page` to an Apex controller method. |
| `FrameworkKind::LwcTemplate` | No | No | No | LWC `.html` `@wire` / `@api` / event handler → JS → `@AuraEnabled` Apex method. Emitter planned post-T1. |
| `FrameworkKind::AuraComponent` | No | No | No | Aura `.cmp` / `.app` binding → Apex `@AuraEnabled`. Pre-LWC, still widespread. Emitter planned post-T1. |
| `FrameworkKind::Trigger` | No | No | No | Apex `trigger` body event binding → Apex dispatch target. Today synthesised via `apex_trigger_body_synthesis_e2e`, but as `Call` edges. Promotion to `Framework(Trigger)` requires updating the synthesis stage emitter. |
| `FrameworkKind::InboundEmail` | No | No | No | Apex `Messaging.InboundEmailHandler` class registered via Email Services. Framework dispatches on inbound email receipt. Emitter planned. |

## `EdgeKind::Declarative(DeclarativeKind)` — out-of-source declarative wiring

Declarative edges are dispatches whose binding lives entirely in non-source artifacts: XML metadata, JSON configuration, platform-managed no-code tools. A source-file parser cannot see the binding; a separate reader (Flow XML, Spring XML, etc.) produces the edge.

| Variant | Emitter exists? | Tests exist? | Ships in code? | Notes |
| --- | --- | --- | --- | --- |
| `DeclarativeKind::Flow` | No (placeholder) | No | Yes | Salesforce Flow XML → Apex Invocable method. Variant ships as the canonical placeholder so the sub-enum is non-uninhabited and downstream match arms compile. Emitter scoped in `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md`. |
| `DeclarativeKind::ProcessBuilder` | No | No | No | Salesforce Process Builder → Apex. Deprecated platform-side but still present in legacy orgs; reader is lower priority than Flow. |
| `DeclarativeKind::WorkflowRule` | No | No | No | Salesforce Workflow Rule → Apex. Same status as ProcessBuilder. |
| `DeclarativeKind::SpringXml` | No | No | No | Spring bean XML → Java bean method. Non-Salesforce but same shape; reader planned when Java Layer-2 work starts. |
| `DeclarativeKind::DjangoUrlconf` | No | No | No | Django `urls.py` string-routed view invocation. Python reader planned. |
| `DeclarativeKind::RailsRoutes` | No | No | No | Ruby on Rails `routes.rb` DSL → controller method. Ruby reader planned. |

## Adding a variant — the checklist

Before you submit a PR that changes this file:

1. Does the emitter land in the same PR? If no, stop — this file is not the place for the variant yet; it is already listed above as planned.
2. Are at least one unit test and one integration test landing in the same PR asserting the emitter produces the variant on a realistic input?
3. Does the variant's round-trip wire string land as a pinned literal in `graphengine-parsing/tests/t1_edgekind_roundtrip.rs`?
4. Have you read `NEW_ENGINEER_PRIMER.md` §8.0 (the diagnostic questions) and answered them for the variant's introduction? A new variant usually forces at least a predicate update (§8 Decision 5) — confirm `is_call_like` / `is_dependency` etc still classify it correctly.

If all four are Yes, flip "Emitter exists?", "Tests exist?", and "Ships in code?" to Yes in this file in the same PR.
