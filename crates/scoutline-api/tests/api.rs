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
async fn json_api() {
    let (app, _t) = app().await;
    assert_eq!(
        get(&app, "/api/v1/search?q=bear").await.0,
        StatusCode::UNAUTHORIZED
    );

    let (status, body) = get(&app, "/api/v1/search?t=tvsearch&q=The+Bear&apikey=SECRET").await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["results"].as_array().unwrap().len(), 2);
    assert_eq!(v["results"][0]["release"]["title"], "The Bear");
    assert_eq!(v["results"][0]["release"]["resolution"], "2160p");

    let (_, body) = get(&app, "/api/v1/indexers?apikey=SECRET").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v[0]["id"], "tracker");
    assert_eq!(v[0]["health"]["queries"], 1);
    assert!(!body.contains("TRACKER-KEY"));
}
