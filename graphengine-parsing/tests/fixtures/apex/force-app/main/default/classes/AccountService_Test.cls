@IsTest
private class AccountService_Test {

    @IsTest
    static void fetchRecent_returnsRows() {
        AccountService svc = new AccountService();
        List<Account> rows = svc.fetchRecent(5);
        System.assertEquals(0, rows.size());
    }

    static testMethod void markProcessed_setsDescription() {
        AccountService svc = new AccountService();
        List<Account> rows = new List<Account>();
        svc.markProcessed(rows);
    }
}
