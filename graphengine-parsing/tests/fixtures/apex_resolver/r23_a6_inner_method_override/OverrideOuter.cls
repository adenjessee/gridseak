// TR-A.6 fixture #3 (cont'd) — inner class overrides BaseLoader.
// Outer holds a typed field of the inner type; the call through
// the field must bind to the override on `ChildLoader`, not to
// the base declaration on `BaseLoader`.
public class OverrideOuter {
    public class ChildLoader extends BaseLoader {
        public override void load(Set<Id> ids) {}
    }

    private ChildLoader loader = new ChildLoader();

    public void kick(Set<Id> ids) {
        loader.load(ids);
    }
}
