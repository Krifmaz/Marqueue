//! # scoutline-core
//!
//! The search side of Scoutline: native Newznab/Torznab indexers, async
//! fan-out search with per-site timeouts, a short-lived query cache,
//! health tracking with backoff, and results parsed with
//! [`playbill`] and de-duplicated across indexers.
//!
//! ```no_run
//! use scoutline_core::{Engine, IndexerConfig, Protocol, SearchQuery};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let engine = Engine::builder()
//!     .indexer(IndexerConfig::new("geek", "NZBgeek", Protocol::Newznab, "https://api.nzbgeek.info", "KEY"))
//!     .build()?;
//! let results = engine.search(&SearchQuery::tv("The Bear").season(3).episode(4)).await;
//! for r in &results.results {
//!     println!("{} ({:?})", r.title, r.release.resolution);
//! }
//! # Ok(()) }
//! ```

pub mod cache;
pub mod category;
mod dedupe;
mod engine;
mod error;
pub mod feed;
pub mod health;
mod indexer;
mod query;
mod result;
pub mod settings;

pub use dedupe::dedupe;
pub use engine::{Engine, EngineBuilder, IndexerEvent, Scope, SearchOutcome};
pub use error::IndexerError;
pub use indexer::{IndexerConfig, IndexerId, Protocol};
pub use query::{SearchQuery, SearchType};
pub use result::{Origin, SearchResult};

/// Re-exported so callers can read [`SearchResult::release`] without
/// depending on playbill directly.
pub use playbill;
