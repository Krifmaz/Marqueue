use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream::{FuturesUnordered, Stream, StreamExt};
use scoutline_net::{Client, NetError, NetSettings, build_client};
use tracing::debug;
use url::Url;

use crate::cache::QueryCache;
use crate::dedupe::dedupe;
use crate::error::IndexerError;
use crate::feed::parse_feed;
use crate::health::HealthTracker;
use crate::indexer::{IndexerConfig, IndexerId, Protocol};
use crate::query::SearchQuery;
use crate::result::SearchResult;

/// One indexer's answer, sent as soon as it arrives.
#[derive(Debug, Clone)]
pub enum IndexerEvent {
    Results {
        indexer: IndexerId,
        results: Vec<SearchResult>,
    },
    Failed {
        indexer: IndexerId,
        error: IndexerError,
    },
}

/// Everything a search found, merged and de-duplicated.
#[derive(Debug, Clone, Default)]
pub struct SearchOutcome {
    pub results: Vec<SearchResult>,
    /// Indexers that didn't answer, and why.
    pub errors: Vec<(IndexerId, IndexerError)>,
}

/// Which indexers a search goes to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    All,
    Protocol(Protocol),
    /// These indexers only.
    Indexers(Vec<IndexerId>),
}

impl Scope {
    fn includes(&self, config: &IndexerConfig) -> bool {
        match self {
            Self::All => true,
            Self::Protocol(p) => config.protocol == *p,
            Self::Indexers(ids) => ids.contains(&config.id),
        }
    }
}

struct Indexer {
    config: IndexerConfig,
    client: Client,
}

struct Inner {
    indexers: Vec<Indexer>,
    cache: QueryCache<(Scope, SearchQuery), Arc<SearchOutcome>>,
    health: HealthTracker,
}

/// Searches every enabled indexer at once. Cheap to clone.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

pub struct EngineBuilder {
    indexers: Vec<IndexerConfig>,
    net: NetSettings,
    cache_ttl: Duration,
    cache_entries: usize,
}

impl Engine {
    pub fn builder() -> EngineBuilder {
        EngineBuilder {
            indexers: Vec::new(),
            net: NetSettings::default(),
            cache_ttl: Duration::from_secs(120),
            cache_entries: 256,
        }
    }

    pub fn indexers(&self) -> impl Iterator<Item = &IndexerConfig> {
        self.inner.indexers.iter().map(|i| &i.config)
    }

    pub fn health(&self) -> &HealthTracker {
        &self.inner.health
    }

    /// Queries every enabled indexer at once and yields each one's answer
    /// as it arrives. Indexers in backoff fail straight away. Bypasses the
    /// cache.
    pub fn search_stream(
        &self,
        query: &SearchQuery,
    ) -> impl Stream<Item = IndexerEvent> + Send + 'static {
        self.search_stream_in(&Scope::All, query)
    }

    /// [`search_stream`](Self::search_stream) limited to `scope`.
    pub fn search_stream_in(
        &self,
        scope: &Scope,
        query: &SearchQuery,
    ) -> impl Stream<Item = IndexerEvent> + Send + 'static {
        let query = Arc::new(query.clone());
        (0..self.inner.indexers.len())
            .filter(|&i| {
                let c = &self.inner.indexers[i].config;
                c.enabled && scope.includes(c)
            })
            .map(|i| {
                let inner = Arc::clone(&self.inner);
                let query = Arc::clone(&query);
                async move { inner.query_one(i, &query).await }
            })
            .collect::<FuturesUnordered<_>>()
    }

    /// Runs the search on every enabled indexer and returns the merged,
    /// de-duplicated results. Repeats within the cache TTL are answered
    /// from the cache.
    pub async fn search(&self, query: &SearchQuery) -> Arc<SearchOutcome> {
        self.search_in(&Scope::All, query).await
    }

    /// [`search`](Self::search) limited to `scope`.
    pub async fn search_in(&self, scope: &Scope, query: &SearchQuery) -> Arc<SearchOutcome> {
        let key = (scope.clone(), query.clone());
        if let Some(hit) = self.inner.cache.get(&key) {
            return hit;
        }

        let mut results = Vec::new();
        let mut errors = Vec::new();
        let mut answered = false;
        let mut events = self.search_stream_in(scope, query);
        while let Some(event) = events.next().await {
            match event {
                IndexerEvent::Results { results: r, .. } => {
                    answered = true;
                    results.extend(r);
                }
                IndexerEvent::Failed { indexer, error } => errors.push((indexer, error)),
            }
        }

        let outcome = Arc::new(SearchOutcome {
            results: dedupe(results),
            errors,
        });
        // Don't cache a search where every site failed: retry next time.
        if answered {
            self.inner.cache.insert(key, Arc::clone(&outcome));
        }
        outcome
    }
}

impl Inner {
    async fn query_one(&self, i: usize, query: &SearchQuery) -> IndexerEvent {
        let Indexer { config, client } = &self.indexers[i];
        let indexer = config.id.clone();

        if let Err(error) = self.health.check(&config.id) {
            return IndexerEvent::Failed { indexer, error };
        }

        let started = Instant::now();
        let result = match tokio::time::timeout(config.timeout, fetch(client, config, query)).await
        {
            Ok(r) => r,
            Err(_) => Err(IndexerError::Timeout(config.timeout)),
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;

        match result {
            Ok(results) => {
                debug!(indexer = %config.id, count = results.len(), elapsed_ms, "indexer answered");
                self.health.record_success(&config.id, elapsed_ms);
                IndexerEvent::Results { indexer, results }
            }
            Err(error) => {
                debug!(indexer = %config.id, %error, "indexer failed");
                self.health.record_failure(&config.id, &error);
                IndexerEvent::Failed { indexer, error }
            }
        }
    }
}

async fn fetch(
    client: &Client,
    config: &IndexerConfig,
    query: &SearchQuery,
) -> Result<Vec<SearchResult>, IndexerError> {
    // `without_url` keeps the API key out of error messages.
    let http = |e: reqwest::Error| IndexerError::Http(e.without_url().to_string());

    let mut url = Url::parse(&config.endpoint())
        .map_err(|e| IndexerError::Http(format!("bad indexer URL: {e}")))?;
    url.query_pairs_mut()
        .extend_pairs(query.params(config.protocol))
        .append_pair("apikey", &config.api_key);

    let response = client.get(url).send().await.map_err(http)?;

    let status = response.status();
    let body = response.text().await.map_err(http)?;
    // Newznab reports bad keys and limits as <error> bodies, sometimes
    // with a non-200 status; prefer that message when there is one.
    let items = match parse_feed(&body) {
        Ok(_) if !status.is_success() => return Err(IndexerError::Status(status.as_u16())),
        Ok(items) => items,
        Err(e @ IndexerError::Api { .. }) => return Err(e),
        Err(_) if !status.is_success() => return Err(IndexerError::Status(status.as_u16())),
        Err(e) => return Err(e),
    };
    Ok(items
        .into_iter()
        .map(|item| SearchResult::from_item(item, config, query.search_type))
        .collect())
}

impl EngineBuilder {
    pub fn indexer(mut self, config: IndexerConfig) -> Self {
        self.indexers.push(config);
        self
    }

    pub fn indexers(mut self, configs: impl IntoIterator<Item = IndexerConfig>) -> Self {
        self.indexers.extend(configs);
        self
    }

    pub fn net(mut self, settings: NetSettings) -> Self {
        self.net = settings;
        self
    }

    /// How long a search answer is reused. Zero turns the cache off.
    pub fn cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    pub fn build(self) -> Result<Engine, NetError> {
        let indexers = self
            .indexers
            .into_iter()
            .map(|config| {
                let client = build_client(&self.net, &config.route)?;
                Ok(Indexer { config, client })
            })
            .collect::<Result<_, NetError>>()?;
        Ok(Engine {
            inner: Arc::new(Inner {
                indexers,
                cache: QueryCache::new(self.cache_ttl, self.cache_entries),
                health: HealthTracker::new(),
            }),
        })
    }
}
