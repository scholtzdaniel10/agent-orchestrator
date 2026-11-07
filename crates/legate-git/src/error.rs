//! Errors surfaced to callers. The display text matches the TypeScript messages.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("{0}")]
    Message(String),
}

impl GitError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl From<std::io::Error> for GitError {
    fn from(err: std::io::Error) -> Self {
        Self::new(err.to_string())
    }
}
