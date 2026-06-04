// R40 / TR-A.4 static-dispatch fixture — `ClassName.staticMethod()`
// shape, the actual call form used by NPSP at
// `TDTM_ObjectDataGateway` line 99-100:
//
//     ADV_PackageInfo_SVC.useAdv() && ADV_PackageInfo_SVC.getApiNPSP() ...
//
// The NPSP rev-7 Round-5 audit recorded 0 inbound Call edges on
// `ADV_PackageInfo_SVC::getApiNPSP()`, classifying it `no_callers`
// even though the method is invoked twice from
// `TDTM_ObjectDataGateway`. Root cause: pre-PR-8 the heuristic
// resolver had no static-dispatch arm — the receiver `ADV_PackageInfo_SVC`
// was a `BareIdent` that was neither a local var nor an instance
// field, and the resolver gave up rather than probing the
// `ApexClassRegistry` for a TypeName match. Risk catalogued as
// R40 in `FOLLOWUP_RISKS.md`.
//
// File pair:
//   - ADV_PackageInfo_SVC_Like.cls       (this file — static method holder)
//   - ADV_PackageInfo_SVC_Like_Caller.cls (caller — `Class.method()`)
//
// §8.3 FQN exercised:
//   ADV_PackageInfo_SVC_Like::getApiNPSP()
//   ADV_PackageInfo_SVC_Like::useAdv()
//
// Expected outcome: 1 heuristic Call edge per receiver site,
// landing on the corresponding static method's Function node.
// Confidence: Medium (Exact tier; arity-0 unique).
public with sharing class ADV_PackageInfo_SVC_Like {
    public static Boolean useAdv() {
        return false;
    }

    public static Object getApiNPSP() {
        return null;
    }
}
