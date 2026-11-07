use legate_store::StoreError;

/// Failure from opening or driving a terminal.
#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    /// `live` sessions already reached the host's cap.
    #[error("too many terminals")]
    TooMany,
    /// The PTY could not be created. `provider` is the session kind (`claude`, `shell`, …).
    #[error("could not start {provider}: {message}")]
    Start { provider: String, message: String },
    /// `launch` was asked to start a shell. Shells use [`crate::setup_launch`].
    #[error("shell sessions have no CLI")]
    ShellHasNoCli,
    /// `create_chat` did not return within 10 seconds.
    #[error("timed out")]
    TimedOut,
    /// A PTY or chat operation failed. The string is the underlying message.
    #[error("{0}")]
    Message(String),
    /// The store rejected a read or write.
    #[error(transparent)]
    Store(#[from] StoreError),
}
