use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use scoutline_core::health::IndexerHealth;
use scoutline_core::{Engine, Protocol, Scope, SearchResult};
use serde::{Deserialize, Serialize};

use crate::torznab::{self, Params, Request};

const UI: &str = include_str!("ui.html");

/// Shared by every request.
pub struct AppState {
    pub engine: Engine,
    pub api_key: String,
}

impl AppState {
    fn authorized(&self, query_key: Option<&str>, headers: &HeaderMap) -> bool {
        let given = query_key.or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()));
        given.is_some_and(|k| constant_time_eq(k.as_bytes(), self.api_key.as_bytes()))
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(|| async { Html(UI) }))
        .route("/health", get(|| async { "ok" }))
        .route("/api/v1/indexers", get(indexers))
        .route("/api/v1/search", get(search))
        .route("/{protocol}/{id}/api", get(feed))
        .route("/{protocol}/{id}", get(feed))
        .with_state(Arc::new(state))
}

/// `/{torznab|newznab}/{id|all}/api`: what Sonarr and Radarr talk to.
async fn feed(
    State(state): State<Arc<AppState>>,
    Path((protocol, id)): Path<(String, String)>,
    Query(params): Query<Params>,
    headers: HeaderMap,
) -> Response {
    let xml = |status: StatusCode, body: String| {
        (
            status,
            [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
            body,
        )
            .into_response()
    };
    // *arr apps only read Newznab errors from 200 responses.
    let fail = |code, msg: &str| xml(StatusCode::OK, torznab::error(code, msg));

    let protocol = match protocol.as_str() {
        "torznab" => Protocol::Torznab,
        "newznab" => Protocol::Newznab,
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    if !state.authorized(params.apikey.as_deref(), &headers) {
        return fail(torznab::ERR_CREDENTIALS, "Invalid API key");
    }
    let (scope, title) = if id == "all" {
        (
            Scope::Protocol(protocol),
            "Scoutline (all indexers)".to_owned(),
        )
    } else {
        match state
            .engine
            .indexers()
            .find(|i| i.id == id && i.protocol == protocol)
        {
            Some(i) => (
                Scope::Indexer(i.id.clone()),
                format!("Scoutline: {}", i.name),
            ),
            None => {
                return fail(
                    torznab::ERR_NO_ITEM,
                    &format!("no {protocol:?} indexer called {id}"),
                );
            }
        }
    };
    match params.request() {
        Ok(Request::Caps) => xml(StatusCode::OK, torznab::caps(&title)),
        Ok(Request::Search(q)) => {
            let out = state.engine.search_in(&scope, &q).await;
            xml(
                StatusCode::OK,
                torznab::feed(&title, protocol, &out.results),
            )
        }
        Err((code, msg)) => fail(code, &msg),
    }
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(flatten)]
    torznab: Params,
    /// Limit to one indexer.
    indexer: Option<String>,
    /// Limit to `torznab` or `newznab` indexers.
    protocol: Option<Protocol>,
}

#[derive(Serialize)]
struct SearchResponse<'a> {
    results: &'a [SearchResult],
    errors: Vec<IndexerFailure>,
}

#[derive(Serialize)]
struct IndexerFailure {
    indexer: String,
    error: String,
}

async fn search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Response {
    if !state.authorized(params.torznab.apikey.as_deref(), &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid API key" })),
        )
            .into_response();
    }
    let query = match params.torznab.request() {
        Ok(Request::Search(q)) => q,
        Ok(Request::Caps) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "use the Torznab endpoint for caps" })),
            )
                .into_response();
        }
        Err((_, msg)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response();
        }
    };
    let scope = match (params.indexer.filter(|i| !i.is_empty()), params.protocol) {
        (Some(id), _) => Scope::Indexer(id),
        (None, Some(p)) => Scope::Protocol(p),
        (None, None) => Scope::All,
    };

    let out = state.engine.search_in(&scope, &query).await;
    Json(SearchResponse {
        results: &out.results,
        errors: out
            .errors
            .iter()
            .map(|(indexer, e)| IndexerFailure {
                indexer: indexer.clone(),
                error: e.to_string(),
            })
            .collect(),
    })
    .into_response()
}

/// An indexer as the UI sees it: no API key.
#[derive(Serialize)]
struct IndexerInfo {
    id: String,
    name: String,
    protocol: Protocol,
    enabled: bool,
    health: Option<IndexerHealth>,
}

#[derive(Deserialize)]
struct KeyParam {
    apikey: Option<String>,
}

async fn indexers(
    State(state): State<Arc<AppState>>,
    Query(k): Query<KeyParam>,
    headers: HeaderMap,
) -> Response {
    if !state.authorized(k.apikey.as_deref(), &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid API key" })),
        )
            .into_response();
    }
    let list: Vec<IndexerInfo> = state
        .engine
        .indexers()
        .map(|i| IndexerInfo {
            id: i.id.clone(),
            name: i.name.clone(),
            protocol: i.protocol,
            enabled: i.enabled,
            health: state.engine.health().get(&i.id),
        })
        .collect();
    Json(list).into_response()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
