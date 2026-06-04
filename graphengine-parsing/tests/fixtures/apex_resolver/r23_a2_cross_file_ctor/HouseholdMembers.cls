// TR-A.2 cross-file fixture, declaration side. File pair:
//   - HouseholdMembers.cls          (this file — the ctor target)
//   - HouseholdMembersCaller.cls    (the caller in the same directory)
//
// §8.3 FQN covered:
//   HouseholdMembers::HouseholdMembers(List, Map)
//
// The caller lives in a different class entirely, so the resolver's
// sibling-inner fast path (§3.1 step 1) is NOT the path exercised
// here — the TR-A.2 registry-wide case-insensitive lookup (§3.1
// step 2) is. That makes this file pair the canonical cross-file
// ctor-binding fixture.
public class HouseholdMembers {
    private List<Id> ids;
    private Map<Id, Account> accounts;

    public HouseholdMembers(List<Id> ids, Map<Id, Account> accounts) {
        this.ids = ids;
        this.accounts = accounts;
    }
}
