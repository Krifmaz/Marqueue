use crate::indexer::Protocol;

/// The Newznab/Torznab search function (`t=` parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SearchType {
    #[default]
    General,
    Movie,
    Tv,
    Book,
    Music,
}

impl SearchType {
    fn function(self) -> &'static str {
        match self {
            Self::General => "search",
            Self::Movie => "movie",
            Self::Tv => "tvsearch",
            Self::Book => "book",
            Self::Music => "music",
        }
    }
}

/// One search, sent to every enabled indexer. A general search with no
/// term is the indexer's RSS feed of latest releases.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct SearchQuery {
    pub search_type: SearchType,
    pub term: Option<String>,
    /// IMDb id, with or without the `tt` prefix.
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<u32>,
    pub tvdb_id: Option<u32>,
    pub season: Option<u32>,
    /// Episode number, or `MM/DD` for daily shows.
    pub episode: Option<String>,
    pub author: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Newznab category ids; empty means all.
    pub categories: Vec<u32>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl SearchQuery {
    /// Latest releases, no search term.
    pub fn rss() -> Self {
        Self::default()
    }

    pub fn general(term: impl Into<String>) -> Self {
        Self {
            term: Some(term.into()),
            ..Self::default()
        }
    }

    pub fn movie(term: impl Into<String>) -> Self {
        Self {
            search_type: SearchType::Movie,
            term: Some(term.into()),
            ..Self::default()
        }
    }

    pub fn tv(term: impl Into<String>) -> Self {
        Self {
            search_type: SearchType::Tv,
            term: Some(term.into()),
            ..Self::default()
        }
    }

    pub fn book(author: Option<String>, title: Option<String>) -> Self {
        Self {
            search_type: SearchType::Book,
            author,
            title,
            ..Self::default()
        }
    }

    pub fn music(artist: Option<String>, album: Option<String>) -> Self {
        Self {
            search_type: SearchType::Music,
            artist,
            album,
            ..Self::default()
        }
    }

    pub fn imdb(mut self, id: impl Into<String>) -> Self {
        self.imdb_id = Some(id.into());
        self
    }

    pub fn tmdb(mut self, id: u32) -> Self {
        self.tmdb_id = Some(id);
        self
    }

    pub fn tvdb(mut self, id: u32) -> Self {
        self.tvdb_id = Some(id);
        self
    }

    pub fn season(mut self, season: u32) -> Self {
        self.season = Some(season);
        self
    }

    pub fn episode(mut self, episode: impl ToString) -> Self {
        self.episode = Some(episode.to_string());
        self
    }

    pub fn categories(mut self, categories: impl IntoIterator<Item = u32>) -> Self {
        self.categories = categories.into_iter().collect();
        self.categories.sort_unstable();
        self.categories.dedup();
        self
    }

    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Query-string parameters for this search, without the API key.
    pub(crate) fn params(&self, protocol: Protocol) -> Vec<(&'static str, String)> {
        let mut p = vec![("t", self.search_type.function().to_owned())];
        if let Some(q) = self
            .term
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
        {
            p.push(("q", q.to_owned()));
        }
        if let Some(id) = &self.imdb_id {
            let digits = id.trim().trim_start_matches("tt");
            // Newznab wants bare digits; Torznab (Jackett/Prowlarr) the tt form.
            let id = match protocol {
                Protocol::Newznab => digits.to_owned(),
                Protocol::Torznab => format!("tt{digits}"),
            };
            p.push(("imdbid", id));
        }
        if let Some(id) = self.tmdb_id {
            p.push(("tmdbid", id.to_string()));
        }
        if let Some(id) = self.tvdb_id {
            p.push(("tvdbid", id.to_string()));
        }
        if let Some(s) = self.season {
            p.push(("season", s.to_string()));
        }
        if let Some(e) = &self.episode {
            p.push(("ep", e.clone()));
        }
        if let Some(a) = &self.author {
            p.push(("author", a.clone()));
        }
        if let Some(t) = &self.title {
            p.push(("title", t.clone()));
        }
        if let Some(a) = &self.artist {
            p.push(("artist", a.clone()));
        }
        if let Some(a) = &self.album {
            p.push(("album", a.clone()));
        }
        if !self.categories.is_empty() {
            let cats: Vec<String> = self.categories.iter().map(u32::to_string).collect();
            p.push(("cat", cats.join(",")));
        }
        if let Some(l) = self.limit {
            p.push(("limit", l.to_string()));
        }
        if let Some(o) = self.offset {
            p.push(("offset", o.to_string()));
        }
        p.push(("extended", "1".to_owned()));
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tv_params() {
        let q = SearchQuery::tv("The Bear")
            .tvdb(412_457)
            .season(3)
            .episode(4)
            .categories([5040, 5000, 5040]);
        assert_eq!(
            q.params(Protocol::Newznab),
            vec![
                ("t", "tvsearch".into()),
                ("q", "The Bear".into()),
                ("tvdbid", "412457".into()),
                ("season", "3".into()),
                ("ep", "4".into()),
                ("cat", "5000,5040".into()),
                ("extended", "1".into()),
            ]
        );
    }

    #[test]
    fn imdb_form_depends_on_protocol() {
        let q = SearchQuery::movie("Dune").imdb("tt15239678");
        let get = |p: Protocol| {
            q.params(p)
                .into_iter()
                .find(|(k, _)| *k == "imdbid")
                .unwrap()
                .1
        };
        assert_eq!(get(Protocol::Newznab), "15239678");
        assert_eq!(get(Protocol::Torznab), "tt15239678");
    }

    #[test]
    fn music_params() {
        let q = SearchQuery::music(Some("Boards of Canada".into()), Some("Geogaddi".into()))
            .categories([3040]);
        let p = q.params(Protocol::Torznab);
        assert_eq!(p[0], ("t", "music".into()));
        assert!(p.contains(&("artist", "Boards of Canada".into())));
        assert!(p.contains(&("album", "Geogaddi".into())));
    }

    #[test]
    fn rss_has_no_term() {
        let p = SearchQuery::rss().params(Protocol::Torznab);
        assert!(!p.iter().any(|(k, _)| *k == "q"));
    }
}
