# Marqueue

One self-hosted Rust program to request movies, TV, anime, ebooks and audiobooks, then find, download, import and upgrade them. Replaces Sonarr, Radarr, Readarr and Overseerr/Jellyseerr.

Release names are parsed by [Playbill](https://github.com/Krifmaz/playbill).

## Workspace

| Crate | Role |
| --- | --- |
| `marqueue-server` | The `marqueue` binary: axum API and web app |
| `marqueue-core` | Media model, quality profiles, release scoring |
| `marqueue-metadata` | Metadata provider trait and local cache |
| `marqueue-adapters` | Traits for indexers, download clients, media servers, notifiers |
| `scoutline` | Built-in indexer engine (phase 4) |

## Run

```
cargo run --bin marqueue
curl 'localhost:7979/api/v1/parse?name=The.Bear.S03E04.2160p.WEB-DL.DDP5.1.HDR.H.265-FLUX'
```

`MARQUEUE_LISTEN` sets the address (default `0.0.0.0:7979`). Docker: `docker buildx build --platform linux/amd64,linux/arm64 .`

## Status

Phase 1 (foundation). See the spec for the full plan.
