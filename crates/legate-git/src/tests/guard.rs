use crate::guard::{
    GuardJob, JOB_STALE_MS, WORKTREE_BUSY_MESSAGE, assert_worktree_idle, worktree_has_active_job,
};

const NOW: i64 = 1_000_000;
const CHANGE: &str = "abcd1234";

fn job(
    status: &str,
    change: Option<&str>,
    owner: Option<&str>,
    heartbeat: Option<i64>,
) -> GuardJob {
    GuardJob {
        status: status.to_string(),
        change: change.map(str::to_string),
        owner_id: owner.map(str::to_string),
        heartbeat_at: heartbeat,
    }
}

#[test]
fn queued_or_running_blocks_merge_and_discard() {
    assert!(worktree_has_active_job(
        &[job("running", Some(CHANGE), Some("a"), Some(NOW))],
        CHANGE,
        NOW
    ));
    assert!(worktree_has_active_job(
        &[job("queued", Some(CHANGE), Some("a"), Some(NOW))],
        CHANGE,
        NOW
    ));
    let err = assert_worktree_idle(
        &[job("running", Some(CHANGE), Some("a"), Some(NOW))],
        CHANGE,
        NOW,
    )
    .unwrap_err();
    assert_eq!(err.to_string(), WORKTREE_BUSY_MESSAGE);
}

#[test]
fn finished_other_worktree_or_stale_heartbeat_does_not_block() {
    assert!(!worktree_has_active_job(
        &[job("done", Some(CHANGE), Some("a"), Some(NOW))],
        CHANGE,
        NOW
    ));
    assert!(!worktree_has_active_job(
        &[job("running", Some("ffff0000"), Some("a"), Some(NOW))],
        CHANGE,
        NOW
    ));
    assert!(!worktree_has_active_job(
        &[job(
            "running",
            Some(CHANGE),
            Some("other"),
            Some(NOW - JOB_STALE_MS)
        )],
        CHANGE,
        NOW
    ));
}

#[test]
fn job_with_no_owner_still_blocks() {
    assert!(worktree_has_active_job(
        &[job("running", Some(CHANGE), None, None)],
        CHANGE,
        NOW
    ));
    assert!(worktree_has_active_job(
        &[job("queued", Some(CHANGE), None, None)],
        CHANGE,
        NOW
    ));
}

#[test]
fn foreign_job_with_a_fresh_heartbeat_blocks() {
    assert!(worktree_has_active_job(
        &[job(
            "running",
            Some(CHANGE),
            Some("other"),
            Some(NOW - JOB_STALE_MS + 1)
        )],
        CHANGE,
        NOW
    ));
}
