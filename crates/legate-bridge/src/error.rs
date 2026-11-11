/// Failure starting or stopping the HTTP server.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// `bind` on `127.0.0.1` failed.
    #[error("failed to bind 127.0.0.1: {0}")]
    Bind(#[source] std::io::Error),
    /// The listener had no socket address.
    #[error("bridge failed to bind")]
    Address(#[source] std::io::Error),
    /// The OS random source failed.
    #[error("failed to generate bridge token")]
    Token(#[source] getrandom::Error),
    /// The accept loop panicked.
    #[error("bridge server stopped unexpectedly")]
    Stopped,
}

/// A tool rejected its arguments, or the backend failed.
///
/// The MCP layer returns this as a tool result with `isError: true`, not as a
/// JSON-RPC error. JSON-RPC errors are reserved for a broken request.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ToolError {
    message: String,
}

impl ToolError {
    /// A tool error whose text is `message` (what the model sees).
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The text returned to the model.
    pub fn message(&self) -> &str {
        &self.message
    }
}
