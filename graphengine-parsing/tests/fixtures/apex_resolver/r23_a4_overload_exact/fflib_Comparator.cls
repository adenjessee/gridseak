// TR-A.4 overload dispatch fixture — exact match wins over widening.
// File pair:
//   - fflib_Comparator.cls            (this file — two overloads)
//   - fflib_ComparatorCaller.cls      (caller with String literals)
//
// §8.3 FQN covered:
//   fflib_Comparator::compare(String, String)
//
// The caller invokes `cmp.compare('a', 'b')` with two String
// literals. The signature matcher must prefer the exact-match
// overload `compare(String, String)` over the Object-accepting
// overload `compare(Object, Object)`, per TR-A.4's Exact-beats-
// Widening ordering (and the new Implicit-tier ordering below
// widening).
//
// Expected outcome (plan §5.2): 1 edge to
// `fflib_Comparator::compare(String, String)`, Medium confidence.
public class fflib_Comparator {
    public Integer compare(String a, String b) {
        return a == b ? 0 : -1;
    }

    public Integer compare(Object a, Object b) {
        return a == b ? 0 : -1;
    }
}
