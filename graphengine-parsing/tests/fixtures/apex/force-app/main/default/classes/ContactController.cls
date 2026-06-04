public with sharing class ContactController {

    @AuraEnabled(cacheable=true)
    public static List<Contact> listByAccount(Id accountId) {
        return [SELECT Id, Name FROM Contact WHERE AccountId = :accountId];
    }

    @AuraEnabled
    public static Contact upsertContact(Contact c) {
        upsert c;
        return c;
    }

    @InvocableMethod(label='Promote Contact')
    public static List<Id> promote(List<Id> contactIds) {
        return contactIds;
    }

    public class HelperInner {
        public String describe(Contact c) {
            return c.Name;
        }
    }
}
