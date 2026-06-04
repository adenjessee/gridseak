public class TemplateHolderCaller {
    public TemplateHolder.Element buildLookup() {
        String[] opts = new String[]{'x'};
        return new TemplateHolder.Element('field', 'True', null, 'CustomLabel', opts);
    }
}
