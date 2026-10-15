//! The Marqueue binary: one process serving the API and web app.

mod config;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use marqueue_adapters::{Indexer, Protocol, SearchQuery};
use marqueue_core::MediaType;
use scoutline::Scoutline;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

use crate::config::Config;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Environment variables seed the first start; everything else is set in the UI.
    let addr: SocketAddr = std::env::var("MARQUEUE_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:7979".to_string())
        .parse()
        .expect("MARQUEUE_LISTEN must be host:port");

    let config_path = PathBuf::from(
        std::env::var("MARQUEUE_CONFIG").unwrap_or_else(|_| config::DEFAULT_PATH.to_string()),
    );
    let config = Config::load_or_create(&config_path).unwrap_or_else(|e| {
        tracing::error!("{e}");
        std::process::exit(1);
    });
    let scoutline = Scoutline::new(&config.indexers, &config.definitions_dir).unwrap_or_else(|e| {
        tracing::error!("indexers: {e}");
        std::process::exit(1);
    });
    if scoutline.indexer_count() == 0 {
        tracing::warn!(path = %config_path.display(), "no indexers yet; add [[indexer]] blocks and restart");
    } else {
        tracing::info!(indexers = scoutline.indexer_count(), "Scoutline ready");
    }

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listen address");
    tracing::info!("marqueue listening on {addr}");
    axum::serve(listener, app(Arc::new(scoutline)))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");
}

fn app(scoutline: Arc<Scoutline>) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/parse", get(parse))
        .route("/api/v1/search", get(search))
        .route("/api/v1/indexers", get(indexers))
        .with_state(scoutline)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Deserialize)]
struct ParseParams {
    name: String,
    #[serde(rename = "type")]
    media_type: Option<MediaType>,
}

/// Parses a release name, like Sonarr/Radarr's `/api/v3/parse`.
async fn parse(Query(p): Query<ParseParams>) -> Json<playbill::Release> {
    let hint = p.media_type.map(MediaType::hint).unwrap_or_default();
    Json(playbill::parse_with_hint(&p.name, hint))
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(rename = "type")]
    media_type: MediaType,
    title: String,
    year: Option<u16>,
    season: Option<u16>,
    episode: Option<u16>,
    absolute: Option<u16>,
}

/// One candidate. Download URLs carry tracker passkeys, so they stay on
/// the server.
#[derive(Serialize)]
struct CandidateOut {
    indexer: String,
    protocol: Protocol,
    size_bytes: Option<u64>,
    seeders: Option<u32>,
    release: playbill::Release,
}

/// Searches every indexer through the embedded Scoutline engine.
async fn search(
    State(scoutline): State<Arc<Scoutline>>,
    Query(p): Query<SearchParams>,
) -> Response {
    let query = SearchQuery {
        media_type: p.media_type,
        title: p.title,
        year: p.year,
        season: p.season,
        episode: p.episode,
        absolute_episode: p.absolute,
    };
    match scoutline.search(&query).await {
        Ok(found) => Json(
            found
                .into_iter()
                .map(|c| CandidateOut {
                    indexer: c.indexer,
                    protocol: c.protocol,
                    size_bytes: c.size_bytes,
                    seeders: c.seeders,
                    release: c.release,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.0 })),
        )
            .into_response(),
    }
}

#[derive(Serialize)]
struct IndexerOut {
    id: String,
    name: String,
    enabled: bool,
    health: Option<scoutline::health::IndexerHealth>,
}

/// Configured indexers and their health. No API keys.
async fn indexers(State(scoutline): State<Arc<Scoutline>>) -> Json<Vec<IndexerOut>> {
    let engine = scoutline.engine();
    Json(
        engine
            .indexers()
            .map(|i| IndexerOut {
                id: i.id.clone(),
                name: i.name.clone(),
                enabled: i.enabled,
                health: engine.health().get(&i.id),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    fn test_app() -> Router {
        app(Arc::new(
            Scoutline::new(&[], std::path::Path::new("/nonexistent")).unwrap(),
        ))
    }

    async fn get_json(uri: &str) -> serde_json::Value {
        let res = test_app()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(res.status().is_success(), "{uri}: {}", res.status());
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn health_ok() {
        assert_eq!(get_json("/api/v1/health").await["status"], "ok");
    }

    #[tokio::test]
    async fn parse_endpoint() {
        let v =
            get_json("/api/v1/parse?name=The.Bear.S03E04.2160p.WEB-DL.DDP5.1.HDR.H.265-FLUX").await;
        assert_eq!(v["title"], "The Bear");
        assert_eq!(v["seasons"][0], 3);
        assert_eq!(v["release_group"], "FLUX");
    }

    #[tokio::test]
    async fn parse_endpoint_with_type_hint() {
        let v = get_json("/api/v1/parse?name=One.Piece.1089.1080p.WEB.H264-GROUP&type=anime").await;
        assert_eq!(v["absolute_episodes"][0], 1089);
    }

    #[tokio::test]
    async fn search_through_embedded_scoutline() {
        use wiremock::matchers::{method, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let tracker = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("t", "tvsearch"))
            .and(query_param("season", "3"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
                <item><title>The.Bear.S03E04.2160p.WEB-DL.DDP5.1.HDR.H.265-FLUX</title>
                <link>https://t.example/dl/1.torrent?passkey=PK</link><size>6442450944</size>
                <torznab:attr name="seeders" value="120"/></item></channel></rss>"#,
            ))
            .mount(&tracker)
            .await;
        let config = Config::parse(&format!(
            "[[indexer]]\nid = \"t\"\nname = \"Tracker\"\nprotocol = \"torznab\"\nurl = \"{}\"\napi_key = \"k\"",
            tracker.uri()
        ))
        .unwrap();
        let app = app(Arc::new(
            Scoutline::new(&config.indexers, &config.definitions_dir).unwrap(),
        ));

        let res = app
            .clone()
            .oneshot(
                Request::get("/api/v1/search?type=tv&title=The+Bear&season=3&episode=4")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success());
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            !text.contains("passkey"),
            "download URLs must stay on the server"
        );
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v[0]["indexer"], "Tracker");
        assert_eq!(v[0]["protocol"], "torrent");
        assert_eq!(v[0]["seeders"], 120);
        assert_eq!(v[0]["release"]["title"], "The Bear");

        let res = app
            .oneshot(
                Request::get("/api/v1/indexers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v[0]["id"], "t");
        assert_eq!(v[0]["health"]["queries"], 1);
    }
}
