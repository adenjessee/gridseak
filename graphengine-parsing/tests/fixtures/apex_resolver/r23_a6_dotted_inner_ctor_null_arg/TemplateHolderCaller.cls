// PR 8 diagnostic fixture — unrelated class instantiates
// `new TemplateHolder.Element('field', 'True', null, 'CustomLabel',
//                              new List<String>{'x'})`.
public class TemplateHolderCaller {
    public TemplateHolder.Element buildLookup() {
        return new TemplateHolder.Element('field', 'True', null, 'CustomLabel',
            new List<String>{'x'});
    }
}
