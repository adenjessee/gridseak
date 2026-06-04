// TR-A.1 fixture: outer method calls a sibling inner class's zero-arg
// constructor. Both classes live in the same compilation unit, so the
// resolver's §3.1 step 1 sibling-inner fast path is the load-bearing
// code path here (the call site emits bare `CallableApi::new`, which
// the registry's dotted-only lookup misses until the fast path
// rewrites it to `UTIL_IntegrationConfig.CallableApi`).
//
// §8.3 FQN covered (same-file call shape):
//   UTIL_IntegrationConfig.CallableApi::CallableApi()
public class UTIL_IntegrationConfig {
    public void initCallableApi() {
        CallableApi api = new CallableApi();
    }

    public class CallableApi {
        public CallableApi() {
        }
    }

    public class OtherHelper {
        public void noop() {
        }
    }
}
