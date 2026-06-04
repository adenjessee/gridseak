// R40 / TR-A.4 static-dispatch fixture — caller side.
//
// `useAdv` is called as a bare `ClassName.method()` expression with
// no surrounding chain. The pre-PR-8 resolver classified the
// receiver as `BareIdent("ADV_PackageInfo_SVC_Like")`, looked it up
// as a local var (none) and as an instance field (none), and then
// declined — no edge emitted. The R40 fix adds a final TypeName
// probe against `ApexClassRegistry` so the receiver resolves to
// `ApexTypeRef::UserDefined { api_name: "ADV_PackageInfo_SVC_Like" }`,
// at which point the existing `pair_methods_with_nodes` arm picks
// the correct static method.
//
// `getApiNPSP()` is the second receiver site; it appears inside a
// short-circuit conjunction, identical NPSP shape (line 99 of
// `TDTM_ObjectDataGateway.cls`).
public with sharing class ADV_PackageInfo_SVC_Like_Caller {
    public Boolean run() {
        return ADV_PackageInfo_SVC_Like.useAdv()
            && ADV_PackageInfo_SVC_Like.getApiNPSP() != null;
    }
}
