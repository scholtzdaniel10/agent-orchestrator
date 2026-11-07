//! Placeholder for `new-task.ts`. The new-task slice fills this in.

use legate_core::{JobType, ProviderId};

pub const DEFAULT_WHO: ProviderId = ProviderId::Cursor;
pub const DEFAULT_KIND: JobType = JobType::Refactor;
pub const DEFAULT_COUNT: i64 = 1;
pub const DEFAULT_ROUNDS: i64 = 5;

pub fn clamp_count(value: i64) -> i64 {
    value.clamp(1, 6)
}

pub fn clamp_rounds(value: i64) -> i64 {
    value.clamp(1, 20)
}
