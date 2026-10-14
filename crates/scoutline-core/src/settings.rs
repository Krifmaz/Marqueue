//! The `[[indexer]]` blocks shared by every app that embeds Scoutline
//! (standalone `scoutline.toml`, Marqueue's config), so they read the
//! same format.

use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

use serde::Deserialize;

use crate::indexer::{IndexerConfig, Protocol};

/// One indexer as written in a config file.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexerSettings {
    /// Letters, digits, `-` or `_`. `all` is reserved.
    pub id: String,
    pub name: Option<String>,
    pub protocol: Protocol,
    pub url: String,
    /// Defaults to `/api`.
    pub api_path: Option<String>,
    pub api_key: String,
    pub timeout_secs: Option<u64>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl IndexerSettings {
    pub fn to_config(&self) -> IndexerConfig {
        let mut c = IndexerConfig::new(
            &self.id,
            self.name.as_deref().unwrap_or(&self.id),
            self.protocol,
            &self.url,
            &self.api_key,
        );
        if let Some(p) = &self.api_path {
            c = c.api_path(p);
        }
        if let Some(t) = self.timeout_secs {
            c = c.timeout(Duration::from_secs(t));
        }
        c.enabled = self.enabled;
        c
    }
}

// Hand-written so the API key never ends up in logs.
impl fmt::Debug for IndexerSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_config().fmt(f)
    }
}

/// Checks ids are valid and unique, and URLs are http(s).
pub fn validate(indexers: &[IndexerSettings]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for i in indexers {
        if i.id.is_empty()
            || !i
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!(
                "indexer id {:?} must be letters, digits, '-' or '_'",
                i.id
            ));
        }
        if i.id == "all" {
            return Err("indexer id \"all\" is reserved for the combined feed".to_owned());
        }
        if !seen.insert(&i.id) {
            return Err(format!("indexer id {:?} is used twice", i.id));
        }
        if !(i.url.starts_with("http://") || i.url.starts_with("https://")) {
            return Err(format!(
                "indexer {:?}: url must start with http:// or https://",
                i.id
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(id: &str, url: &str) -> IndexerSettings {
        IndexerSettings {
            id: id.into(),
            name: None,
            protocol: Protocol::Torznab,
            url: url.into(),
            api_path: None,
            api_key: "secret-key".into(),
            timeout_secs: Some(5),
            enabled: true,
        }
    }

    #[test]
    fn converts_and_hides_key() {
        let s = one("t", "http://x");
        let c = s.to_config();
        assert_eq!(c.name, "t");
        assert_eq!(c.timeout, Duration::from_secs(5));
        assert!(!format!("{s:?}").contains("secret-key"));
    }

    #[test]
    fn validation() {
        assert!(validate(&[one("a", "http://x"), one("b", "https://y")]).is_ok());
        assert!(validate(&[one("all", "http://x")]).is_err());
        assert!(validate(&[one("a b", "http://x")]).is_err());
        assert!(validate(&[one("a", "http://x"), one("a", "http://y")]).is_err());
        assert!(validate(&[one("a", "ftp://x")]).is_err());
    }
}
