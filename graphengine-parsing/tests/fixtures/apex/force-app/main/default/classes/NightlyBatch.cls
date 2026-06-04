global class NightlyBatch implements Database.Batchable<SObject>, Schedulable {

    global Database.QueryLocator start(Database.BatchableContext bc) {
        return Database.getQueryLocator('SELECT Id FROM Account');
    }

    global void execute(Database.BatchableContext bc, List<SObject> scope) {
        for (SObject s : scope) {
            // no-op
        }
    }

    global void finish(Database.BatchableContext bc) {
        // no-op
    }

    global void execute(SchedulableContext sc) {
        Database.executeBatch(new NightlyBatch(), 200);
    }
}
