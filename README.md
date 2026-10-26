# Marqueue

One self-hosted Rust program to request movies, TV, anime, ebooks and audiobooks, then find, download, import and upgrade them. Replaces Sonarr, Radarr, Readarr and Overseerr/Jellyseerr.

Release names are parsed by [playbill](crates/playbill).

## Workspace

| Crate | Role |
| --- | --- |
| `marqueue-server` | The `marqueue` binary: axum API and web app |
| `marqueue-core` | Media model, quality profiles, release scoring |
| `marqueue-metadata` | Metadata provider trait and local cache |
| `marqueue-adapters` | Traits for indexers, download clients, media servers, notifiers |
| `scoutline` | Built-in indexer engine: Scoutline embedded, no Prowlarr needed |

## Run

```
cargo run --bin marqueue
curl 'localhost:7979/api/v1/parse?name=The.Bear.S03E04.2160p.WEB-DL.DDP5.1.HDR.H.265-FLUX'
```

`MARQUEUE_LISTEN` sets the address (default `0.0.0.0:7979`). `MARQUEUE_CONFIG` sets the config file (default `/config/marqueue.toml`); the first start writes a commented one.

## Indexers

Marqueue searches indexers itself through the embedded Scoutline engine: every indexer at once, results parsed and de-duplicated, failing sites backed off. Add them to `marqueue.toml` and restart:

```toml
[[indexer]]
id = "nzbgeek"
name = "NZBgeek"
protocol = "newznab"            # or "torznab"
url = "https://api.nzbgeek.info"
api_key = "YOUR-INDEXER-KEY"
```

These are the same blocks as a standalone `scoutline.toml`, so they can be copied across.

Sites without an API run through the YAML definitions Prowlarr and Jackett use. Put `<id>.yml` in `/config/definitions` (`definitions_dir` changes it) and add:

```toml
[[indexer]]
id = "mytracker"
definition = "mytracker"
[indexer.settings]
username = "me"
password = "secret"
```

| Endpoint | What |
| --- | --- |
| `GET /api/v1/search?type=tv&title=The+Bear&season=3&episode=4` | Search all indexers (`type`: movie, tv, anime, ebook, audiobook; also `year`, `absolute`) |
| `GET /api/v1/indexers` | Indexers and their health |

## Docker

Images for amd64 and arm64 are published to `ghcr.io/krifmaz/marqueue` on every push.

```
docker compose up -d
curl localhost:7979/api/v1/health
```

## Status

Phase 1 (foundation). See the spec for the full plan.
