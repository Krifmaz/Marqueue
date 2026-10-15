//! Definitions run end to end against mock sites: login, search, parsing,
//! download proxy.

use std::collections::BTreeMap;
use std::path::PathBuf;

use scoutline_core::settings::{IndexerSettings, SettingValue};
use scoutline_core::{Engine, SearchQuery};
use scoutline_net::NetSettings;
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn defs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/defs")
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn html(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/html; charset=utf-8")
        .set_body_string(body)
}

/// A tracker that wants a form login with a CSRF token and a session cookie.
async fn tracker(password: &str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/login.php"))
        .respond_with(html(fixture("login.html")))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/takelogin.php"))
        .and(body_string_contains("csrf=tok-123"))
        .and(body_string_contains("username=alice"))
        .and(body_string_contains(format!("password={password}")))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "/index.php")
                .insert_header("set-cookie", "session=abc; Path=/"),
        )
        .with_priority(1)
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/takelogin.php"))
        .respond_with(html(
            "<div class=\"login-error\">Username or password incorrect</div>".into(),
        ))
        .mount(&s)
        .await;

    // Logged-in pages need the cookie; otherwise the site bounces to login.
    for (p, body) in [
        ("/index.php", fixture("index.html")),
        ("/browse.php", fixture("browse.html")),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .and(header("cookie", "session=abc"))
            .respond_with(html(body))
            .with_priority(1)
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/login.php"))
            .mount(&s)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/download.php"))
        .and(query_param("id", "101"))
        .and(header("cookie", "session=abc"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-bittorrent")
                .set_body_bytes(b"d8:announce3:urle".to_vec()),
        )
        .mount(&s)
        .await;
    s
}

fn html_settings(server: &MockServer, password: &str) -> IndexerSettings {
    let mut settings = BTreeMap::new();
    settings.insert("username".into(), SettingValue::Text("alice".into()));
    settings.insert("password".into(), SettingValue::Text(password.into()));
    IndexerSettings {
        id: "ex".into(),
        definition: Some("example-html".into()),
        url: server.uri(),
        settings,
        ..Default::default()
    }
}

fn engine(settings: Vec<IndexerSettings>) -> Engine {
    let indexers =
        scoutline_defs::build_indexers(&settings, &defs_dir(), &NetSettings::default()).unwrap();
    Engine::builder()
        .indexers(indexers)
        .cache_ttl(std::time::Duration::ZERO)
        .build()
        .unwrap()
}

#[tokio::test]
async fn html_tracker_with_form_login() {
    let site = tracker("secret").await;
    let engine = engine(vec![html_settings(&site, "secret")]);
    assert_eq!(
        engine.indexers().next().unwrap().name,
        "Example HTML Tracker"
    );

    let q = SearchQuery::tv("The.Bear")
        .season(3)
        .episode(4)
        .categories([5000]);
    let out = engine.search(&q).await;
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(out.results.len(), 2, "the advert row is skipped");

    // The request carried the rendered inputs.
    let requests = site.received_requests().await.unwrap();
    let search = requests
        .iter()
        .find(|r| r.url.path() == "/browse.php" && r.headers.get("cookie").is_some())
        .unwrap();
    let qs = search.url.query().unwrap();
    assert!(qs.contains("search=The+Bear+S03E04"), "{qs}");
    assert!(qs.contains("sort=added"), "{qs}");
    assert!(qs.contains("c2=1&c3=1"), "{qs}");
    assert!(!qs.contains("fl=1"), "{qs}");

    let bear = out
        .results
        .iter()
        .find(|r| r.title.starts_with("The.Bear"))
        .unwrap();
    assert_eq!(bear.release.title.as_deref(), Some("The Bear"));
    assert_eq!(bear.size, Some(6 * 1024 * 1024 * 1024));
    assert_eq!(bear.categories, vec![5000, 5040]);
    assert_eq!(bear.imdb_id.as_deref(), Some("tt14452776"));
    let o = bear.best_origin();
    assert_eq!(o.seeders, Some(1204));
    assert_eq!(o.leechers, Some(35));
    assert_eq!(o.grabs, Some(812));
    assert_eq!(o.download_factor, Some(0.0), "freeleech");
    assert_eq!(
        o.download_url.as_deref(),
        Some(format!("{}/download.php?id=101&passkey=PK", site.uri()).as_str())
    );
    assert_eq!(
        o.info_url.as_deref(),
        Some(format!("{}/details.php?id=101", site.uri()).as_str())
    );
    let age = chrono::Utc::now() - bear.published.unwrap();
    assert!((age.num_minutes() - 120).abs() < 2, "{age:?}");

    let anime = out
        .results
        .iter()
        .find(|r| r.title.contains("Frieren"))
        .unwrap();
    assert_eq!(anime.categories, vec![5000, 5070]);
    assert_eq!(anime.release.absolute_episodes, vec![28]);
    assert_eq!(anime.best_origin().download_factor, Some(1.0));

    // A second search reuses the session: no new login.
    engine.search(&q).await;
    let logins = site
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/takelogin.php")
        .count();
    assert_eq!(logins, 1);

    // Downloads go through the session.
    let backend = engine.indexers().next().unwrap().backend.clone().unwrap();
    assert!(backend.needs_download_proxy());
    let dl = backend
        .download(o.download_url.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(dl.bytes, b"d8:announce3:urle");
    assert!(
        backend
            .download("https://elsewhere.example/x.torrent")
            .await
            .is_err(),
        "only this site's links"
    );
}

#[tokio::test]
async fn wrong_password_shows_the_sites_message() {
    let site = tracker("secret").await;
    let engine = engine(vec![html_settings(&site, "wrong")]);
    let out = engine.search(&SearchQuery::general("x")).await;
    let err = out.errors[0].1.to_string();
    assert!(err.contains("Username or password incorrect"), "{err}");
}

#[tokio::test]
async fn json_site_with_nested_rows() {
    let site = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/search"))
        .and(header("x-api-key", "K"))
        .and(query_param("q", "Boards of Canada Geogaddi"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("search.json")))
        .mount(&site)
        .await;

    let mut settings = BTreeMap::new();
    settings.insert("apikey".into(), SettingValue::Text("K".into()));
    let engine = engine(vec![IndexerSettings {
        id: "js".into(),
        definition: Some("example-json".into()),
        url: site.uri(),
        settings,
        ..Default::default()
    }]);

    let q = SearchQuery::music(Some("Boards of Canada".into()), Some("Geogaddi".into()));
    let out = engine.search(&q).await;
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(out.results.len(), 2);
    let flac = out
        .results
        .iter()
        .find(|r| r.title.contains("FLAC"))
        .unwrap();
    assert_eq!(flac.title, "Boards of Canada - Geogaddi (2002) [FLAC]");
    assert_eq!(
        flac.infohash.as_deref(),
        Some("abcdef0123456789abcdef0123456789abcdef01")
    );
    assert_eq!(flac.size, Some(524_288_000));
    assert_eq!(flac.categories, vec![3000, 3040]);
    assert_eq!(flac.best_origin().download_factor, Some(0.0));
    assert_eq!(
        flac.published.unwrap().to_rfc3339(),
        "2025-10-13T08:00:00+00:00"
    );
    let mp3 = out
        .results
        .iter()
        .find(|r| r.title.contains("MP3"))
        .unwrap();
    assert_eq!(mp3.best_origin().download_factor, Some(1.0));
}

#[test]
fn saved_page_parses_offline() {
    // The check every definition update must pass: parse a recorded page.
    let def =
        std::sync::Arc::new(scoutline_defs::load_definition(&defs_dir(), "example-html").unwrap());
    let idx = scoutline_defs::DefinitionIndexer::new(
        def,
        Some("https://tracker.example/"),
        &BTreeMap::new(),
        &NetSettings::default(),
        &Default::default(),
    )
    .unwrap();
    let items = idx.parse_results(&fixture("browse.html"), None).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0].enclosure_url.as_deref(),
        Some("https://tracker.example/download.php?id=101&passkey=PK")
    );
}

#[test]
fn missing_definition_is_a_clear_error() {
    let err = scoutline_defs::build_indexers(
        &[IndexerSettings {
            id: "x".into(),
            definition: Some("nope".into()),
            ..Default::default()
        }],
        &defs_dir(),
        &NetSettings::default(),
    )
    .unwrap_err();
    assert!(err.contains("definition \"nope\" not found"), "{err}");
}
