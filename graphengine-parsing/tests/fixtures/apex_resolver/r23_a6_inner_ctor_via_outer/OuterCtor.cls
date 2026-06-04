// TR-A.6 fixture #1 — `new Outer.Inner(args)` from a top-level
// class.
//
// §4.11.1 shape #3 canonical: a caller outside `OuterCtor` writes
// `new OuterCtor.InnerCtor('x')` and the resolver must bind the
// edge to the inner class's declared constructor. Registry key for
// the inner class is the dotted `OuterCtor.InnerCtor`; the extractor
// already emits this form so the core registry lookup works. What
// TR-A.6 proves here is the end-to-end constructor arm still fires
// for a dotted type name.
//
// Expected: 1 Medium edge from `OuterCtorCaller::run()` to
// `OuterCtor.InnerCtor::InnerCtor(String)`.
public class OuterCtor {
    public class InnerCtor {
        public String payload;
        public InnerCtor(String s) {
            payload = s;
        }
    }
}
