// TR-A.2 cross-file overloaded-ctor fixture, call side. Uses an
// integer literal `42` as the discriminator — the arg-type inferrer
// resolves it to `Primitive("Integer")`, and the signature matcher
// lands on the `GiftBatch(Integer)` ctor (not `GiftBatch(String)`)
// via the exact-match tier.
public class GiftBatchCaller {
    public void process() {
        GiftBatch gb = new GiftBatch(42);
    }
}
