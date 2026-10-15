//! Indexers that aren't plain Newznab/Torznab feeds (e.g. YAML
//! definitions scraping a site) plug in here.

use std::future::Future;
use std::pin::Pin;

use crate::error::IndexerError;
use crate::feed::FeedItem;
use crate::query::SearchQuery;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A downloaded `.torrent` / `.nzb`, for sites that need the indexer's
/// session to fetch it.
#[derive(Debug, Clone)]
pub struct Download {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Searches one site. Results are returned as feed items so they go
/// through the same playbill parsing and de-duplication as native feeds.
pub trait Backend: Send + Sync {
    fn search<'a>(
        &'a self,
        query: &'a SearchQuery,
    ) -> BoxFuture<'a, Result<Vec<FeedItem>, IndexerError>>;

    /// True when download links only work with this indexer's session
    /// (cookies), so the server must fetch them on the client's behalf.
    fn needs_download_proxy(&self) -> bool {
        false
    }

    fn download<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Download, IndexerError>> {
        let _ = url;
        Box::pin(async {
            Err(IndexerError::Http(
                "this indexer has no download proxy".into(),
            ))
        })
    }
}
