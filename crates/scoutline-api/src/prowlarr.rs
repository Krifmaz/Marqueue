//! Prowlarr-compatible REST API (`/api/v1/search`, `/api/v1/indexer`), so
//! apps that talk to Prowlarr directly, like Encore, can point at
//! Scoutline instead. Results also carry Scoutline's parsed `release` and
//! every `source` the release was found on.

use chrono::Utc;
use scoutline_core::health::IndexerHealth;
use scoutline_core::{
    Engine, Origin, Protocol, Scope, SearchQuery, SearchResult, SearchType, category, playbill,
};
use serde::Serialize;

/// Prowlarr's special `indexerIds`: every Usenet / every torrent indexer.
const ALL_USENET: i64 = -1;
const ALL_TORRENT: i64 = -2;

/// A parsed `/api/v1/search` query string. Prowlarr accepts list
/// parameters both repeated and comma-separated.
#[derive(Debug, Default, PartialEq)]
pub struct SearchParams {
    pub query: Option<String>,
    pub search_type: SearchType,
    pub categories: Vec<u32>,
    pub indexer_ids: Vec<i64>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl SearchParams {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut p = Self::default();
        for (k, v) in url::form_urlencoded::parse(raw.as_bytes()) {
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            match k.as_ref() {
                "query" | "q" => p.query = Some(v.to_owned()),
                "type" => {
                    p.search_type = match v {
                        "search" => SearchType::General,
                        "tvsearch" => SearchType::Tv,
                        "movie" => SearchType::Movie,
                        "book" => SearchType::Book,
                        "music" | "audio" => SearchType::Music,
                        other => return Err(format!("unknown search type {other:?}")),
                    }
                }
                "categories" => {
                    for c in v.split(',') {
                        p.categories.push(
                            c.trim()
                                .parse()
                                .map_err(|_| format!("bad category {c:?}"))?,
                        );
                    }
                }
                "indexerIds" => {
                    for i in v.split(',') {
                        p.indexer_ids.push(
                            i.trim()
                                .parse()
                                .map_err(|_| format!("bad indexer id {i:?}"))?,
                        );
                    }
                }
                "limit" => {
                    p.limit = Some(v.parse().map_err(|_| "limit must be a number".to_owned())?)
                }
                "offset" => {
                    p.offset = Some(
                        v.parse()
                            .map_err(|_| "offset must be a number".to_owned())?,
                    )
                }
                _ => {}
            }
        }
        Ok(p)
    }

    pub fn query(&self) -> SearchQuery {
        let mut q = SearchQuery {
            search_type: self.search_type,
            term: self.query.clone(),
            limit: self.limit,
            offset: self.offset,
            ..SearchQuery::default()
        };
        q = q.categories(self.categories.iter().copied());
        q
    }

    /// Which indexers to ask. Numeric ids are Prowlarr-style positions,
    /// see [`numeric_id`].
    pub fn scope(&self, engine: &Engine) -> Scope {
        if self.indexer_ids.is_empty() {
            return Scope::All;
        }
        if self.indexer_ids == [ALL_USENET] {
            return Scope::Protocol(Protocol::Newznab);
        }
        if self.indexer_ids == [ALL_TORRENT] {
            return Scope::Protocol(Protocol::Torznab);
        }
        let ids = engine
            .indexers()
            .enumerate()
            .filter(|(n, i)| {
                let num = numeric_id(*n);
                self.indexer_ids.contains(&num)
                    || (self.indexer_ids.contains(&ALL_USENET) && i.protocol == Protocol::Newznab)
                    || (self.indexer_ids.contains(&ALL_TORRENT) && i.protocol == Protocol::Torznab)
            })
            .map(|(_, i)| i.id.clone())
            .collect();
        Scope::Indexers(ids)
    }
}

/// Prowlarr ids are numbers; Scoutline's are names. Indexers are numbered
/// from 1 in config order.
pub fn numeric_id(position: usize) -> i64 {
    position as i64 + 1
}

fn protocol_name(p: Protocol) -> &'static str {
    match p {
        Protocol::Torznab => "torrent",
        Protocol::Newznab => "usenet",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    id: u32,
    name: String,
}

/// One result in Prowlarr's `ReleaseResource` shape.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release<'a> {
    guid: String,
    title: &'a str,
    sort_title: String,
    protocol: &'static str,
    size: Option<u64>,
    age: i64,
    age_hours: f64,
    age_minutes: f64,
    publish_date: Option<String>,
    indexer_id: i64,
    indexer: &'a str,
    download_url: Option<&'a str>,
    magnet_url: Option<&'a str>,
    info_url: Option<&'a str>,
    comment_url: Option<&'a str>,
    info_hash: Option<&'a str>,
    seeders: Option<u32>,
    leechers: Option<u32>,
    grabs: Option<u32>,
    imdb_id: u64,
    tmdb_id: u32,
    tvdb_id: u32,
    categories: Vec<Category>,
    download_volume_factor: Option<f32>,
    upload_volume_factor: Option<f32>,
    /// Scoutline: what playbill read from the title.
    release: &'a playbill::Release,
    /// Scoutline: every indexer this release was found on, best first.
    sources: &'a [Origin],
}

pub fn release<'a>(r: &'a SearchResult, engine: &Engine) -> Release<'a> {
    let o = r.best_origin();
    let indexer_id = engine
        .indexers()
        .position(|i| i.id == o.indexer_id)
        .map(numeric_id)
        .unwrap_or(0);
    let (age_minutes, publish_date) = match r.published {
        Some(p) => (
            (Utc::now() - p).num_seconds().max(0) as f64 / 60.0,
            Some(p.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        ),
        None => (0.0, None),
    };
    let magnet = r.magnet.as_deref();
    Release {
        guid: o
            .guid
            .clone()
            .or_else(|| o.download_url.clone())
            .unwrap_or_default(),
        title: &r.title,
        sort_title: r.title.to_lowercase(),
        protocol: protocol_name(r.protocol),
        size: r.size,
        age: (age_minutes / 1440.0) as i64,
        age_hours: (age_minutes / 60.0 * 10.0).round() / 10.0,
        age_minutes: age_minutes.round(),
        publish_date,
        indexer_id,
        indexer: &o.indexer_name,
        // A magnet-only result shows up as both, as Prowlarr does.
        download_url: o.download_url.as_deref().or(magnet),
        magnet_url: magnet,
        info_url: o.info_url.as_deref(),
        comment_url: o.info_url.as_deref(),
        info_hash: r.infohash.as_deref(),
        seeders: o.seeders,
        leechers: o.leechers,
        grabs: o.grabs,
        imdb_id: r
            .imdb_id
            .as_deref()
            .and_then(|id| id.trim_start_matches("tt").parse().ok())
            .unwrap_or(0),
        tmdb_id: r.tmdb_id.unwrap_or(0),
        tvdb_id: r.tvdb_id.unwrap_or(0),
        categories: r
            .categories
            .iter()
            .map(|&id| Category {
                id,
                name: category::by_id(id).map_or_else(|| id.to_string(), |c| c.name.to_owned()),
            })
            .collect(),
        download_volume_factor: o.download_factor,
        upload_volume_factor: o.upload_factor,
        release: &r.release,
        sources: &r.origins,
    }
}

/// One indexer in Prowlarr's `IndexerResource` shape, plus health.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Indexer {
    id: i64,
    /// Scoutline's own id, used in `/torznab/{slug}/api`.
    slug: String,
    name: String,
    enable: bool,
    protocol: &'static str,
    implementation: &'static str,
    health: Option<IndexerHealth>,
}

pub fn indexers(engine: &Engine) -> Vec<Indexer> {
    engine
        .indexers()
        .enumerate()
        .map(|(n, i)| Indexer {
            id: numeric_id(n),
            slug: i.id.clone(),
            name: i.name.clone(),
            enable: i.enabled,
            protocol: protocol_name(i.protocol),
            implementation: match i.protocol {
                Protocol::Torznab => "Torznab",
                Protocol::Newznab => "Newznab",
            },
            health: engine.health().get(&i.id),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_encore_search() {
        let p = SearchParams::parse(
            "query=Boards+of+Canada+Geogaddi&type=search&categories=3000&limit=100",
        )
        .unwrap();
        assert_eq!(p.query.as_deref(), Some("Boards of Canada Geogaddi"));
        assert_eq!(p.categories, vec![3000]);
        assert_eq!(p.limit, Some(100));
        let q = p.query();
        assert_eq!(q.search_type, SearchType::General);
        assert_eq!(q.categories, vec![3000]);
    }

    #[test]
    fn repeated_and_comma_lists() {
        let p = SearchParams::parse(
            "categories=3000&categories=3010,3040&indexerIds=1,2&indexerIds=-2&type=music",
        )
        .unwrap();
        assert_eq!(p.categories, vec![3000, 3010, 3040]);
        assert_eq!(p.indexer_ids, vec![1, 2, -2]);
        assert_eq!(p.search_type, SearchType::Music);
        assert!(SearchParams::parse("type=radio").is_err());
        assert!(SearchParams::parse("categories=audio").is_err());
    }
}
