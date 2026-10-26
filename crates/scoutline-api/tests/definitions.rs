//! A definition-based private tracker served through the Torznab API, with
//! downloads proxied through Scoutline's login session.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scoutline_api::config::Config;
use scoutline_api::{AppState, router};
use scoutline_core::feed::parse_feed;
use tower::ServiceExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scoutline-defs/tests/fixtures"
);

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
}

fn html(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/html")
        .set_body_string(body)
}

#[tokio::test]
async fn private_tracker_through_torznab_with_download_proxy() {
    let site = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/login.php"))
        .respond_with(html(fixture("login.html")))
        .mount(&site)
        .await;
    Mock::given(method("POST"))
        .and(path("/takelogin.php"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "/index.php")
                .insert_header("set-cookie", "session=abc; Path=/"),
        )
        .mount(&site)
        .await;
    for (p, body) in [
        ("/index.php", fixture("index.html")),
        ("/browse.php", fixture("browse.html")),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .and(header("cookie", "session=abc"))
            .respond_with(html(body))
            .mount(&site)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/download.php"))
        .and(header("cookie", "session=abc"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-bittorrent")
                .set_body_bytes(b"d8:announce3:urle".to_vec()),
        )
        .mount(&site)
        .await;

    let cfg = Config::parse(&format!(
        r#"
        api_key = "SECRET"
        definitions_dir = "{FIXTURES}/defs"
        [[indexer]]
        id = "ex"
        definition = "example-html"
        url = "{}"
        [indexer.settings]
        username = "alice"
        password = "pw"
        "#,
        site.uri()
    ))
    .unwrap();
    let app = router(AppState {
        engine: cfg.build_engine().unwrap(),
        api_key: cfg.api_key,
    });

    let get = |uri: String| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::get(uri)
                        .header("host", "scoutline:9797")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = resp.status();
            let ct = resp
                .headers()
                .get("content-type")
                .map(|v| v.to_str().unwrap().to_owned());
            let body = resp
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec();
            (status, ct, body)
        }
    };

    let (_, _, body) =
        get("/torznab/ex/api?t=tvsearch&q=The+Bear&season=3&ep=4&apikey=SECRET".into()).await;
    let xml = String::from_utf8(body).unwrap();
    let items = parse_feed(&xml).unwrap();
    assert_eq!(items.len(), 2, "{xml}");
    let link = items
        .iter()
        .find(|i| i.title.starts_with("The.Bear"))
        .unwrap()
        .enclosure_url
        .clone()
        .unwrap();
    assert!(
        link.starts_with("http://scoutline:9797/dl/ex?apikey=SECRET&link="),
        "{link}"
    );
    assert!(
        !xml.contains("passkey=PK\""),
        "tracker links are not handed out directly"
    );

    // The *arr app fetches the proxied link: Scoutline downloads it with the session.
    let (status, ct, body) = get(link.trim_start_matches("http://scoutline:9797").to_owned()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ct.as_deref(), Some("application/x-bittorrent"));
    assert_eq!(body, b"d8:announce3:urle");

    // Not for other sites, and not without the key.
    let (status, _, _) =
        get("/dl/ex?apikey=SECRET&link=http%3A%2F%2F169.254.169.254%2Fx".into()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let (status, _, _) = get(format!("/dl/ex?link={}", "x")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
