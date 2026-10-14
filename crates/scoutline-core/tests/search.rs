//! Fan-out search against mock indexers.

use std::time::Duration;

use futures::StreamExt;
use scoutline_core::{Engine, IndexerConfig, IndexerError, IndexerEvent, Protocol, SearchQuery};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TORZNAB: &str = include_str!("fixtures/torznab.xml");
const NEWZNAB: &str = include_str!("fixtures/newznab.xml");

async fn serve(body: &'static str, expect: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "tvsearch"))
        .and(query_param("apikey", "KEY"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .expect(expect)
        .mount(&server)
        .await;
    server
}

fn indexer(id: &str, protocol: Protocol, server: &MockServer) -> IndexerConfig {
    IndexerConfig::new(id, id, protocol, server.uri(), "KEY")
}

#[tokio::test]
async fn fans_out_merges_and_caches() {
    let tracker_a = serve(TORZNAB, 1).await;
    let tracker_b = serve(TORZNAB, 1).await;
    let usenet = serve(NEWZNAB, 1).await;

    let engine = Engine::builder()
        .indexer(indexer("a", Protocol::Torznab, &tracker_a))
        .indexer(indexer("b", Protocol::Torznab, &tracker_b))
        .indexer(indexer("u", Protocol::Newznab, &usenet))
        .build()
        .unwrap();

    let q = SearchQuery::tv("The Bear").season(3).episode(4);
    let out = engine.search(&q).await;
    assert!(out.errors.is_empty(), "{:?}", out.errors);

    // Two torrents (merged across a and b) plus one NZB.
    assert_eq!(out.results.len(), 3);
    let uhd = out.results.iter().find(|r| r.infohash.is_some()).unwrap();
    assert_eq!(uhd.origins.len(), 2);
    assert_eq!(uhd.seeders(), Some(240));
    assert_eq!(uhd.release.title.as_deref(), Some("The Bear"));
    // Newest first.
    assert_eq!(
        out.results[0].title,
        "The.Bear.S03E04.2160p.WEB-DL.DDP5.1.HDR.H.265-FLUX"
    );

    // Second search is served from cache; `expect(1)` checks on drop.
    let again = engine.search(&q).await;
    assert_eq!(again.results.len(), 3);
}

#[tokio::test]
async fn slow_and_broken_sites_fail_alone() {
    let good = serve(TORZNAB, 1).await;

    let slow = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(TORZNAB)
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&slow)
        .await;

    let broken = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).set_body_string("<html>down</html>"))
        .mount(&broken)
        .await;

    let bad_key = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<?xml version="1.0"?><error code="100" description="Incorrect user credentials"/>"#,
        ))
        .mount(&bad_key)
        .await;

    let engine = Engine::builder()
        .indexer(indexer("good", Protocol::Torznab, &good))
        .indexer(indexer("slow", Protocol::Torznab, &slow).timeout(Duration::from_millis(200)))
        .indexer(indexer("broken", Protocol::Torznab, &broken))
        .indexer(indexer("bad-key", Protocol::Newznab, &bad_key))
        .cache_ttl(Duration::ZERO)
        .build()
        .unwrap();

    let started = std::time::Instant::now();
    let events: Vec<IndexerEvent> = engine
        .search_stream(&SearchQuery::tv("The Bear"))
        .collect()
        .await;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "timeout didn't cut off the slow site"
    );
    assert!(events.iter().any(|e| matches!(e, IndexerEvent::Results { indexer, results } if indexer == "good" && results.len() == 2)));

    let error_of = |id: &str| {
        events.iter().find_map(|e| match e {
            IndexerEvent::Failed { indexer, error } if indexer == id => Some(error.clone()),
            _ => None,
        })
    };
    assert!(matches!(error_of("slow"), Some(IndexerError::Timeout(_))));
    assert!(matches!(
        error_of("broken"),
        Some(IndexerError::Status(503))
    ));
    assert!(matches!(
        error_of("bad-key"),
        Some(IndexerError::Api { code: 100, .. })
    ));

    let h = engine.health().get("broken").unwrap();
    assert_eq!(h.consecutive_failures, 1);
    assert!(engine.health().get("good").unwrap().is_healthy());
}

#[tokio::test]
async fn failing_site_backs_off() {
    let broken = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .expect(2)
        .mount(&broken)
        .await;

    let engine = Engine::builder()
        .indexer(indexer("broken", Protocol::Newznab, &broken))
        .cache_ttl(Duration::ZERO)
        .build()
        .unwrap();

    let q = SearchQuery::rss();
    engine.search(&q).await;
    engine.search(&q).await;
    // Third search is skipped without a request.
    let out = engine.search(&q).await;
    assert!(matches!(out.errors[0].1, IndexerError::BackingOff { .. }));
}

#[tokio::test]
async fn api_key_stays_out_of_errors() {
    // Nothing listens on port 9, so the request fails at connect time.
    let engine = Engine::builder()
        .indexer(IndexerConfig::new(
            "x",
            "x",
            Protocol::Newznab,
            "http://127.0.0.1:9",
            "SECRET",
        ))
        .build()
        .unwrap();
    let out = engine.search(&SearchQuery::rss()).await;
    let msg = out.errors[0].1.to_string();
    assert!(!msg.contains("SECRET"), "{msg}");
}
