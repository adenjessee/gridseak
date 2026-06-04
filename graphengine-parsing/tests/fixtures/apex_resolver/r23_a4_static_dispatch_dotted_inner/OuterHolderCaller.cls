// R40 / TR-A.4 static-dispatch fixture — caller side, dotted form.
//
// `OuterHolder.InnerSvc.describe()` is the only call. The receiver
// `OuterHolder.InnerSvc` is dotted — pre-PR-8 it was deferred and
// dropped. After R40, `resolve_type_name_receiver` probes the
// registry with the raw dotted text, finds the inner-class entry,
// and binds to the static method on `OuterHolder.InnerSvc`.
public with sharing class OuterHolderCaller {
    public String run() {
        return OuterHolder.InnerSvc.describe();
    }
}
