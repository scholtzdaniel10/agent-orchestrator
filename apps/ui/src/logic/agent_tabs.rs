//! Agent tabs in the middle of the window. Port of `agent-tabs.ts`.

use legate_core::{JobRecord, JobStatus, StepTool};

pub fn step_word(tool: StepTool) -> &'static str {
    match tool {
        StepTool::Say => "SAY",
        StepTool::Think => "THINK",
        StepTool::Read => "READ",
        StepTool::Edit => "EDIT",
        StepTool::Shell => "RUN",
        StepTool::Search => "SEARCH",
        StepTool::Web => "WEB",
        StepTool::Mcp => "MCP",
        StepTool::Other => "TOOL",
    }
}

pub fn agent_status_label(job: &JobRecord) -> &'static str {
    match job.status {
        JobStatus::Queued => "waiting",
        JobStatus::Running => "working",
        JobStatus::Failed if job.error.as_deref() == Some("stopped") => "stopped",
        JobStatus::Failed => "failed",
        JobStatus::Done => "done",
    }
}

pub fn agent_jobs(jobs: &[JobRecord], opened: &[String], closed: &[String]) -> Vec<JobRecord> {
    let mut shown = Vec::new();
    let mut seen = Vec::new();
    for id in opened {
        let Some(job) = jobs.iter().find(|job| &job.id == id) else {
            continue;
        };
        if closed.iter().any(|closed_id| closed_id == id)
            || seen.iter().any(|seen_id| seen_id == id)
        {
            continue;
        }
        shown.push(job.clone());
        seen.push(id.clone());
    }
    for job in jobs {
        if seen.iter().any(|id| id == &job.id) || closed.iter().any(|id| id == &job.id) {
            continue;
        }
        if job.title.is_none() || job.lead_message.is_none() {
            continue;
        }
        if job.status != JobStatus::Queued && job.status != JobStatus::Running {
            continue;
        }
        shown.push(job.clone());
        seen.push(job.id.clone());
    }
    shown
}

pub fn agent_tab_label(job: &JobRecord) -> String {
    job.title
        .clone()
        .unwrap_or_else(|| job.prompt.chars().take(40).collect())
}

#[cfg(test)]
mod tests {
    use super::{agent_jobs, agent_status_label, agent_tab_label, step_word};
    use legate_core::{JobRecord, JobStatus, JobType, ProviderId, StepTool};
    use std::collections::BTreeMap;

    fn job(id: &str) -> JobRecord {
        JobRecord {
            id: id.into(),
            job_type: JobType::Refactor,
            prompt: "Move the queue into its own module and keep the tests green".into(),
            provider: Some(ProviderId::Grok),
            status: JobStatus::Running,
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
    fn opened_plan_jobs_come_first_and_closed_ones_stay_closed() {
        let mut a = job("a");
        a.title = Some("Queue".into());
        a.lead_message = Some("m1".into());
        a.status = JobStatus::Done;
        let mut b = job("b");
        b.title = Some("Failover".into());
        b.lead_message = Some("m1".into());
        let mut c = job("c");
        c.title = Some("Old".into());
        c.lead_message = Some("m0".into());
        c.status = JobStatus::Done;
        let mut d = job("d");
        d.lead_message = Some("m2".into());
        let mut e = job("e");
        e.title = Some("Hidden".into());
        e.lead_message = Some("m3".into());
        e.status = JobStatus::Queued;
        let jobs = vec![a, b, c, d, e];
        let ids = |opened: &[&str], closed: &[&str]| -> Vec<String> {
            agent_jobs(
                &jobs,
                &opened
                    .iter()
                    .map(|id| (*id).to_string())
                    .collect::<Vec<_>>(),
                &closed
                    .iter()
                    .map(|id| (*id).to_string())
                    .collect::<Vec<_>>(),
            )
            .into_iter()
            .map(|item| item.id)
            .collect()
        };
        assert_eq!(ids(&["a"], &[]), vec!["a", "b", "e"]);
        assert_eq!(ids(&["a", "a", "gone"], &["e"]), vec!["a", "b"]);
        assert!(ids(&[], &["b", "e"]).is_empty());
    }

    #[test]
    fn labels_status_words_and_step_words() {
        let mut titled = job("x");
        titled.title = Some("Queue".into());
        assert_eq!(agent_tab_label(&titled), "Queue");
        assert_eq!(
            agent_tab_label(&job("x")),
            "Move the queue into its own module and k"
        );
        let mut queued = job("x");
        queued.status = JobStatus::Queued;
        assert_eq!(agent_status_label(&queued), "waiting");
        assert_eq!(agent_status_label(&job("x")), "working");
        let mut done = job("x");
        done.status = JobStatus::Done;
        assert_eq!(agent_status_label(&done), "done");
        let mut stopped = job("x");
        stopped.status = JobStatus::Failed;
        stopped.error = Some("stopped".into());
        assert_eq!(agent_status_label(&stopped), "stopped");
        let mut failed = job("x");
        failed.status = JobStatus::Failed;
        failed.error = Some("boom".into());
        assert_eq!(agent_status_label(&failed), "failed");
        assert_eq!(step_word(StepTool::Shell), "RUN");
        assert_eq!(step_word(StepTool::Read), "READ");
    }
}
