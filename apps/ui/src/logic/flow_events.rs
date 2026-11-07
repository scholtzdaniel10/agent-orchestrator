//! Placeholder for `flow-events.ts`. The activity slice fills this in.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowEvent {
    pub id: String,
    pub text: String,
}

pub fn format_hours_minutes(ms: i64) -> String {
    let minutes = ms / 60_000;
    format!("{}h {}m", minutes / 60, minutes % 60)
}
