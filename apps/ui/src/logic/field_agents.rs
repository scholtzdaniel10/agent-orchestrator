//! What the empty-workspace background (`assets/field.js`) shows for each agent.

use legate_core::{JobRecord, JobStatus, PROVIDERS};

/// Local storage key; "off" hides the background.
pub const BACKGROUND_KEY: &str = "orch.background";

/// `"lead:working claude:resting codex:error"`: agents that are running, queued, or whose
/// newest job failed. Jobs come oldest first.
pub fn field_agents(jobs: &[JobRecord], lead_busy: bool) -> String {
    let mut out = Vec::new();
    if lead_busy {
        out.push("lead:working".to_string());
    }
    for provider in PROVIDERS {
        let has = |status| {
            jobs.iter()
                .any(|job| job.provider == Some(provider) && job.status == status)
        };
        let newest = jobs.iter().rev().find(|job| job.provider == Some(provider));
        let state = if has(JobStatus::Running) {
            "working"
        } else if has(JobStatus::Queued) {
            "resting"
        } else if newest.is_some_and(|job| job.status == JobStatus::Failed) {
            "error"
        } else {
            continue;
        };
        out.push(format!("{}:{state}", provider.as_str()));
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::field_agents;
    use legate_core::{JobRecord, JobStatus, JobType, ProviderId};
    use std::collections::BTreeMap;

    fn job(provider: ProviderId, status: JobStatus) -> JobRecord {
        JobRecord {
            id: String::new(),
            job_type: JobType::Refactor,
            prompt: String::new(),
            provider: Some(provider),
            status,
            output: String::new(),
            failed_over: Vec::new(),
            steps: None,
            steps_dropped: None,
            model: None,
            reason: None,
            edit: None,
            access: None,
            change: None,
            group: None,
            lead_message: None,
            routine: None,
            title: None,
            session_id: None,
            goal: None,
            base: None,
            error: None,
            owner_id: None,
            heartbeat_at: None,
            cancel_requested: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn running_beats_queued_and_only_the_newest_failure_counts() {
        use JobStatus::{Done, Failed, Queued, Running};
        use ProviderId::{Claude, Codex, Cursor, Grok};
        let jobs = [
            job(Claude, Queued),
            job(Claude, Running),
            job(Codex, Done),
            job(Codex, Failed),
            job(Cursor, Failed),
            job(Cursor, Done),
            job(Grok, Queued),
        ];
        assert_eq!(
            field_agents(&jobs, true),
            "lead:working claude:working codex:error grok:resting"
        );
        assert_eq!(field_agents(&[], false), "");
    }
}
