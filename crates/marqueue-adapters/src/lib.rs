//! Every outside integration sits behind a trait here, so adding a client or
//! server later doesn't touch the core.
//!
//! Phase 1 targets: Prowlarr (Torznab/Newznab), qBittorrent, SABnzbd,
//! Plex, Jellyfin, Audiobookshelf, Discord, ntfy, webhooks and email.

use marqueue_core::MediaType;
use playbill::Release;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct AdapterError(pub String);

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AdapterError {}

pub type Result<T> = std::result::Result<T, AdapterError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Torrent,
    Usenet,
}

/// What to search an indexer for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub media_type: MediaType,
    pub title: String,
    pub year: Option<u16>,
    pub season: Option<u16>,
    pub episode: Option<u16>,
    pub absolute_episode: Option<u16>,
}

/// One result from an indexer, with its name already parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub indexer: String,
    pub protocol: Protocol,
    pub download_url: String,
    pub size_bytes: Option<u64>,
    pub seeders: Option<u32>,
    pub release: Release,
}

impl Candidate {
    /// Parses `name` with the hint for `media_type`.
    pub fn new(
        indexer: &str,
        protocol: Protocol,
        download_url: &str,
        name: &str,
        media_type: MediaType,
    ) -> Self {
        Self {
            indexer: indexer.to_string(),
            protocol,
            download_url: download_url.to_string(),
            size_bytes: None,
            seeders: None,
            release: playbill::parse_with_hint(name, media_type.hint()),
        }
    }

    pub fn size_mib(&self) -> Option<u64> {
        self.size_bytes.map(|b| b / (1024 * 1024))
    }
}

/// Seed rules sent with a grab for private trackers.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SeedRules {
    pub ratio: Option<f32>,
    pub seed_minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadState {
    Queued,
    Downloading,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadStatus {
    pub id: String,
    pub state: DownloadState,
    pub progress: f32,
    /// Where the finished files are, once completed.
    pub output_path: Option<String>,
}

/// Prowlarr, Scoutline or any Torznab/Newznab feed.
pub trait Indexer: Send + Sync {
    fn name(&self) -> &str;
    fn search(&self, query: &SearchQuery) -> impl Future<Output = Result<Vec<Candidate>>> + Send;
}

/// qBittorrent, SABnzbd, ...
pub trait DownloadClient: Send + Sync {
    fn name(&self) -> &str;
    fn protocol(&self) -> Protocol;
    /// Sends a release to the client and returns its download id.
    fn grab(
        &self,
        candidate: &Candidate,
        seed: SeedRules,
    ) -> impl Future<Output = Result<String>> + Send;
    fn status(&self, id: &str) -> impl Future<Output = Result<DownloadStatus>> + Send;
    fn remove(&self, id: &str, delete_files: bool) -> impl Future<Output = Result<()>> + Send;
}

/// Plex, Jellyfin, Audiobookshelf, ...
pub trait MediaServer: Send + Sync {
    fn name(&self) -> &str;
    fn rescan(&self, path: &str) -> impl Future<Output = Result<()>> + Send;
}

/// Discord, ntfy, webhooks, email, ...
pub trait Notifier: Send + Sync {
    fn name(&self) -> &str;
    fn notify(&self, title: &str, body: &str) -> impl Future<Output = Result<()>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_parses_with_media_hint() {
        let c = Candidate::new(
            "test",
            Protocol::Torrent,
            "magnet:?",
            "One.Piece.1089.1080p.WEB.H264-GROUP",
            MediaType::Anime,
        );
        assert_eq!(c.release.absolute_episodes, vec![1089]);
    }
}
