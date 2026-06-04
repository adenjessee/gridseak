// TR-A.1 fixture: outer calls inner ctor taking `AsyncApexJob`.
//
// §8.3 FQN covered:
//   UTIL_JobProgress_CTRL.BatchJob::BatchJob(AsyncApexJob)
//
// Shape intent: the `j` arg is a bare identifier (not a literal / not
// a `new X()`), so `CallSite.arg_types` lands `Unresolved { raw: "j" }`
// at that position. `Unresolved` acts as a wildcard in the signature
// matcher — the single-arity candidate still wins via the wildcard
// path. This proves the arity filter + wildcard interaction
// (COMMIT_1_SCOPING_DECISIONS.md Q1).
public class UTIL_JobProgress_CTRL {
    public void track(AsyncApexJob j) {
        BatchJob bj = new BatchJob(j);
    }

    public class BatchJob {
        public BatchJob(AsyncApexJob j) {
        }
    }
}
