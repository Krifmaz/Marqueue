//! Torznab/Newznab request parsing and XML output, so Sonarr, Radarr and
//! friends can use Scoutline like any other indexer.

use std::fmt::Write;

use scoutline_core::{Protocol, SearchQuery, SearchResult, SearchType, category};
use serde::Deserialize;

/// Newznab error codes.
pub const ERR_CREDENTIALS: u32 = 100;
pub const ERR_BAD_PARAMETER: u32 = 201;
pub const ERR_NO_FUNCTION: u32 = 202;
pub const ERR_NO_ITEM: u32 = 300;

/// Query string of a Torznab/Newznab request.
#[derive(Debug, Default, Deserialize)]
pub struct Params {
    pub t: Option<String>,
    pub apikey: Option<String>,
    pub q: Option<String>,
    pub imdbid: Option<String>,
    pub tmdbid: Option<String>,
    pub tvdbid: Option<String>,
    pub season: Option<String>,
    pub ep: Option<String>,
    pub author: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub cat: Option<String>,
    pub limit: Option<String>,
    pub offset: Option<String>,
}

/// What a request asks for.
#[derive(Debug, PartialEq)]
pub enum Request {
    Caps,
    Search(Box<SearchQuery>),
}

impl Params {
    pub fn request(&self) -> Result<Request, (u32, String)> {
        let search_type = match self.t.as_deref().unwrap_or("search") {
            "caps" => return Ok(Request::Caps),
            "search" => SearchType::General,
            "tvsearch" => SearchType::Tv,
            "movie" => SearchType::Movie,
            "book" => SearchType::Book,
            "music" => SearchType::Music,
            other => return Err((ERR_NO_FUNCTION, format!("no such function: {other}"))),
        };
        let text = |v: &Option<String>| {
            v.as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        let num = |name: &str, v: &Option<String>| -> Result<Option<u32>, (u32, String)> {
            match text(v) {
                // Some apps send season as "S01".
                Some(v) => v
                    .trim_start_matches(['S', 's'])
                    .parse()
                    .map(Some)
                    .map_err(|_| (ERR_BAD_PARAMETER, format!("{name} must be a number"))),
                None => Ok(None),
            }
        };
        let categories = match text(&self.cat) {
            Some(c) => c
                .split(',')
                .map(|id| id.trim().parse::<u32>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| {
                    (
                        ERR_BAD_PARAMETER,
                        "cat must be comma-separated numbers".to_owned(),
                    )
                })?,
            None => Vec::new(),
        };

        let mut q = SearchQuery {
            search_type,
            term: text(&self.q),
            imdb_id: text(&self.imdbid),
            tmdb_id: num("tmdbid", &self.tmdbid)?,
            tvdb_id: num("tvdbid", &self.tvdbid)?,
            season: num("season", &self.season)?,
            episode: text(&self.ep),
            author: text(&self.author),
            title: text(&self.title),
            artist: text(&self.artist),
            album: text(&self.album),
            limit: num("limit", &self.limit)?,
            offset: num("offset", &self.offset)?,
            ..SearchQuery::default()
        };
        q = q.categories(categories);
        Ok(Request::Search(Box::new(q)))
    }
}

/// `t=caps` answer: what Scoutline can search for.
pub fn caps(server_title: &str) -> String {
    let mut x = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = write!(
        x,
        r#"<caps><server title="{}"/><limits default="100" max="100"/><searching><search available="yes" supportedParams="q"/><tv-search available="yes" supportedParams="q,season,ep,imdbid,tvdbid"/><movie-search available="yes" supportedParams="q,imdbid,tmdbid"/><book-search available="yes" supportedParams="q,author,title"/><music-search available="yes" supportedParams="q,artist,album"/><audio-search available="yes" supportedParams="q,artist,album"/></searching><categories>"#,
        esc(server_title)
    );
    let mut open = false;
    for c in category::ALL {
        if c.id % 1000 == 0 {
            if open {
                x.push_str("</category>");
            }
            let _ = write!(x, r#"<category id="{}" name="{}">"#, c.id, esc(c.name));
            open = true;
        } else {
            let _ = write!(x, r#"<subcat id="{}" name="{}"/>"#, c.id, esc(c.name));
        }
    }
    if open {
        x.push_str("</category>");
    }
    x.push_str("</categories></caps>");
    x
}

pub fn error(code: u32, description: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><error code="{code}" description="{}"/>"#,
        esc(description)
    )
}

/// An RSS feed of `results` in the Torznab or Newznab dialect.
pub fn feed(title: &str, protocol: Protocol, results: &[SearchResult]) -> String {
    let (ns, ns_url, mime) = match protocol {
        Protocol::Torznab => (
            "torznab",
            "http://torznab.com/schemas/2015/feed",
            "application/x-bittorrent",
        ),
        Protocol::Newznab => (
            "newznab",
            "http://www.newznab.com/DTD/2010/feeds/attributes/",
            "application/x-nzb",
        ),
    };
    let mut x = String::with_capacity(1024 + results.len() * 1024);
    let _ = write!(
        x,
        r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:{ns}="{ns_url}"><channel><title>{}</title><description>Scoutline</description>"#,
        esc(title)
    );

    for r in results {
        let o = r.best_origin();
        let Some(link) = o.download_url.as_deref() else {
            continue;
        };
        let attr = |x: &mut String, name: &str, value: &dyn std::fmt::Display| {
            let _ = write!(
                x,
                r#"<{ns}:attr name="{name}" value="{}"/>"#,
                esc(&value.to_string())
            );
        };

        x.push_str("<item>");
        let _ = write!(x, "<title>{}</title>", esc(&r.title));
        let guid = o.guid.as_deref().unwrap_or(link);
        let _ = write!(x, r#"<guid isPermaLink="false">{}</guid>"#, esc(guid));
        let _ = write!(x, "<link>{}</link>", esc(link));
        if let Some(info) = &o.info_url {
            let _ = write!(x, "<comments>{}</comments>", esc(info));
        }
        if let Some(p) = r.published {
            let _ = write!(x, "<pubDate>{}</pubDate>", p.to_rfc2822());
        }
        let _ = write!(
            x,
            r#"<jackettindexer id="{}">{}</jackettindexer>"#,
            esc(&o.indexer_id),
            esc(&o.indexer_name)
        );
        if let Some(size) = r.size {
            let _ = write!(x, "<size>{size}</size>");
        }
        for c in &r.categories {
            let _ = write!(x, "<category>{c}</category>");
        }
        let _ = write!(
            x,
            r#"<enclosure url="{}" length="{}" type="{mime}"/>"#,
            esc(link),
            r.size.unwrap_or(0)
        );
        for c in &r.categories {
            attr(&mut x, "category", c);
        }
        if let Some(size) = r.size {
            attr(&mut x, "size", &size);
        }
        if let Some(s) = o.seeders {
            attr(&mut x, "seeders", &s);
            attr(&mut x, "peers", &(s + o.leechers.unwrap_or(0)));
        }
        if let Some(g) = o.grabs {
            attr(&mut x, "grabs", &g);
        }
        if let Some(h) = &r.infohash {
            attr(&mut x, "infohash", h);
        }
        if let Some(m) = &r.magnet {
            attr(&mut x, "magneturl", m);
        }
        if let Some(id) = &r.imdb_id {
            attr(&mut x, "imdbid", id);
        }
        if let Some(id) = r.tvdb_id {
            attr(&mut x, "tvdbid", &id);
        }
        if let Some(id) = r.tmdb_id {
            attr(&mut x, "tmdbid", &id);
        }
        if let Some(f) = o.download_factor {
            attr(&mut x, "downloadvolumefactor", &f);
        }
        if let Some(f) = o.upload_factor {
            attr(&mut x, "uploadvolumefactor", &f);
        }
        x.push_str("</item>");
    }
    x.push_str("</channel></rss>");
    x
}

/// Escapes text for XML content and attribute values.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters other than tab/newline aren't valid XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use scoutline_core::Origin;
    use scoutline_core::feed::parse_feed;

    fn params(pairs: &[(&str, &str)]) -> Params {
        let qs: String = pairs
            .iter()
            .map(|(k, v)| format!("{k}={}&", v.replace(' ', "+")))
            .collect();
        axum::extract::Query::<Params>::try_from_uri(&format!("/api?{qs}").parse().unwrap())
            .unwrap()
            .0
    }

    fn result(title: &str) -> SearchResult {
        SearchResult {
            title: title.into(),
            release: scoutline_core::playbill::parse(title),
            protocol: Protocol::Torznab,
            size: Some(1234),
            published: Some(chrono::Utc::now()),
            categories: vec![5040],
            infohash: Some("abc".into()),
            magnet: None,
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: Some(412_457),
            origins: vec![Origin {
                indexer_id: "t".into(),
                indexer_name: "Tracker".into(),
                guid: Some("g1".into()),
                download_url: Some("https://t.example/dl?id=1&passkey=PK".into()),
                info_url: None,
                seeders: Some(10),
                leechers: Some(3),
                grabs: None,
                download_factor: Some(0.0),
                upload_factor: Some(1.0),
            }],
        }
    }

    #[test]
    fn reads_sonarr_tvsearch() {
        let p = params(&[
            ("t", "tvsearch"),
            ("q", "The Bear"),
            ("tvdbid", "412457"),
            ("season", "S03"),
            ("ep", "4"),
            ("cat", "5030,5040"),
        ]);
        let Request::Search(q) = p.request().unwrap() else {
            panic!()
        };
        assert_eq!(q.search_type, SearchType::Tv);
        assert_eq!(q.term.as_deref(), Some("The Bear"));
        assert_eq!(q.season, Some(3));
        assert_eq!(q.episode.as_deref(), Some("4"));
        assert_eq!(q.categories, vec![5030, 5040]);
    }

    #[test]
    fn caps_and_errors() {
        assert_eq!(params(&[("t", "caps")]).request(), Ok(Request::Caps));
        assert_eq!(
            params(&[("t", "radio")]).request().unwrap_err().0,
            ERR_NO_FUNCTION
        );
        assert_eq!(
            params(&[("cat", "tv")]).request().unwrap_err().0,
            ERR_BAD_PARAMETER
        );
        assert!(caps("Scoutline").contains(r#"<subcat id="5070" name="TV/Anime"/>"#));
    }

    #[test]
    fn feed_round_trips_through_our_parser() {
        let r = result("The.Bear.S03E04.1080p.WEB.H264-<Crab&Co>");
        let xml = feed("Scoutline", Protocol::Torznab, &[r]);
        let items = parse_feed(&xml).unwrap();
        assert_eq!(items[0].title, "The.Bear.S03E04.1080p.WEB.H264-<Crab&Co>");
        assert_eq!(items[0].attr("seeders"), Some("10"));
        assert_eq!(items[0].attr("peers"), Some("13"));
        assert_eq!(items[0].attr("infohash"), Some("abc"));
        assert_eq!(items[0].categories, vec![5040]);
        assert_eq!(items[0].size, Some(1234));
        assert_eq!(
            items[0].enclosure_url.as_deref(),
            Some("https://t.example/dl?id=1&passkey=PK")
        );
    }

    #[test]
    fn error_is_parseable() {
        let err = parse_feed(&error(ERR_CREDENTIALS, "Invalid API key")).unwrap_err();
        assert!(err.to_string().contains("Invalid API key"));
    }
}
