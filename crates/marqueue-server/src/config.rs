//! `/config/marqueue.toml`. For now it holds the indexers Scoutline
//! searches; the rest of Marqueue's settings will live in the UI.

use std::path::Path;

use scoutline::IndexerSettings;
use serde::Deserialize;

pub const DEFAULT_PATH: &str = "/config/marqueue.toml";

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// Newznab/Torznab indexers, same `[[indexer]]` blocks as Scoutline's
    /// standalone `scoutline.toml`.
    #[serde(default, rename = "indexer")]
    pub indexers: Vec<IndexerSettings>,
}

const STARTER: &str = r#"# Marqueue settings. Restart Marqueue after editing.

# Indexers Marqueue searches through its built-in Scoutline engine.
# Add one [[indexer]] block per Newznab or Torznab indexer; the same blocks
# work in a standalone scoutline.toml.
#
# [[indexer]]
# id = "nzbgeek"                      # letters, digits, - or _
# name = "NZBgeek"
# protocol = "newznab"                # or "torznab"
# url = "https://api.nzbgeek.info"
# api_path = "/api"                   # optional, default /api
# api_key = "YOUR-INDEXER-KEY"
# timeout_secs = 15                   # optional
# enabled = true                      # optional
"#;

impl Config {
    /// Reads the config, writing a commented starter file first if there
    /// is none.
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("creating {}: {e}", dir.display()))?;
            }
            std::fs::write(path, STARTER)
                .map_err(|e| format!("writing {}: {e}", path.display()))?;
            tracing::info!(path = %path.display(), "wrote a starter config");
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("in {}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let c: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        scoutline::validate(&c.indexers)?;
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_has_no_indexers() {
        assert!(Config::parse(STARTER).unwrap().indexers.is_empty());
    }

    #[test]
    fn reads_scoutline_blocks() {
        let c = Config::parse(
            r#"
            [[indexer]]
            id = "geek"
            protocol = "newznab"
            url = "https://api.nzbgeek.info"
            api_key = "x"
            "#,
        )
        .unwrap();
        assert_eq!(c.indexers[0].id, "geek");
        assert!(Config::parse("[[indexer]]\nid = \"all\"\nprotocol = \"torznab\"\nurl = \"http://x\"\napi_key = \"k\"").is_err());
    }
}
