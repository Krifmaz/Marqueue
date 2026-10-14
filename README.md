# Scoutline

A Rust indexer engine that replaces Prowlarr. It ships as a crate inside [Marqueue](https://github.com/Krifmaz/Marqueue) and will also run standalone with a Torznab/Newznab API.

Release names are parsed with [playbill](https://github.com/Krifmaz/playbill), the same parser Marqueue uses, so scoring matches the library.

## Status

Phase 1 of 5: native feeds.

- Newznab and Torznab indexers by URL + API key
- Search types: general/RSS, movie (IMDb/TMDB), TV (TVDB, season, episode), book (author, title)
- Async fan-out across all indexers, per-site timeouts, results streamed as each site answers
- Short-lived query cache (2 minutes by default)
- Per-indexer health: failure history, stats, exponential backoff (1 min doubling to 3 h)
- Results parsed by playbill and de-duplicated across indexers (infohash, or title + size within 2%)
- API keys never show up in logs or error messages

## Crates

| Crate | Purpose |
| --- | --- |
| `scoutline-core` | Indexers, search, cache, health, results |
| `scoutline-net` | HTTP clients and per-indexer routes (direct for now) |

`scoutline-defs` (YAML definition engine) and `scoutline-api` (Torznab/Newznab output, standalone UI) arrive in phases 2 and 4.

## Usage

```rust
use scoutline_core::{Engine, IndexerConfig, Protocol, SearchQuery};

let engine = Engine::builder()
    .indexer(IndexerConfig::new("geek", "NZBgeek", Protocol::Newznab, "https://api.nzbgeek.info", "KEY"))
    .build()?;

// Everything, merged:
let out = engine.search(&SearchQuery::tv("The Bear").tvdb(412457).season(3).episode(4)).await;

// Or each site's results as they arrive:
let mut events = engine.search_stream(&SearchQuery::movie("Dune").imdb("tt15239678"));
```

## Tests

`cargo test` runs offline: feed parsing uses saved sample responses in `crates/scoutline-core/tests/fixtures`, and fan-out tests run against local mock indexers.
