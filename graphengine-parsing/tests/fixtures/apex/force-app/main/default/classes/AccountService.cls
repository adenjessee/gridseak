public with sharing class AccountService {
    private final AccountRepository repository;

    public AccountService() {
        this.repository = new AccountRepository();
    }

    public AccountService(AccountRepository repo) {
        this.repository = repo;
    }

    public List<Account> fetchRecent(Integer limitCount) {
        List<Account> accounts = repository.query(limitCount);
        return accounts;
    }

    public void markProcessed(List<Account> accounts) {
        for (Account a : accounts) {
            a.Description = 'processed';
        }
        repository.persist(accounts);
    }

    @AuraEnabled
    public static Account getById(Id accountId) {
        AccountService svc = new AccountService();
        List<Account> rows = svc.fetchRecent(1);
        if (rows.isEmpty()) {
            return null;
        }
        return rows[0];
    }

    public Integer switchExample(Integer x) {
        Integer total = 0;
        switch on x {
            when 1, 2 { total = 10; }
            when 3    { total = 20; }
            when else { total = 99; }
        }
        return total;
    }
}
