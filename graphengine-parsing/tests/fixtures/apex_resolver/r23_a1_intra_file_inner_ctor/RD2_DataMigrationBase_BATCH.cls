// TR-A.1 fixture: outer method calls sibling inner's 3-arg ctor.
//
// §8.3 FQN covered:
//   RD2_DataMigrationBase_BATCH.Logger::Logger(SObjectType, String, String)
//
// Shape intent: `Account.SObjectType` → Sobject("Account"), two string
// literals → Primitive("String"). The only candidate ctor has arity 3
// with matching types, so the ladder lands on exact match and emits a
// single Medium-confidence edge.
//
// Hosting directory name (`r23_a1_intra_file_inner_ctor`) preserves
// the plan's fixture identifier; Apex requires the .cls stem to equal
// the top-level class name, which is why the scenario name lives on
// the directory rather than the file itself.
public class RD2_DataMigrationBase_BATCH {
    public void start() {
        Logger lg = new Logger(Account.SObjectType, 'migration', 'start');
    }

    public class Logger {
        public Logger(SObjectType t, String context, String message) {
        }
    }
}
