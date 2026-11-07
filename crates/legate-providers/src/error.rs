use thiserror::Error;

/// Failure while preparing a CLI run. Spawn failures are not returned here;
/// they resolve on [`crate::RunHandle::exit`].
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("could not encode bridge config: {0}")]
    Encode(#[from] serde_json::Error),
}
