use std::sync::Arc;

use axum::extract::{Path, Query, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use scoutline_core::{Engine, Protocol, Scope};

use crate::prowlarr;
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
        .route("/api/v1/indexer", get(indexers))
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
                Scope::Indexers(vec![i.id.clone()]),
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

/// The API key from `?apikey=` or `X-Api-Key`.
fn query_key(raw: Option<&str>) -> Option<String> {
    url::form_urlencoded::parse(raw.unwrap_or("").as_bytes())
        .find(|(k, _)| k == "apikey")
        .map(|(_, v)| v.into_owned())
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "message": msg }))).into_response()
}

/// Prowlarr-compatible search. Indexers that failed are listed in the
/// `X-Scoutline-Failures` header as JSON, since Prowlarr's body is a bare
/// array.
async fn search(
    State(state): State<Arc<AppState>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    if !state.authorized(query_key(raw.as_deref()).as_deref(), &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "Invalid API key");
    }
    let params = match prowlarr::SearchParams::parse(raw.as_deref().unwrap_or("")) {
        Ok(p) => p,
        Err(msg) => return json_error(StatusCode::BAD_REQUEST, &msg),
    };
    let out = state
        .engine
        .search_in(&params.scope(&state.engine), &params.query())
        .await;
    let releases: Vec<_> = out
        .results
        .iter()
        .map(|r| prowlarr::release(r, &state.engine))
        .collect();
    let failures: Vec<_> = out
        .errors
        .iter()
        .map(|(indexer, e)| serde_json::json!({ "indexer": indexer, "error": e.to_string() }))
        .collect();

    let mut resp = Json(releases).into_response();
    if let Ok(v) = HeaderValue::from_str(&serde_json::Value::from(failures).to_string()) {
        resp.headers_mut().insert("x-scoutline-failures", v);
    }
    resp
}

async fn indexers(
    State(state): State<Arc<AppState>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    if !state.authorized(query_key(raw.as_deref()).as_deref(), &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "Invalid API key");
    }
    Json(prowlarr::indexers(&state.engine)).into_response()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
