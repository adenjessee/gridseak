// TR-A.6 fixture #5 — NPSP canonical shape (implicit default ctor):
// `RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader::FirstCascadeUndeleteLoader()`.
//
// §4.11.1 shape #3 (canonical). The inner class `FirstCascadeUndeleteLoader`
// declares no constructor; the outer class instantiates it with
// `new FirstCascadeUndeleteLoader()`. The resolver's
// implicit-default-ctor fallback (constructor_resolver.rs:149-160)
// must fire on the dotted `Outer.Inner` target key so the edge
// lands on the inner type node. This is the second of two NPSP
// revert-population exemplars explicitly called out in
// PHASE_A_EXECUTION_PLAN §6.2 for the PR 5 acceptance table.
//
// Expected: 1 Medium edge from `RD_CascadeDeleteLookups_TDTM::run()`
// to the `FirstCascadeUndeleteLoader` type node (Struct) under
// the implicit-default-ctor fallback, since no ctor is declared.
public class RD_CascadeDeleteLookups_TDTM {
    public class FirstCascadeUndeleteLoader {
        public void noop() {}
    }

    public FirstCascadeUndeleteLoader run() {
        return new FirstCascadeUndeleteLoader();
    }
}
