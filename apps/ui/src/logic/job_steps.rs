//! Placeholder for `job-steps.ts`. The jobs slice fills this in.

use legate_core::JobRecord;

pub fn latest_step_title(job: &JobRecord) -> Option<String> {
    job.steps.as_ref()?.last().map(|step| step.title.clone())
}
