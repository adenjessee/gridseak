// TR-A.6 fixture #3 — BaseLoader (top-level parent). See
// OverrideOuter.cls for the override / caller. Split out so each
// top-level public class lives in its own file, per Apex's one-
// public-class-per-file rule.
public virtual class BaseLoader {
    public virtual void load(Set<Id> ids) {}
}
