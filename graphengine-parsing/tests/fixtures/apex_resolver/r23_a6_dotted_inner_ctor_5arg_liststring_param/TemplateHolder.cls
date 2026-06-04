// Same as 5arg_all_strings but last param is `List<String>` instead of `String[]`.
public class TemplateHolder {
    public class Element {
        public String api; public String required; public String dataType; public String label;
        public List<String> opts;
        public Element(String a1, String a2, String a3, String a4, List<String> a5) {
            api = a1; required = a2; dataType = a3; label = a4; opts = a5;
        }
    }
}
