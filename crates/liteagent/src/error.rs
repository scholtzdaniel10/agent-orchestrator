/// Library result.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from talking to a model server.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The HTTP client could not be constructed.
    #[error("failed to build HTTP client: {0}")]
    Client(String),
    /// The TCP connection or request failed.
    #[error("http request to {url} failed: {source}")]
    Http {
        /// Request URL.
        url: String,
        /// Underlying transport error.
        #[source]
        source: reqwest::Error,
    },
    /// The server returned a non-success status.
    #[error("server {url} returned {status}: {body}")]
    Status {
        /// Request URL.
        url: String,
        /// HTTP status code.
        status: u16,
        /// Response body, truncated.
        body: String,
    },
    /// The response was not JSON.
    #[error("invalid JSON from {url}: {source}")]
    Json {
        /// Request URL.
        url: String,
        /// Parse error.
        #[source]
        source: serde_json::Error,
    },
    /// A required response field was missing.
    #[error("response from {url} missing {field}")]
    Missing {
        /// Request URL.
        url: String,
        /// Field name.
        field: &'static str,
    },
    /// Ollama was selected but no model name was set.
    #[error(
        "model name is required for the Ollama backend; call Backend::with_model before running"
    )]
    ModelRequired,
    /// Auto-detect could not classify the URL.
    #[error("could not detect a model server at {url}: {reason}")]
    Detect {
        /// Base URL.
        url: String,
        /// What was tried.
        reason: String,
    },
}
