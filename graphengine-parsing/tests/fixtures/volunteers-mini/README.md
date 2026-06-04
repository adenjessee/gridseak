# volunteers-mini

A minimal structural-regression fixture carved out of the
[Volunteers-for-Salesforce](https://github.com/SFDO-Community/Volunteers-for-Salesforce)
project (Salesforce.org, BSD-3-Clause — see `LICENSE.vendored`).

Used by `tests/apex_volunteers_corpus_e2e.rs` to pin the structural
invariants that the Apex pipeline guarantees on real SFDX source:

- `__file_module__` invariant: every `.cls` and `.trigger` file produces
  exactly one `Module` node.
- `apex_sharing` propagation: outer class sharing decorators reach inner
  classes (Sprint E.5).
- Inheritance edges: `Extends`/`Implements` are first-class edge kinds
  (Sprint E.1), including outer classes whose inner classes extend
  built-in types like `Exception`.
- Trigger synthesis: `.trigger` bodies become synthetic `__trigger__`
  Function nodes whose parents carry `trigger_events` (Sprint E.3/E.4).
- Same-class call preference (Sprint H.1): methods declared on the
  caller's own class take precedence over cross-class name matches.
- External module invariant (Sprint G.1 dedup): Volunteers has no
  managed-package namespaces, so total `Module` count must equal
  unique file count exactly.

## Files vendored

| File | Structural value |
|---|---|
| `classes/UTIL_Describe.cls` | `with sharing` outer + inner `PermsException extends Exception` and `SchemaDescribeException extends Exception` — exercises sharing inheritance (E.5) and Extends-to-built-in handling. |
| `classes/VOL_SharedCode.cls` | Shared utility class used by the other vendored classes; this is the cross-class fanout destination that H.1 same-class preference filters out. |
| `classes/VOL_CTRL_VolunteersFind.cls` | Controller with extensive intra-class and cross-class call sites; exercises the H.1 resolver path. |
| `triggers/VOL_Campaign_CreateStatuses.trigger` | Real trigger covering E.3 (synthetic `__trigger__` Function) and E.4 (`trigger_events`). |

Each file carries its Salesforce `*-meta.xml` metadata sibling because
the SFDX layout detection reads api-version metadata to verify project
shape.

## Do not modify

If a test-expected invariant changes, update the test's assertion
rather than the fixture — the fixture is vendored upstream content and
must stay byte-identical to what a customer would clone.
