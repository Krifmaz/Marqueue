use std::time::Duration;

/// Why one indexer failed to answer a search.
#[derive(Debug, Clone, thiserror::Error)]
pub enum IndexerError {
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("request failed: {0}")]
    Http(String),
    #[error("HTTP {0}")]
    Status(u16),
    /// The indexer answered with a Newznab `<error>` element.
    #[error("indexer error {code}: {description}")]
    Api { code: u32, description: String },
    #[error("unreadable response: {0}")]
    Parse(String),
    #[error("backing off until {until}: {reason}")]
    BackingOff {
        until: chrono::DateTime<chrono::Utc>,
        reason: String,
    },
}

impl IndexerError {
    /// Errors that say the site is struggling, so health should back off.
    /// Bad credentials and parse errors also count: retrying won't fix them.
    pub fn counts_as_failure(&self) -> bool {
        !matches!(self, Self::BackingOff { .. })
    }
}
