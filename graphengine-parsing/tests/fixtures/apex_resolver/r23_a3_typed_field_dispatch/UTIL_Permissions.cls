// TR-A.3 typed-field dispatch fixture, declaration side. File pair:
//   - UTIL_Permissions.cls             (this file — method target)
//   - UTIL_PermissionsCaller.cls       (caller with typed field)
//
// §8.3 FQN covered:
//   UTIL_Permissions::canUpdate(SObjectType)
//
// The caller declares `private UTIL_Permissions permissionsService;`
// and invokes `permissionsService.canUpdate(Account.SObjectType)`.
// TR-A.3's field-type-aware dispatch must bind the bare-identifier
// receiver `permissionsService` to its declared `UTIL_Permissions`
// type by walking the enclosing class's `fields` table, then look up
// `canUpdate` on that class. This file carries the target.
public class UTIL_Permissions {
    public Boolean canUpdate(SObjectType token) {
        return token != null;
    }
}
