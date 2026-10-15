//! Scoutline embedded in Marqueue: searches every configured Newznab and
//! Torznab indexer at once, in-process, with no Prowlarr and no HTTP hop.
//!
//! [`Scoutline`] implements [`marqueue_adapters::Indexer`]. Names are read
//! with [`playbill`] using the hint for the media type being searched, like
//! every other Marqueue candidate.

use std::path::Path;

use marqueue_adapters::{AdapterError, Candidate, Indexer, Protocol, Result, SearchQuery};
use marqueue_core::MediaType;
use scoutline_core::{Engine, SearchType, category};

pub use scoutline_core::health;
pub use scoutline_core::settings::{IndexerSettings, SettingValue, validate};

/// The embedded indexer engine.
#[derive(Clone)]
pub struct Scoutline {
    engine: Engine,
}

impl Scoutline {
    /// Builds the engine for these indexers; YAML definitions are read
    /// from `definitions_dir`. Fails on invalid settings.
    pub fn new(
        indexers: &[IndexerSettings],
        definitions_dir: &Path,
    ) -> std::result::Result<Self, AdapterError> {
        validate(indexers).map_err(AdapterError)?;
        let net = scoutline_net::NetSettings::default();
        let configs = scoutline_defs::build_indexers(indexers, definitions_dir, &net)
            .map_err(AdapterError)?;
        let engine = Engine::builder()
            .net(net)
            .indexers(configs)
            .build()
            .map_err(|e| AdapterError(e.to_string()))?;
        Ok(Self { engine })
    }

    /// The engine itself, for health and stats.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn indexer_count(&self) -> usize {
        self.engine.indexers().filter(|i| i.enabled).count()
    }
}

impl Indexer for Scoutline {
    fn name(&self) -> &str {
        "Scoutline"
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>> {
        let q = to_scoutline(query);
        let out = self.engine.search(&q).await;
        if out.results.is_empty() && !out.errors.is_empty() {
            let why: Vec<String> = out
                .errors
                .iter()
                .map(|(id, e)| format!("{id}: {e}"))
                .collect();
            return Err(AdapterError(format!(
                "every indexer failed: {}",
                why.join("; ")
            )));
        }

        Ok(out
            .results
            .iter()
            .filter_map(|r| {
                let o = r.best_origin();
                let url = o.download_url.as_deref().or(r.magnet.as_deref())?;
                let protocol = match r.protocol {
                    scoutline_core::Protocol::Torznab => Protocol::Torrent,
                    scoutline_core::Protocol::Newznab => Protocol::Usenet,
                };
                let mut c =
                    Candidate::new(&o.indexer_name, protocol, url, &r.title, query.media_type);
                c.size_bytes = r.size;
                c.seeders = r.seeders();
                Some(c)
            })
            .collect())
    }
}

/// Turns Marqueue's query into the Newznab/Torznab search the *arr apps
/// would send for it.
pub fn to_scoutline(q: &SearchQuery) -> scoutline_core::SearchQuery {
    let title = q.title.trim();
    let (search_type, term, cats) = match q.media_type {
        MediaType::Movie => (
            SearchType::Movie,
            match q.year {
                Some(y) => format!("{title} {y}"),
                None => title.to_owned(),
            },
            vec![category::MOVIES.id],
        ),
        // Anime trackers name episodes by absolute number, so search for it.
        MediaType::Anime if q.absolute_episode.is_some() => (
            SearchType::General,
            format!("{title} {:02}", q.absolute_episode.unwrap_or_default()),
            vec![category::TV_ANIME.id],
        ),
        MediaType::Anime => (
            SearchType::Tv,
            title.to_owned(),
            vec![category::TV_ANIME.id],
        ),
        MediaType::Tv => (SearchType::Tv, title.to_owned(), vec![category::TV.id]),
        MediaType::Ebook => (SearchType::Book, title.to_owned(), vec![category::BOOKS.id]),
        MediaType::Audiobook => (
            SearchType::General,
            title.to_owned(),
            vec![category::AUDIO_AUDIOBOOK.id],
        ),
    };
    let mut out = scoutline_core::SearchQuery {
        search_type,
        term: Some(term),
        ..Default::default()
    }
    .categories(cats);
    if search_type == SearchType::Tv {
        out.season = q.season.map(u32::from);
        out.episode = q.episode.map(|e| e.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn query(media_type: MediaType, title: &str) -> SearchQuery {
        SearchQuery {
            media_type,
            title: title.into(),
            year: None,
            season: None,
            episode: None,
            absolute_episode: None,
        }
    }

    #[test]
    fn maps_queries() {
        let mut q = query(MediaType::Tv, "The Bear");
        q.season = Some(3);
        q.episode = Some(4);
        let s = to_scoutline(&q);
        assert_eq!(s.search_type, SearchType::Tv);
        assert_eq!((s.season, s.episode.as_deref()), (Some(3), Some("4")));
        assert_eq!(s.categories, vec![5000]);

        let mut q = query(MediaType::Movie, "Dune Part Two");
        q.year = Some(2024);
        assert_eq!(to_scoutline(&q).term.as_deref(), Some("Dune Part Two 2024"));

        let mut q = query(MediaType::Anime, "One Piece");
        q.absolute_episode = Some(1089);
        let s = to_scoutline(&q);
        assert_eq!(
            (s.search_type, s.term.as_deref()),
            (SearchType::General, Some("One Piece 1089"))
        );
        assert_eq!(s.categories, vec![5070]);
    }

    const FEED: &str = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
<item><title>One.Piece.1089.1080p.WEB.H264-GROUP</title>
<guid>g1</guid><link>https://t.example/dl/1.torrent</link><size>1500000000</size>
<torznab:attr name="category" value="5070"/><torznab:attr name="seeders" value="33"/></item>
</channel></rss>"#;

    #[tokio::test]
    async fn searches_and_builds_candidates() {
        let tracker = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("t", "search"))
            .and(query_param("q", "One Piece 1089"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
            .mount(&tracker)
            .await;

        let settings: Vec<IndexerSettings> = vec![IndexerSettings {
            id: "t".into(),
            name: Some("Tracker".into()),
            protocol: Some(scoutline_core::Protocol::Torznab),
            url: tracker.uri(),
            api_key: "k".into(),
            ..Default::default()
        }];
        let s = Scoutline::new(&settings, Path::new("/nonexistent")).unwrap();
        assert_eq!(s.indexer_count(), 1);

        let mut q = query(MediaType::Anime, "One Piece");
        q.absolute_episode = Some(1089);
        let c = s.search(&q).await.unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].indexer, "Tracker");
        assert_eq!(c[0].protocol, Protocol::Torrent);
        assert_eq!(c[0].seeders, Some(33));
        assert_eq!(c[0].size_bytes, Some(1_500_000_000));
        assert_eq!(c[0].release.absolute_episodes, vec![1089]);
    }

    #[tokio::test]
    async fn all_failing_is_an_error() {
        let settings = vec![IndexerSettings {
            id: "down".into(),
            protocol: Some(scoutline_core::Protocol::Newznab),
            url: "http://127.0.0.1:9".into(),
            api_key: "k".into(),
            timeout_secs: Some(2),
            ..Default::default()
        }];
        let s = Scoutline::new(&settings, Path::new("/nonexistent")).unwrap();
        let err = s
            .search(&query(MediaType::Movie, "Dune"))
            .await
            .unwrap_err();
        assert!(err.0.contains("every indexer failed"), "{}", err.0);
    }
}
