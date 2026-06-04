// TR-A.4 overload dispatch fixture, caller side — exact-match case.
//
// The field `cmp` is typed `fflib_Comparator`. The call site
// `cmp.compare('a', 'b')` carries two String-literal args whose
// inferred `ApexTypeRef` is `Primitive("String")`. The resolver
// must pick the exact String-accepting overload over the
// Object-accepting one.
public class fflib_ComparatorCaller {
    private fflib_Comparator cmp;

    public fflib_ComparatorCaller(fflib_Comparator c) {
        this.cmp = c;
    }

    public Integer run() {
        return cmp.compare('a', 'b');
    }
}
