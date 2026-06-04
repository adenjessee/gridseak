// TR-A.3 domain-layer-typed-local fixture, call side.
//
// `Contacts domain = new Contacts();` declares a local variable
// with an explicit `Contacts` type annotation. The subsequent
// `domain.loadAccountByIdMap()` call must be bound via the local
// scope table (not the enclosing-class fields path — the method
// has no `Contacts` field).
//
// This exercises the `LocalVarScope` branch of TR-A.3: the
// extractor walked the method body, recorded the local's declared
// type, and the resolver's field-type-aware dispatch binds the
// bare-identifier receiver `domain` against that local-scope
// entry.
//
// Expected outcome (plan §4.2): 1 edge to
// `Contacts::loadAccountByIdMap()`, Medium confidence.
public class ContactsDomainCaller {
    public Map<Id, Account> fetchAll() {
        Contacts domain = new Contacts();
        return domain.loadAccountByIdMap();
    }
}
