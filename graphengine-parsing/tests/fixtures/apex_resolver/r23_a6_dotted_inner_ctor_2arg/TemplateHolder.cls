// Narrowing: 2-arg dotted-inner ctor from unrelated class.
public class TemplateHolder {
    public class Element {
        public String api;
        public String required;
        public Element(String apiArg, String requiredArg) {
            api = apiArg;
            required = requiredArg;
        }
    }
}
