public without sharing class AccountRepository {
    public List<Account> query(Integer limitCount) {
        return [SELECT Id, Name, Description FROM Account LIMIT :limitCount];
    }

    public void persist(List<Account> accounts) {
        update accounts;
    }

    public List<Account> searchByTerm(String term) {
        List<List<SObject>> results = [FIND :term IN ALL FIELDS RETURNING Account(Id, Name)];
        return (List<Account>) results[0];
    }
}
