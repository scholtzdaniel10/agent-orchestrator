//! Failures the renderer would have seen as a thrown `Error` message.

/// A refusal or failure whose text matches the Electron main process.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    message: String,
}

impl AppError {
    /// An error whose display text is `message`.
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The sentence the UI shows.
    pub fn as_str(&self) -> &str {
        &self.message
    }
}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}
