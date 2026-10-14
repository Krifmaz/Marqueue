//! # scoutline-net
//!
//! Builds the HTTP clients Scoutline uses to reach indexers. Each indexer
//! picks a [`Route`]; only direct connections exist so far; proxies,
//! FlareSolverr and WireGuard routes come in phase 3.

use std::time::Duration;

pub use reqwest::Client;

/// Default user agent sent to every indexer unless overridden.
pub const DEFAULT_USER_AGENT: &str = concat!("Scoutline/", env!("CARGO_PKG_VERSION"));

/// How requests to one indexer leave the machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Route {
    /// Straight out, no proxy.
    #[default]
    Direct,
}

/// Settings shared by every client.
#[derive(Debug, Clone)]
pub struct NetSettings {
    pub user_agent: String,
    pub connect_timeout: Duration,
}

impl Default for NetSettings {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("building HTTP client: {0}")]
    Build(#[from] reqwest::Error),
}

/// Builds a client for `route`. It has no overall request timeout; callers
/// set one per indexer.
pub fn build_client(settings: &NetSettings, route: &Route) -> Result<Client, NetError> {
    let builder = Client::builder()
        .user_agent(&settings.user_agent)
        .connect_timeout(settings.connect_timeout)
        .gzip(true)
        .brotli(true);
    let builder = match route {
        Route::Direct => builder.no_proxy(),
    };
    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_direct_client() {
        build_client(&NetSettings::default(), &Route::Direct).unwrap();
    }
}
