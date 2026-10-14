//! # scoutline-api
//!
//! The standalone Scoutline server: a Torznab/Newznab API per indexer and
//! for all indexers, a JSON API, and a small web UI for manual search.

pub mod config;
pub mod torznab;
mod web;

pub use web::{AppState, router};
