// TR-A.6 fixture #4 — NPSP canonical shape:
// `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set<Id>)`.
//
// §4.11.1 shape #1 (canonical). Mirrors the exact declarative
// structure of `CAM_CascadeDeleteLookups_TDTM` in NPSP: an
// `*_TDTM.cls` defines the cascade-delete entry via a field of an
// inner `CascadeDeleteLoader` type which overrides the parent
// `load(Set<Id>)` method. This is one of the 48 FQNs on the
// §4.11.1 revert-population acceptance table; PR 5's gate is
// >=42/48 resolving live.
//
// Expected: 1 Medium edge from `CAM_CascadeDeleteLookups_TDTM::run()`
// to `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set<Id>)`.
public class CAM_CascadeDeleteLookups_TDTM {
    public virtual class CascadeDeleteLoaderBase {
        public virtual void load(Set<Id> ids) {}
    }

    public class CascadeDeleteLoader extends CascadeDeleteLoaderBase {
        public override void load(Set<Id> ids) {}
    }

    private CascadeDeleteLoader loader = new CascadeDeleteLoader();

    public void run(Set<Id> ids) {
        loader.load(ids);
    }
}
