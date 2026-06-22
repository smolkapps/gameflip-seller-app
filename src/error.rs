//! Error type for the Gameflip seller library.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// Invalid configuration / credentials supplied by the caller.
    #[error("configuration error: {0}")]
    Config(String),

    /// The underlying HTTP transport failed (connection, TLS, timeout, …).
    #[error("transport error: {0}")]
    Transport(String),

    /// The Gameflip API returned a non-SUCCESS envelope or an HTTP error.
    /// `code` is the API/HTTP status code; `message` is the API message.
    #[error("Gameflip API error {code}: {message}")]
    Api { code: i64, message: String },

    /// A response body could not be parsed into the expected shape.
    #[error("failed to decode response: {0}")]
    Decode(String),

    /// A photo file/URL was unusable (wrong MIME type, too large, unreadable).
    #[error("photo error: {0}")]
    Photo(String),
}

impl Error {
    /// Convenience for building an API error.
    pub fn api(code: i64, message: impl Into<String>) -> Self {
        Error::Api {
            code,
            message: message.into(),
        }
    }
}
