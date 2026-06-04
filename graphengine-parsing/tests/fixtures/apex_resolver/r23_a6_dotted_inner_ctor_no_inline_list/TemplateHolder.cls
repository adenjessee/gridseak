// PR 8 diagnostic variant — passes a pre-built `String[]` local
// variable instead of an inline `new List<String>{...}`. Isolates
// whether the inline collection literal as the last arg is what
// trips the ctor drop.
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
    }
}
