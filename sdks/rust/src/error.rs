use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("authentication failed: {message}")]
    Authentication { message: String, status: u16 },

    #[error("forbidden: {message}")]
    Forbidden { message: String, status: u16 },

    #[error("rate limited: {message}")]
    RateLimit {
        message: String,
        status: u16,
        retry_after: Option<Duration>,
    },

    #[error("HTTP {status}: {message}")]
    Api { message: String, status: u16 },

    #[error(transparent)]
    Request(#[from] reqwest::Error),

    #[error("failed to parse response: {0}")]
    Decode(String),
}

impl Error {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Authentication { status, .. } => Some(*status),
            Self::Forbidden { status, .. } => Some(*status),
            Self::RateLimit { status, .. } => Some(*status),
            Self::Api { status, .. } => Some(*status),
            Self::Request(e) => e.status().map(|s| s.as_u16()),
            Self::Decode(_) => None,
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self.status() {
            Some(429 | 502 | 503 | 504) => true,
            _ => matches!(self, Self::Request(e) if e.is_timeout() || e.is_connect()),
        }
    }
}
