// TR-A.2 cross-file fixture, declaration side, with NO explicit
// constructor. Exercises the implicit default-ctor path in the
// resolver: a zero-arg `new RD_InstallScript_BATCH()` call site must
// land a Medium-confidence edge onto the class's `Struct` node.
//
// No `Function` node exists for an implicit default ctor, so the
// resolver short-circuits before `rank_candidates` even runs and
// emits the edge directly against the class type (see
// `constructor_resolver.rs` § implicit-default-ctor arm).
//
// §8.3 FQN covered:
//   RD_InstallScript_BATCH::RD_InstallScript_BATCH()
public class RD_InstallScript_BATCH {
    public void onInstall() {
    }
}
