# Apex Corpus Fixture

Minimal SFDX-shaped Apex project used by:

- `graphengine-parsing/tests/apex_query_validation.rs` — validates each
  tree-sitter query in `configs/apex.yaml` captures the expected tokens
  against real Apex source.
- `graphengine-parsing/tests/apex_heuristic_corpus.rs` — end-to-end
  `ApexHeuristicResolver` smoke tests driven off the same corpus.

The fixture is intentionally small but **covers every feature the Apex
integration currently cares about**:

- `with sharing`, `without sharing`, `inherited sharing`, omitted sharing.
- `@AuraEnabled`, `@AuraEnabled(cacheable=true)`, `@InvocableMethod`,
  `@IsTest`, and the legacy `testMethod` keyword.
- `global class ... implements Database.Batchable<SObject>, Schedulable`.
- SOQL (`[SELECT ...]`) and SOSL (`[FIND :term IN ALL FIELDS ...]`).
- Managed-package references via both `__` suffix (`npsp__Household__c`)
  and dotted qualified names (`pi.FormHelper`).
- A `trigger` on `Account` with multiple events and cross-class calls,
  including every `Trigger.*` context-variable shape the resolver must
  filter out.
- Per-class `.cls-meta.xml` and a minimal `object-meta.xml` so the
  metadata readers have realistic input.

The fixture deliberately does **not** depend on the apex-jorje LSP — it
is pure source + metadata so the heuristic path can be validated in CI
without requiring Java or the Salesforce LSP jar.
