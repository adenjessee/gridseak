// R40 / TR-A.4 static-dispatch fixture — `Outer.Inner.staticMethod()`
// shape (the dotted-receiver companion to the bare-receiver
// fixture).
//
// Pre-PR-8 the `normalise_receiver` arm classified `Outer.Inner`
// as `NormalisedReceiver::DottedDefer` (no payload) and
// auto-deferred all dotted receivers to TR-A.6, but TR-A.6 only
// covers constructors and instance-method dispatch through
// declared field types — it never resolved a dotted receiver as a
// type name. So inner-class static methods were silently dropped
// even when the dotted name matched a registry entry.
//
// The R40 fix changes `DottedDefer` to carry the raw text and
// adds a TypeName probe against the `ApexClassRegistry` (which
// keys inner classes as `Outer.Inner`).
//
// File pair:
//   - OuterHolder.cls         (this file — outer + inner with static)
//   - OuterHolderCaller.cls   (caller — `Outer.Inner.method()`)
public with sharing class OuterHolder {
    public class InnerSvc {
        public static String describe() {
            return 'OuterHolder.InnerSvc';
        }
    }
}
