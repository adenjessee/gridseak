// TR-A.6 fixture #2 — Outer holds a typed field of Inner type,
// calls an inner method on it.
//
// §4.11.1 shape #1: the inner class's short name is used in the
// field declaration inside the outer class (Apex allows unqualified
// inner-class references from the outer); the resolver must see
// `Inner` as `OuterField.Inner` in the registry. This is the core
// TR-A.6 wiring: a short-name field type that only hits the registry
// under its dotted key.
//
// Expected: 1 Medium edge from `OuterField::run()` to
// `OuterField.Inner::ping()`.
public class OuterField {
    public class Inner {
        public String ping() {
            return 'pong';
        }
    }

    private Inner inner = new Inner();

    public String run() {
        return inner.ping();
    }
}
