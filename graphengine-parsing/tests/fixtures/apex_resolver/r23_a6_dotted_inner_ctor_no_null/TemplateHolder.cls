// PR 8 diagnostic variant — same shape as
// r23_a6_dotted_inner_ctor_null_arg but caller passes a real String in
// every slot (no `null`). Isolates whether `null` in arg_types is the
// cause of the ctor drop.
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
