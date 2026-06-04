public with sharing class UTIL_JobProgress_CTRL {
    public Boolean hasActiveJobs { get; private set; }
    public String jobSummary { get; private set; }

    public UTIL_JobProgress_CTRL() {
        this.hasActiveJobs = false;
        this.jobSummary = '';
    }

    public PageReference refreshJobs() {
        this.hasActiveJobs = true;
        this.jobSummary = 'Refreshed at ' + DateTime.now();
        return null;
    }
}
