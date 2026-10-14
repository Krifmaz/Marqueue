use chrono::{DateTime, Utc};

use crate::feed::FeedItem;
use crate::indexer::{IndexerConfig, IndexerId, Protocol};

/// Where one copy of a release can be grabbed from.
#[derive(Debug, Clone, PartialEq)]
pub struct Origin {
    pub indexer_id: IndexerId,
    pub indexer_name: String,
    pub guid: Option<String>,
    /// `.nzb` / `.torrent` URL, or a magnet link if that's all there is.
    pub download_url: Option<String>,
    pub info_url: Option<String>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    pub grabs: Option<u32>,
    /// Torznab `downloadvolumefactor`: 0 is freeleech.
    pub download_factor: Option<f32>,
    pub upload_factor: Option<f32>,
}

/// One release found by the search, parsed by playbill. After
/// [`dedupe`](crate::dedupe) a result may have several origins; the
/// first is the best one.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub title: String,
    pub release: playbill::Release,
    pub protocol: Protocol,
    pub size: Option<u64>,
    pub published: Option<DateTime<Utc>>,
    pub categories: Vec<u32>,
    /// Lowercase hex BitTorrent infohash.
    pub infohash: Option<String>,
    pub magnet: Option<String>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<u32>,
    pub tvdb_id: Option<u32>,
    pub origins: Vec<Origin>,
}

impl SearchResult {
    pub(crate) fn from_item(item: FeedItem, indexer: &IndexerConfig) -> Self {
        let infohash = item
            .attr("infohash")
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty());
        let magnet = item.attr("magneturl").map(str::to_owned);
        let seeders = item.attr_num::<u32>("seeders");
        // Torznab `peers` counts seeders too.
        let leechers = item.attr_num::<u32>("leechers").or_else(|| {
            Some(
                item.attr_num::<u32>("peers")?
                    .saturating_sub(seeders.unwrap_or(0)),
            )
        });
        let imdb_id = item
            .attr("imdbid")
            .or_else(|| item.attr("imdb"))
            .map(|id| id.trim().trim_start_matches("tt"))
            .filter(|id| !id.is_empty() && *id != "0")
            .map(|id| format!("tt{id:0>7}"));

        let origin = Origin {
            indexer_id: indexer.id.clone(),
            indexer_name: indexer.name.clone(),
            guid: item.guid.clone(),
            download_url: item
                .enclosure_url
                .clone()
                .or_else(|| item.link.clone())
                .or_else(|| magnet.clone()),
            info_url: item
                .comments
                .clone()
                .or_else(|| item.guid.clone().filter(|g| g.starts_with("http"))),
            seeders,
            leechers,
            grabs: item.attr_num("grabs"),
            download_factor: item.attr_num("downloadvolumefactor"),
            upload_factor: item.attr_num("uploadvolumefactor"),
        };

        Self {
            release: playbill::parse(&item.title),
            title: item.title.clone(),
            protocol: indexer.protocol,
            size: item.size,
            published: item.published,
            tmdb_id: item.attr_num("tmdbid").filter(|&id| id > 0),
            tvdb_id: item.attr_num("tvdbid").filter(|&id| id > 0),
            categories: item.categories,
            infohash,
            magnet,
            imdb_id,
            origins: vec![origin],
        }
    }

    /// The best place to grab this release from.
    pub fn best_origin(&self) -> &Origin {
        &self.origins[0]
    }

    /// Total seeders across every origin.
    pub fn seeders(&self) -> Option<u32> {
        self.origins
            .iter()
            .filter_map(|o| o.seeders)
            .reduce(|a, b| a + b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::parse_feed;

    #[test]
    fn maps_torznab_item() {
        let idx = IndexerConfig::new("t", "Tracker", Protocol::Torznab, "http://t", "k");
        let item = parse_feed(include_str!("../tests/fixtures/torznab.xml"))
            .unwrap()
            .remove(0);
        let r = SearchResult::from_item(item, &idx);

        assert_eq!(r.release.title.as_deref(), Some("The Bear"));
        assert_eq!(r.release.seasons, vec![3]);
        assert_eq!(r.release.episodes, vec![4]);
        assert_eq!(
            r.infohash.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(r.tvdb_id, Some(412_457));
        let o = r.best_origin();
        assert_eq!(o.seeders, Some(120));
        assert_eq!(o.leechers, Some(15));
        assert_eq!(o.download_factor, Some(0.0));
        assert_eq!(
            o.download_url.as_deref(),
            Some("https://tracker.example/dl/1001.torrent?passkey=PK")
        );
    }
}
