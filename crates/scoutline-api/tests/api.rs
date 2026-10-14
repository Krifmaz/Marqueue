//! The HTTP API against a mock indexer.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scoutline_api::config::Config;
use scoutline_api::{AppState, router};
use scoutline_core::feed::parse_feed;
use tower::ServiceExt;
use wiremock::matchers::{method, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TORZNAB: &str = include_str!("../../scoutline-core/tests/fixtures/torznab.xml");

async fn app() -> (Router, MockServer) {
    let tracker = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("apikey", "TRACKER-KEY"))
        .respond_with(ResponseTemplate::new(200).set_body_string(TORZNAB))
        .mount(&tracker)
        .await;
    let cfg = Config::parse(&format!(
        r#"
        api_key = "SECRET"
        [[indexer]]
        id = "tracker"
        name = "Example Tracker"
        protocol = "torznab"
        url = "{}"
        api_key = "TRACKER-KEY"
        "#,
        tracker.uri()
    ))
    .unwrap();
    let state = AppState {
        engine: cfg.build_engine().unwrap(),
        api_key: cfg.api_key,
    };
    (router(state), tracker)
}

async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn health_and_ui_need_no_key() {
    let (app, _t) = app().await;
    assert_eq!(get(&app, "/health").await, (StatusCode::OK, "ok".into()));
    assert!(get(&app, "/").await.1.contains("<title>Scoutline</title>"));
}

#[tokio::test]
async fn torznab_feed_for_sonarr() {
    let (app, _t) = app().await;

    let (_, body) = get(&app, "/torznab/all/api?t=caps&apikey=wrong").await;
    assert!(body.contains(r#"code="100""#), "{body}");

    let (_, caps) = get(&app, "/torznab/all/api?t=caps&apikey=SECRET").await;
    assert!(caps.contains("<tv-search available=\"yes\""));

    for path in ["/torznab/all/api", "/torznab/tracker/api"] {
        let (status, body) = get(
            &app,
            &format!("{path}?t=tvsearch&q=The+Bear&season=3&ep=4&apikey=SECRET"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let items = parse_feed(&body).unwrap();
        assert_eq!(items.len(), 2, "{path}");
        assert_eq!(items[0].attr("seeders"), Some("120"));
        assert!(!body.contains("TRACKER-KEY"));
    }

    // Usenet feed has no torrent indexers in it.
    let (_, body) = get(&app, "/newznab/all/api?t=search&apikey=SECRET").await;
    assert!(parse_feed(&body).unwrap().is_empty());

    let (_, body) = get(&app, "/newznab/tracker/api?t=search&apikey=SECRET").await;
    assert!(body.contains(r#"code="300""#), "{body}");
}

#[tokio::test]
async fn prowlarr_api_as_encore_calls_it() {
    let (app, _t) = app().await;
    assert_eq!(
        get(&app, "/api/v1/search?query=bear").await.0,
        StatusCode::UNAUTHORIZED
    );

    // Exactly what Encore sends: X-Api-Key header, query/type/categories/limit.
    let req = Request::get("/api/v1/search?query=The+Bear&type=search&categories=3000&limit=100")
        .header("X-Api-Key", "SECRET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["x-scoutline-failures"], "[]");
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let r = &v.as_array().unwrap()[0];
    assert_eq!(
        r["title"],
        "The.Bear.S03E04.2160p.WEB-DL.DDP5.1.HDR.H.265-FLUX"
    );
    assert_eq!(r["protocol"], "torrent");
    assert_eq!(r["indexer"], "Example Tracker");
    assert_eq!(r["indexerId"], 1);
    assert_eq!(r["seeders"], 120);
    assert_eq!(r["size"], 6_442_450_944u64);
    assert_eq!(
        r["downloadUrl"],
        "https://tracker.example/dl/1001.torrent?passkey=PK"
    );
    assert!(
        r["magnetUrl"]
            .as_str()
            .unwrap()
            .starts_with("magnet:?xt=urn:btih:")
    );
    assert_eq!(r["publishDate"], "2026-10-13T08:00:00Z");
    assert_eq!(r["categories"][1]["name"], "TV/UHD");
    assert_eq!(r["release"]["title"], "The Bear");

    let (_, body) = get(&app, "/api/v1/indexer?apikey=SECRET").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v[0]["id"], 1);
    assert_eq!(v[0]["slug"], "tracker");
    assert_eq!(v[0]["enable"], true);
    assert_eq!(v[0]["protocol"], "torrent");
    assert_eq!(v[0]["health"]["queries"], 1);
    assert!(!body.contains("TRACKER-KEY"));

    // -1 is Prowlarr's "all Usenet": none here.
    let (_, body) = get(
        &app,
        "/api/v1/search?query=bear&indexerIds=-1&apikey=SECRET",
    )
    .await;
    assert_eq!(body, "[]");
}

#[tokio::test]
async fn music_search_reaches_indexer() {
    let (app, tracker) = app().await;
    Mock::given(method("GET"))
        .and(query_param("t", "music"))
        .and(query_param("artist", "Boards of Canada"))
        .respond_with(ResponseTemplate::new(200).set_body_string(TORZNAB))
        .with_priority(1)
        .expect(1)
        .mount(&tracker)
        .await;
    let (_, body) = get(
        &app,
        "/torznab/all/api?t=music&artist=Boards+of+Canada&album=Geogaddi&apikey=SECRET",
    )
    .await;
    assert_eq!(parse_feed(&body).unwrap().len(), 2);
}
