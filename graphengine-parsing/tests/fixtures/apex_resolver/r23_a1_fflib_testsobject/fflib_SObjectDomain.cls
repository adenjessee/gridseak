// TR-A.1 fixture: inner-class ctor taking a `List<SObject>` argument.
//
// §8.3 FQN covered:
//   fflib_SObjectDomain.TestSObjectDisableBehaviour::TestSObjectDisableBehaviour(List)
//
// Shape intent: `new List<Account>()` inferred by the argument-type
// inferrer as `Collection { kind: List, element: Sobject("Account") }`.
// The ctor takes `List<Account>`, so the matcher lands on exact
// collection-element match. This fixture is the canonical List-shaped
// ctor that the fflib library surfaces throughout NPSP.
public class fflib_SObjectDomain {
    public void register() {
        TestSObjectDisableBehaviour t = new TestSObjectDisableBehaviour(new List<Account>());
    }

    public class TestSObjectDisableBehaviour {
        public TestSObjectDisableBehaviour(List<Account> records) {
        }
    }
}
