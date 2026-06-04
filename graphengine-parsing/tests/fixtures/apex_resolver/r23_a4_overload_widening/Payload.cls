// TR-A.4 overload dispatch fixture — user-defined payload class.
// Kept isolated so the local-var inference in WideLoggerCaller
// binds `Payload p` to a user-declared registry entry rather than
// an arity-only fallback.
public class Payload {}
