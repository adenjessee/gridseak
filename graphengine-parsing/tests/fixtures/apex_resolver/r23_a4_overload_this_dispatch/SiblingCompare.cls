// TR-A.4 this-dispatch fixture — `this.compare(s1, s2)` picks
// the String-accepting sibling overload.
//
// Single-file fixture. A class carries two overloads,
// `compare(String, String)` and `compare(Object, Object)`. From
// an unrelated method, `this.compare(s1, s2)` is called with two
// String-typed locals. TR-A.3's SelfRef arm normalises the
// receiver to the enclosing class; TR-A.4's signature matcher
// then disambiguates the two overloads using arg types.
//
// Expected outcome (plan §5.2): 1 edge to
// `SiblingCompare::compare(String, String)`, Medium confidence
// (Exact tier wins).
public class SiblingCompare {
    public Integer compare(String a, String b) {
        return a == b ? 0 : -1;
    }

    public Integer compare(Object a, Object b) {
        return a == b ? 0 : -1;
    }

    public Integer dispatch() {
        String s1 = 'a';
        String s2 = 'b';
        return this.compare(s1, s2);
    }
}
