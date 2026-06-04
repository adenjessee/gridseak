trigger AccountTrigger on Account (before insert, before update, after insert) {
    if (Trigger.isBefore && Trigger.isInsert) {
        for (Account a : Trigger.new) {
            if (a.Description == null) {
                a.Description = 'created';
            }
        }
    }
    if (Trigger.isAfter && Trigger.isInsert) {
        AccountService svc = new AccountService();
        svc.markProcessed(Trigger.new);
    }
}
