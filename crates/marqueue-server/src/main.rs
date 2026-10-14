//! The Marqueue binary: one process serving the API and web app.

use std::net::SocketAddr;

use axum::{Json, Router, extract::Query, routing::get};
use marqueue_core::MediaType;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

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

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listen address");
    tracing::info!("marqueue listening on {addr}");
    axum::serve(listener, app())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");
}

fn app() -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/parse", get(parse))
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn get_json(uri: &str) -> serde_json::Value {
        let res = app()
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
}
