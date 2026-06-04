// TR-A.2 cross-file overloaded-ctor fixture, declaration side.
// Declares two overloaded constructors differing only by the single
// argument type. The caller in the sibling `GiftBatchCaller.cls`
// picks exactly one via a literal discriminator, exercising the full
// arity → exact → widening ladder of
// `signature_matcher::rank_candidates`.
//
// §8.3 FQNs covered (overloaded-ctor shape):
//   Gift::Gift(GiftId)
//   GiftEntryProcessorQueueFinalizer::GiftEntryProcessorQueueFinalizer(GiftBatchId)
//
// Discriminator rule (PHASE_A_EXECUTION_PLAN.md §3.2): the
// disambiguating argument MUST be a literal or a `new X()` expression,
// never a bare identifier. Identifier-type inference is TR-A.3 and
// would force a Commit 1 / Commit 3 acceptance split.
public class GiftBatch {
    public GiftBatch(String name) {
    }

    public GiftBatch(Integer count) {
    }
}
