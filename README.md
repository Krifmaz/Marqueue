# Scoutline

A Rust indexer engine that replaces Prowlarr. It ships as a crate inside [Marqueue](https://github.com/Krifmaz/Marqueue) and will also run standalone with a Torznab/Newznab API.

Release names are parsed with [playbill](https://github.com/Krifmaz/playbill), the same parser Marqueue uses, so scoring matches the library.

## Try it with Docker

```sh
curl -O https://raw.githubusercontent.com/Krifmaz/Scoutline/HEAD/compose.yaml
docker compose up -d
```

1. The first start writes `./config/scoutline.toml` with a fresh `api_key`.
2. Add your indexers to that file as `[[indexer]]` blocks (examples are in the file), then `docker compose restart`.
3. Open http://localhost:9797, paste the `api_key`, and search.

Images: `ghcr.io/krifmaz/scoutline:latest` (amd64 and arm64), built from the default branch.

### Add it to Sonarr or Radarr

Add a **Torznab** indexer (for torrents) or a **Newznab** indexer (for Usenet):

| Field | Value |
| --- | --- |
| URL | `http://scoutline:9797/torznab/all` (or `/newznab/all`) |
| API Path | `/api` |
| API Key | the `api_key` from `scoutline.toml` |

Swap `all` for an indexer `id` to add a single indexer. Use `localhost` instead of `scoutline` if Sonarr isn't in the same compose network.

### Use it in place of Prowlarr (Encore and other apps)

Apps that call Prowlarr's own API can point at Scoutline unchanged: set the Prowlarr URL to `http://scoutline:9797` and the Prowlarr API key to Scoutline's `api_key`. For Encore:

```yaml
environment:
  - TORZNAB_ENABLED=true
  - PROWLARR_URL=http://scoutline:9797
  - PROWLARR_API_KEY=${SCOUTLINE_API_KEY}
```

Set the same key on Scoutline with `SCOUTLINE_API_KEY` (it overrides `api_key` in the file) and put it in a `.env` file next to the compose file.

### Sites without an API (YAML definitions)

Scoutline runs the Cardigann-style YAML definitions Prowlarr and Jackett use (login, search paths, CSS/JSON selectors, filters, category maps). Put `<id>.yml` files in `/config/definitions` and add the indexer:

```toml
[[indexer]]
id = "mytracker"
definition = "mytracker"     # reads /config/definitions/mytracker.yml
[indexer.settings]           # the settings the definition asks for
username = "me"
password = "secret"
```

None are bundled; the community definitions have no licence file, so get them yourself (e.g. from Prowlarr's Indexers repository, `definitions/v11`). All 579 current v11 definitions load and pass Scoutline's checks:

```sh
cargo run -p scoutline-defs --example check -- path/to/definitions/v11
cargo run -p scoutline-defs --example search -- path/to/definitions/v11 nyaasi "frieren"
```

Download links from sites that need a login go through Scoutline (`/dl/{id}`), which fetches them with its session; it only fetches links on that indexer's own site.

### Endpoints

| Path | What |
| --- | --- |
| `/` | Manual search UI |
| `/{torznab,newznab}/{all,id}/api` | Torznab/Newznab API (`t=caps`, `search`, `tvsearch`, `movie`, `music`, `book`) |
| `/api/v1/search` | Prowlarr-compatible search (`query`, `type`, `categories`, `indexerIds`, `limit`, `offset`), plus parsed `release` and all `sources` per result |
| `/api/v1/indexer` | Prowlarr-compatible indexer list, plus `health` |
| `/dl/{id}?link=...` | Download proxy for sites that need Scoutline's login session |
| `/health` | Liveness, no key needed |

Every endpoint except `/` and `/health` needs `apikey=` or an `X-Api-Key` header.

## Status

Phase 1 (native feeds) and phase 2 (definition engine) done, plus a preview of the standalone server.

- Newznab and Torznab indexers by URL + API key
- Search types: general/RSS, movie (IMDb/TMDB), TV (TVDB, season, episode), music (artist, album), book (author, title)
- Async fan-out across all indexers, per-site timeouts, results streamed as each site answers
- Short-lived query cache (2 minutes by default)
- Per-indexer health: failure history, stats, exponential backoff (1 min doubling to 3 h)
- Results parsed by playbill (hinted by category and search type) and de-duplicated across indexers (infohash, or title + size within 2%)
- API keys never show up in logs or error messages

## Crates

| Crate | Purpose |
| --- | --- |
| `scoutline-core` | Indexers, search, cache, health, results |
| `scoutline-net` | HTTP clients and per-indexer routes (direct for now) |
| `scoutline-defs` | YAML definition engine: templates, selectors, filters, logins |
| `scoutline-api` | `scoutline` server: Torznab/Newznab output, Prowlarr-compatible API, web UI |

Not yet: Cloudflare (FlareSolverr/byparr), proxies and WireGuard routes (phase 3), automatic definition updates, captcha logins.

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
