public inherited sharing class ManagedPackageConsumer {

    public List<npsp__Household__c> loadHouseholds() {
        return [SELECT Id, Name FROM npsp__Household__c LIMIT 10];
    }

    public void useDottedType() {
        pi.FormHelper helper = new pi.FormHelper();
        helper.render();
    }

    public String internalOnly(Account a) {
        return a.Name;
    }
}
