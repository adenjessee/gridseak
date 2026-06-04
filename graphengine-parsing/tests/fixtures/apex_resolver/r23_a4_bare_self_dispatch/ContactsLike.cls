// TR-A.4 bare-self-call fixture — `loadAccountByIdMap();` with
// no receiver must bind to the enclosing class's own method.
//
// Mirrors the real NPSP shape deferred in FOLLOWUP_RISKS.md R38:
//   public static void loadAccountByIdMap(...) { ... }
//   // elsewhere in the same class:
//   loadAccountByIdMap();
//
// TR-A.3 declined bare-receiver call sites (its grammar started
// at `this.` or a typed receiver). TR-A.4's `field_type_resolver`
// now treats `receiver_text = None` as `SelfRef { field: None }`,
// funnelling the bare call through the same enclosing-class
// lookup the explicit-`this` form uses.
//
// Expected outcome (plan §5.2 + §8.3 R38 entry):
// 1 edge to `ContactsLike::loadAccountByIdMap()`, Medium
// confidence (Exact tier, arity-0 unique).
//
// Note: the NPSP call site lives inside a property getter (`get {
// ... }`), which the current Apex extractor does NOT index as a
// Function node. Even with TR-A.4's bare-self-dispatch arm
// working perfectly (proven by this fixture's normal-method
// call), NPSP's `accountById` getter body will remain invisible
// to the resolver until property accessors are extracted as
// callable nodes. That is documented as R39 in
// FOLLOWUP_RISKS.md.
public class ContactsLike {
    public void loadAccountByIdMap() {}

    public void run() {
        loadAccountByIdMap();
    }
}
