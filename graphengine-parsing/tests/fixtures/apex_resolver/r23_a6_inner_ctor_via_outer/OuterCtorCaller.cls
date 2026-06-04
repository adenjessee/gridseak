public class OuterCtorCaller {
    public OuterCtor.InnerCtor run() {
        return new OuterCtor.InnerCtor('hi');
    }
}
