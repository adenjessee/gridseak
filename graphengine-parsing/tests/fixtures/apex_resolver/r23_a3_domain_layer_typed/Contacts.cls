// TR-A.3 domain-layer-typed-local fixture, declaration side. File
// pair:
//   - Contacts.cls                  (this file — method target)
//   - ContactsDomainCaller.cls      (caller with typed local var)
//
// §8.3 FQN covered:
//   Contacts::loadAccountByIdMap()
//
// Mirrors the classic fflib / SFDO domain-layer pattern: a method
// returning a typed `Contacts` domain object, called via a local
// variable declared with the same `Contacts` type. TR-A.3's local-
// scope branch must record the local declaration's declared type
// and bind the receiver `domain` on the method call to it.
public class Contacts {
    public Map<Id, Account> loadAccountByIdMap() {
        return new Map<Id, Account>();
    }
}
