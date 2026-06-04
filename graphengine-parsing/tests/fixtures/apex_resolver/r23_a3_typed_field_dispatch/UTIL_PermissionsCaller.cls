// TR-A.3 typed-field dispatch fixture, call side. Caller for
// `UTIL_Permissions.cls`. The receiver of the method call is a
// bare-identifier reference (`permissionsService`) to a
// private field whose declared type is `UTIL_Permissions`. The
// field-type-aware dispatch resolver must:
//
//   1. Strip the `permissionsService.canUpdate` call site's
//      receiver.
//   2. Find `permissionsService` in this class's fields table,
//      pick up the declared `UTIL_Permissions` type.
//   3. Look up `canUpdate` on `UTIL_Permissions` via the registry.
//
// Expected outcome (plan §4.2): 1 edge to
// `UTIL_Permissions::canUpdate(SObjectType)`, Medium confidence.
public class UTIL_PermissionsCaller {
    private UTIL_Permissions permissionsService;

    public UTIL_PermissionsCaller() {
        this.permissionsService = new UTIL_Permissions();
    }

    public Boolean checkAccountUpdate() {
        return permissionsService.canUpdate(Account.SObjectType);
    }
}
