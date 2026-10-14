use std::fmt;
use std::time::Duration;

use scoutline_net::Route;

/// Stable identifier for an indexer, chosen by the caller.
pub type IndexerId = String;

/// Which API the indexer speaks. Both share the same request format;
/// Torznab adds torrent fields (seeders, infohash, magnet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    Newznab,
    Torznab,
}

/// A native Newznab or Torznab indexer, reached by URL + API key.
#[derive(Clone)]
pub struct IndexerConfig {
    pub id: IndexerId,
    pub name: String,
    pub protocol: Protocol,
    /// Site root, e.g. `https://api.nzbgeek.info`.
    pub base_url: String,
    /// Path to the API under `base_url`. Defaults to `/api`.
    pub api_path: String,
    pub api_key: String,
    /// How long to wait for this site before giving up on it.
    pub timeout: Duration,
    pub route: Route,
    pub enabled: bool,
}

impl IndexerConfig {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        protocol: Protocol,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            protocol,
            base_url: base_url.into(),
            api_path: "/api".to_owned(),
            api_key: api_key.into(),
            timeout: Duration::from_secs(15),
            route: Route::Direct,
            enabled: true,
        }
    }

    pub fn api_path(mut self, path: impl Into<String>) -> Self {
        self.api_path = path.into();
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Full API endpoint: `base_url` joined with `api_path`.
    pub(crate) fn endpoint(&self) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            self.api_path.trim_start_matches('/')
        )
    }
}

// Hand-written so the API key never ends up in logs.
impl fmt::Debug for IndexerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexerConfig")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("api_path", &self.api_path)
            .field("api_key", &"<redacted>")
            .field("timeout", &self.timeout)
            .field("route", &self.route)
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_joins_cleanly() {
        let c = IndexerConfig::new(
            "a",
            "A",
            Protocol::Torznab,
            "http://host:9117/api/v2.0/indexers/x/results/torznab/",
            "k",
        )
        .api_path("/api");
        assert_eq!(
            c.endpoint(),
            "http://host:9117/api/v2.0/indexers/x/results/torznab/api"
        );
    }

    #[test]
    fn debug_hides_key() {
        let c = IndexerConfig::new("a", "A", Protocol::Newznab, "http://x", "secret-key");
        assert!(!format!("{c:?}").contains("secret-key"));
    }
}
