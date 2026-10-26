//! # scoutline-defs
//!
//! Runs Cardigann-style YAML indexer definitions, the format Prowlarr and
//! Jackett use for hundreds of sites: login, search paths and inputs, CSS
//! and JSON selectors, filters and category maps.
//!
//! Definitions are read from a folder at runtime (`<id>.yml`); none are
//! bundled. Each runs as a [`scoutline_core::backend::Backend`], so its
//! results get the same parsing, de-duplication, caching and health
//! tracking as native Newznab/Torznab feeds.

pub mod dates;
pub mod extract;
pub mod filters;
pub mod indexer;
pub mod model;
pub mod template;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use scoutline_core::settings::IndexerSettings;
use scoutline_core::{IndexerConfig, Protocol};
use scoutline_net::NetSettings;

pub use indexer::DefinitionIndexer;
pub use model::Definition;

/// Reads `<dir>/<id>.yml` (or `.yaml`).
pub fn load_definition(dir: &Path, id: &str) -> Result<Definition, String> {
    for ext in ["yml", "yaml"] {
        let path = dir.join(format!("{id}.{ext}"));
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("reading {}: {e}", path.display()))?;
            return Definition::from_yaml(&text).map_err(|e| format!("{}: {e}", path.display()));
        }
    }
    Err(format!("definition {id:?} not found in {}", dir.display()))
}

/// Builds every configured indexer: native feeds as they are, definition
/// indexers with their definition loaded from `definitions_dir`.
pub fn build_indexers(
    settings: &[IndexerSettings],
    definitions_dir: &Path,
    net: &NetSettings,
) -> Result<Vec<IndexerConfig>, String> {
    settings
        .iter()
        .map(|s| {
            let Some(def_id) = &s.definition else {
                return Ok(s.to_config());
            };
            let def = Arc::new(load_definition(definitions_dir, def_id)?);
            let mirror = (!s.url.is_empty()).then_some(s.url.as_str());
            let mut c = s.to_config();
            let backend =
                DefinitionIndexer::new(Arc::clone(&def), mirror, &s.settings, net, &c.route)
                    .map_err(|e| format!("indexer {:?}: {e}", s.id))?;
            c.protocol = Protocol::Torznab;
            c.base_url = mirror.unwrap_or(&def.links[0]).to_owned();
            if s.name.is_none() {
                c.name = def.name.clone();
            }
            if s.timeout_secs.is_none() {
                // Scraped sites are slower than APIs, and may log in first.
                c.timeout = Duration::from_secs(30);
            }
            c.backend = Some(Arc::new(backend));
            Ok(c)
        })
        .collect()
}
