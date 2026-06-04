// TR-A.3 DI-constructor-injected fixture, call side.
//
// Field `instrumentation` is declared with type
// `SfdoInstrumentationService` and populated by a constructor
// parameter (classic Salesforce DI idiom: interfaces / services get
// passed in so tests can substitute stubs). The invocation
// `instrumentation.log('...')` lives inside an instance method;
// TR-A.3 must bind the receiver `instrumentation` to its declared
// type via the field table and resolve `log(String)` on
// `SfdoInstrumentationService`.
//
// Expected outcome (plan §4.2): 1 edge to
// `SfdoInstrumentationService::log(String)`, Medium confidence.
public class SfdoInstrumentationCaller {
    private SfdoInstrumentationService instrumentation;

    public SfdoInstrumentationCaller(SfdoInstrumentationService injected) {
        this.instrumentation = injected;
    }

    public void emit() {
        instrumentation.log('hello');
    }
}
