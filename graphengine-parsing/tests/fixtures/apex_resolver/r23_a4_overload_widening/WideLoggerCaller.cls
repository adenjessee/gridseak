// TR-A.4 overload dispatch fixture, caller side — implicit
// Object-widening case.
//
// Local `Payload p = new Payload();` gives the resolver a typed
// local of user-defined type `Payload`. The call
// `logger.log(p)` must dispatch to `WideLogger::log(Object)` via
// the TR-A.4 Implicit tier. Expected edge: Low confidence
// (Implicit tier is structurally weaker than typed-widening).
public class WideLoggerCaller {
    private WideLogger logger;

    public WideLoggerCaller(WideLogger l) {
        this.logger = l;
    }

    public void emit() {
        Payload p = new Payload();
        logger.log(p);
    }
}
