use legate_router::RouterError;
use legate_store::StoreError;

/// Recoverable failure from the lead, a goal, a routine, or a trigger.
#[derive(Debug, thiserror::Error)]
pub enum LeadError {
    /// A message the TypeScript code threw as `Error`.
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Router(#[from] RouterError),
}

impl LeadError {
    pub fn message(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }
}
