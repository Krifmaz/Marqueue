//! Metadata providers (TMDB, TVDB, AniList, Open Library, ...) behind one
//! trait, with results cached locally so Marqueue never depends on a central
//! server. Providers land per phase; this crate defines the shape.

use marqueue_core::MediaType;
use serde::{Deserialize, Serialize};

/// A search hit or lookup result from a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TitleInfo {
    /// Provider name, e.g. `"tmdb"`.
    pub provider: String,
    /// The provider's own id.
    pub external_id: String,
    pub media_type: MediaType,
    pub title: String,
    pub year: Option<u16>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
}

#[derive(Debug)]
pub enum MetadataError {
    NotFound,
    RateLimited,
    Other(String),
}

impl std::fmt::Display for MetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::RateLimited => f.write_str("rate limited"),
            Self::Other(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for MetadataError {}

/// A public metadata source.
pub trait MetadataProvider: Send + Sync {
    fn name(&self) -> &'static str;
    /// Media types this provider covers.
    fn media_types(&self) -> &'static [MediaType];
    fn search(
        &self,
        media_type: MediaType,
        query: &str,
        year: Option<u16>,
    ) -> impl Future<Output = Result<Vec<TitleInfo>, MetadataError>> + Send;
    fn lookup(
        &self,
        external_id: &str,
    ) -> impl Future<Output = Result<TitleInfo, MetadataError>> + Send;
}
