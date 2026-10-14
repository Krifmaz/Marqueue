//! Scoutline: the built-in indexer engine (phase 4). Until then Marqueue
//! uses Prowlarr over Torznab/Newznab.
//!
//! Scoutline implements [`marqueue_adapters::Indexer`] and reads names with
//! the same parser as Marqueue, [`playbill`].

pub use marqueue_adapters::Indexer;
pub use playbill::parse;
