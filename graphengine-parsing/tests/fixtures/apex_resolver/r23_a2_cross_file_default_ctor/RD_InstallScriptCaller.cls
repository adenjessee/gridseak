// TR-A.2 cross-file fixture, call side. Invokes the implicit zero-arg
// default constructor of `RD_InstallScript_BATCH` (declared in the
// sibling file without any explicit ctor).
public class RD_InstallScriptCaller {
    public void bootstrap() {
        RD_InstallScript_BATCH job = new RD_InstallScript_BATCH();
    }
}
