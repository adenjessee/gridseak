// TR-A.3 DI-constructor-injected fixture, declaration side. File
// pair:
//   - SfdoInstrumentationService.cls   (this file — method target)
//   - SfdoInstrumentationCaller.cls    (caller with ctor-injected field)
//
// §8.3 FQN covered:
//   SfdoInstrumentationService::log(String)
//
// The caller's field is populated through its constructor, not a
// field initialiser. Field-type-aware dispatch still binds on the
// DECLARED TYPE of the field (recorded in `ApexClassSymbols.fields`
// by the class-symbols extractor when the field declaration is
// parsed). The resolver does not need to trace the constructor's
// parameter → field assignment; it only needs the field's declared
// type. This fixture locks that contract — a DI-shaped class must
// resolve identically to a same-file field-initialiser class.
public class SfdoInstrumentationService {
    public void log(String message) {
        System.debug(message);
    }
}
