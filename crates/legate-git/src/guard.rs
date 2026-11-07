//! A queued or running job, in this process or a live foreign one, holds its worktree.

pub const JOB_STALE_MS: i64 = 20_000;

pub const WORKTREE_BUSY_MESSAGE: &str =
    "A job is still running in this worktree. Stop it first, or wait for it to finish.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardJob {
    pub status: String,
    pub change: Option<String>,
    pub owner_id: Option<String>,
    pub heartbeat_at: Option<i64>,
}

pub fn worktree_has_active_job(jobs: &[GuardJob], change_id: &str, now: i64) -> bool {
    jobs.iter()
        .any(|job| job.change.as_deref() == Some(change_id) && job_is_live(job, now))
}

pub fn assert_worktree_idle(
    jobs: &[GuardJob],
    change_id: &str,
    now: i64,
) -> Result<(), crate::error::GitError> {
    if worktree_has_active_job(jobs, change_id, now) {
        return Err(crate::error::GitError::new(WORKTREE_BUSY_MESSAGE));
    }
    Ok(())
}

fn job_is_live(job: &GuardJob, now: i64) -> bool {
    if job.status != "queued" && job.status != "running" {
        return false;
    }
    match (&job.owner_id, job.heartbeat_at) {
        (Some(_), Some(heartbeat)) => now - heartbeat < JOB_STALE_MS,
        _ => true,
    }
}
