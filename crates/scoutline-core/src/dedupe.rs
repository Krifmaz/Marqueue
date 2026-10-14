use std::collections::HashMap;

use crate::indexer::Protocol;
use crate::result::SearchResult;

/// Sizes within this fraction of each other count as the same release.
/// Usenet indexers often report slightly different sizes for one post.
const SIZE_TOLERANCE: f64 = 0.02;

/// Merges results that are the same release found on several indexers.
///
/// Torrents match on infohash. Everything else matches on protocol,
/// normalized title and a size within 2%. Merged results keep every
/// origin, best first (most seeders, then most grabs), and are returned
/// newest first.
pub fn dedupe(results: impl IntoIterator<Item = SearchResult>) -> Vec<SearchResult> {
    let mut out: Vec<SearchResult> = Vec::new();
    let mut by_hash: HashMap<String, usize> = HashMap::new();
    let mut by_title: HashMap<(Protocol, String), Vec<usize>> = HashMap::new();

    for r in results {
        let title_key = (r.protocol, normalize(&r.title));
        let hit = r
            .infohash
            .as_ref()
            .and_then(|h| by_hash.get(h).copied())
            .or_else(|| {
                by_title.get(&title_key)?.iter().copied().find(|&i| {
                    let o = &out[i];
                    // Two different infohashes are two different torrents.
                    let hashes_clash =
                        matches!((&o.infohash, &r.infohash), (Some(a), Some(b)) if a != b);
                    !hashes_clash && sizes_match(o.size, r.size)
                })
            });

        let idx = match hit {
            Some(i) => {
                merge(&mut out[i], r);
                i
            }
            None => {
                out.push(r);
                out.len() - 1
            }
        };
        if let Some(h) = &out[idx].infohash {
            by_hash.entry(h.clone()).or_insert(idx);
        }
        let titles = by_title.entry(title_key).or_default();
        if !titles.contains(&idx) {
            titles.push(idx);
        }
    }

    for r in &mut out {
        r.origins
            .sort_by(|a, b| b.seeders.cmp(&a.seeders).then(b.grabs.cmp(&a.grabs)));
    }
    out.sort_by(|a, b| {
        b.published
            .cmp(&a.published)
            .then_with(|| a.title.cmp(&b.title))
    });
    out
}

fn merge(into: &mut SearchResult, from: SearchResult) {
    for o in from.origins {
        // The same indexer can list a release twice; keep one.
        if !into
            .origins
            .iter()
            .any(|x| x.indexer_id == o.indexer_id && x.guid == o.guid)
        {
            into.origins.push(o);
        }
    }
    into.size = into.size.or(from.size);
    into.published = match (into.published, from.published) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    into.infohash = into.infohash.take().or(from.infohash);
    into.magnet = into.magnet.take().or(from.magnet);
    into.imdb_id = into.imdb_id.take().or(from.imdb_id);
    into.tmdb_id = into.tmdb_id.or(from.tmdb_id);
    into.tvdb_id = into.tvdb_id.or(from.tvdb_id);
    for c in from.categories {
        if !into.categories.contains(&c) {
            into.categories.push(c);
        }
    }
}

/// Lowercase letters and digits only, so `The.Bear` and `The Bear` match.
fn normalize(title: &str) -> String {
    title
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn sizes_match(a: Option<u64>, b: Option<u64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (lo, hi) = (a.min(b) as f64, a.max(b) as f64);
            hi - lo <= hi * SIZE_TOLERANCE
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::Origin;

    fn result(
        indexer: &str,
        title: &str,
        size: u64,
        hash: Option<&str>,
        seeders: u32,
    ) -> SearchResult {
        SearchResult {
            title: title.into(),
            release: playbill::parse(title),
            protocol: if hash.is_some() {
                Protocol::Torznab
            } else {
                Protocol::Newznab
            },
            size: Some(size),
            published: None,
            categories: vec![5040],
            infohash: hash.map(str::to_owned),
            magnet: None,
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            origins: vec![Origin {
                indexer_id: indexer.into(),
                indexer_name: indexer.into(),
                guid: Some(format!("{indexer}-{title}")),
                download_url: None,
                info_url: None,
                seeders: Some(seeders),
                leechers: None,
                grabs: None,
                download_factor: None,
                upload_factor: None,
            }],
        }
    }

    #[test]
    fn merges_by_infohash_best_first() {
        let out = dedupe([
            result("a", "Show.S01E01.1080p-GRP", 1000, Some("abc"), 5),
            result("b", "Show S01E01 1080p GRP", 1000, Some("abc"), 50),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].best_origin().indexer_id, "b");
        assert_eq!(out[0].seeders(), Some(55));
    }

    #[test]
    fn merges_usenet_by_title_and_close_size() {
        let out = dedupe([
            result("a", "Show.S01E01.1080p-GRP", 1_000_000, None, 0),
            result("b", "Show.S01E01.1080p-GRP", 1_010_000, None, 0),
            result("c", "Show.S01E01.1080p-GRP", 2_000_000, None, 0),
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn different_hashes_stay_apart() {
        let out = dedupe([
            result("a", "Show.S01E01.1080p-GRP", 1000, Some("abc"), 1),
            result("b", "Show.S01E01.1080p-GRP", 1000, Some("def"), 1),
        ]);
        assert_eq!(out.len(), 2);
    }
}
