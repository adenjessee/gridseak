// PR 8 diagnostic fixture — mirrors NPSP's
// `new GE_Template.Element('field', 'True', null, 'CustomLabel',
//                          new List<String>{...})` call shape.
//
// Shape:
//  * outer top-level class `TemplateHolder`
//  * inner class `Element` with two ctor overloads
//    - `Element(String, String, String, String, String[])`
//    - `Element(String, String, String, String[])`
//  * caller in an unrelated class writes the 5-arg form with a `null`
//    at the third position.
//
// If the resolver correctly binds the ctor call to the 5-arg overload,
// we can rule out "dotted inner ctor cross-file + null-arg signature
// matching" as the root cause of NPSP Sample 8. If it does not, this
// fixture is the minimal reproducer.
public class TemplateHolder {
    public class Element {
        public String api;
        public String required;
        public String dataType;
        public String label;
        public String[] options;

        public Element(String apiArg, String requiredArg, String dataTypeArg,
                       String labelArg, String[] optionsArg) {
            api = apiArg;
            required = requiredArg;
            dataType = dataTypeArg;
            label = labelArg;
            options = optionsArg;
        }

        public Element(String apiArg, String requiredArg, String labelArg,
                       String[] optionsArg) {
            api = apiArg;
            required = requiredArg;
            label = labelArg;
            options = optionsArg;
        }
    }
}
