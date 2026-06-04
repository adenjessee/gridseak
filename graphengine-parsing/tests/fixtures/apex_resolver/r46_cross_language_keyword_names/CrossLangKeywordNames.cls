/**
 * R46 regression fixture — Apex methods whose names are reserved in a
 * DIFFERENT language. The R46 bug was a language-blind keyword filter
 * that dropped these at extraction because:
 *  - `match`  is a Rust keyword
 *  - `type`   is a TypeScript contextual keyword
 *  - `lambda` is a Python keyword
 *  - `in`     is a C# keyword
 *  - `of`     is (was) listed as a TS keyword
 *  - `object` is a C# keyword
 *  - `default` is a TS keyword
 *  - `where`  is a Rust / C# contextual keyword
 * None of these are reserved in Apex. All must be extracted as Function
 * nodes and their call sites must resolve back to them.
 */
public class CrossLangKeywordNames {
    public void callAll() {
        match();
        type();
        lambda();
        in();
        of();
        object();
        where();
    }

    public void match() {
        System.debug('match called');
    }

    public void type() {
        System.debug('type called');
    }

    public void lambda() {
        System.debug('lambda called');
    }

    public void in() {
        System.debug('in called');
    }

    public void of() {
        System.debug('of called');
    }

    public void object() {
        System.debug('object called');
    }

    public void where() {
        System.debug('where called');
    }
}
