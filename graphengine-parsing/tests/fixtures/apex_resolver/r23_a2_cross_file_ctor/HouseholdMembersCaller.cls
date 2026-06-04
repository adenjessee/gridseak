// TR-A.2 cross-file fixture, call side. Caller for
// `HouseholdMembers.cls`. Instantiates `HouseholdMembers` with a
// `List<Id>` and a `Map<Id, Account>`, both constructed in-place so
// the argument-type inferrer resolves each argument to its structured
// type without needing a local-variable scope table (which is Phase
// B's TR-A.3 territory).
public class HouseholdMembersCaller {
    public void run() {
        HouseholdMembers hm = new HouseholdMembers(
            new List<Id>(),
            new Map<Id, Account>()
        );
    }
}
