// TR-A.4 overload dispatch fixture — implicit conversion picks
// Object overload.
//
// Expected outcome (plan §5.2): 1 edge, Low confidence.
public class WideLogger {
    public void log(String s) {}
    public void log(Object o) {}
}
