# WS-LAYER2-JAVA — Java Layer-2 adapter

> **Kind.** Design stub.
> **Parent.** [`README.md`](README.md) (WS-LAYER2-ADAPTERS index).
> **Status.** Unstaffed *on the Layer-2 axis*. The Java heuristic
> extractor ships today
> (`graphengine-parsing/src/syntax/language/extractors/java.rs`).
> Scans on Java repos produce a full call graph and every
> graph-level metric; this stub tracks the unshipped
> semantic-confirmation layer. Per-edge precision on heuristic-only
> Java is unmeasured — there are no dedicated `java*.rs` integration
> tests yet (the ground-truth gap noted in
> `TRUST_AND_ACCURACY_MEMO.md §4`).
> **Target resolver.** [eclipse.jdt.ls](https://github.com/eclipse-jdtls/eclipse.jdt.ls) — the Eclipse JDT Language Server, industry standard for Java LSP.

## Why eclipse.jdt.ls

- It wraps Eclipse JDT, the battle-tested Java compiler and
  analyser that backs most enterprise Java IDEs.
- Alternatives: **IntelliJ IDEA's backend** (closed-source,
  non-starter); **javac-based one-off tools** (reinvention,
  explicit anti-goal). jdt.ls is the only viable choice.

## Expected install friction

- Requires JDK 17+ (Temurin or Corretto). Same JDK constraint
  that gated the Apex LSP (Jorje) work — the infrastructure
  carries over.
- jdt.ls is distributed as a tarball from the Eclipse downloads
  site; the adapter's install step is `scripts/download_jdt_ls.sh`
  on the pattern of `scripts/download_apex_jorje.sh`. Pin the
  SHA256 for reproducibility.
- The customer's build system (Maven / Gradle / Bazel) must be
  detectable — jdt.ls needs the classpath to resolve. Maven
  `pom.xml` and Gradle `build.gradle` auto-detected; Bazel
  projects require explicit classpath input.

## Adapter contract specifics

- Must resolve `textDocument/definition` on call sites.
- Must handle **Java generics dispatch** — a call on a
  `List<? extends Foo>` receiver should resolve to the erased
  target method; the adapter must match jdt.ls's resolution and
  not attempt a more-specific inference.
- Must handle **method references** (`SomeClass::someMethod`)
  and **lambdas** — both produce call edges, both should be
  Layer-2-confirmed when jdt.ls can resolve.
- Must handle **annotation-processor-generated code** —
  `target/generated-sources/*.java` files must be included in
  the corpus walk if present; jdt.ls resolves through them.

## Coverage-gap counterparts (Axis 2)

- **Reflection (`Class.forName`, `Method.invoke`)** —
  `CoverageGap::DynamicReflection`.
- **Annotation-processor bodies** where the processor is user-
  defined and generates code we haven't seen yet (i.e. pre-
  build scans).
- **`@Autowired` / `@Component` / `@Bean` dependency wiring**
  — these are framework-dispatch shapes. A Spring framework
  resolver (Axis 3) subsumes this; without it, emit
  `framework_annotation_unresolved`.
- **`sealed` class dispatch** (Java 17+) — jdt.ls handles, but
  we should confirm our adapter surfaces all permitted subclasses.
- **Record synthetic accessors** — `record Foo(int x) { }`
  generates a synthetic `x()` accessor. jdt.ls handles; we
  should sanity-check.

## Readiness strategy

jdt.ls is the slowest of the five proposed adapters (the
initial workspace-index is observably slow on large
enterprise monorepos):

1. Start jdt.ls.
2. Wait for the `$/progress "Importing Maven projects"` /
   equivalent completion message.
3. Probe `definition` on a canary symbol; retry with
   exponential backoff if `IndexIncomplete`.

Timeout: 300 s (5 minutes) on a 100k-file enterprise Java
monorepo.

## Benchmarks to target

- **Accuracy.** At least 50 % Layer-2-confirmed call edges on a
  Maven Java project with normal dependency declarations.
- **Cold-start.** < 5 minutes on 100k-file monorepo.
- **Warm-path.** < 2 s per file incremental.

## Out of scope (v0)

- **Kotlin / Scala.** Different LSPs; different adapter stubs.
- **Java 8.** jdt.ls supports but deprecated; v0 pins Java 11+.
- **Android projects.** Differently shaped classpath resolution
  (R8 / D8 / resource IDs); separate post-v0 workstream.

## Overlooked risks

- **Enterprise monorepo cold-start is brutal.** jdt.ls on a
  100k+ file monorepo can take 5–15 minutes the first run. The
  adapter's `ReadinessStrategy` must not silently time out — it
  should emit periodic progress to the customer-facing log so
  they know the scan is proceeding, not hung.
- **Classpath ambiguity.** A Java project with transitive
  dependency conflicts (two versions of the same jar) will
  produce slightly different resolution depending on which
  version jdt.ls picks. Record the resolved classpath in the
  scan metadata.
- **Annotation-processor-generated code drift.** If the customer
  generates code at build time, a scan before build sees
  different code than a scan after build. Adapter must document
  which mode the scan was run in, and the operator should
  default to "after-build" state for customer scans.
