//! The `[[indexer]]` blocks shared by every app that embeds Scoutline
//! (standalone `scoutline.toml`, Marqueue's config), so they read the
//! same format.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::time::Duration;

use serde::Deserialize;

use crate::indexer::{IndexerConfig, Protocol};

/// One indexer as written in a config file: either a native
/// Newznab/Torznab feed (`protocol`, `url`, `api_key`) or a site run
/// through a YAML definition (`definition`, `settings`).
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexerSettings {
    /// Letters, digits, `-` or `_`. `all` is reserved.
    pub id: String,
    pub name: Option<String>,
    /// Required for native feeds; definitions are Torznab.
    pub protocol: Option<Protocol>,
    /// Feed URL, or for a definition an optional mirror of its site.
    #[serde(default)]
    pub url: String,
    /// Defaults to `/api`.
    pub api_path: Option<String>,
    #[serde(default)]
    pub api_key: String,
    pub timeout_secs: Option<u64>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Definition id: `<id>.yml` in the definitions folder.
    pub definition: Option<String>,
    /// The definition's settings, e.g. `username`, `password`, `cookie`.
    #[serde(default)]
    pub settings: BTreeMap<String, SettingValue>,
}

/// A definition setting as written in the config.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

impl SettingValue {
    pub fn as_text(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Int(i) => i.to_string(),
            Self::Text(t) => t.clone(),
        }
    }
}

fn yes() -> bool {
    true
}

// Hand-written so `enabled` defaults to true, as in config files.
impl Default for IndexerSettings {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: None,
            protocol: None,
            url: String::new(),
            api_path: None,
            api_key: String::new(),
            timeout_secs: None,
            enabled: true,
            definition: None,
            settings: BTreeMap::new(),
        }
    }
}

impl IndexerSettings {
    pub fn is_definition(&self) -> bool {
        self.definition.is_some()
    }

    /// The indexer for a native feed. Definition indexers also need their
    /// definition loaded; `scoutline-defs` builds those.
    pub fn to_config(&self) -> IndexerConfig {
        let mut c = IndexerConfig::new(
            &self.id,
            self.name.as_deref().unwrap_or(&self.id),
            self.protocol.unwrap_or(Protocol::Torznab),
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
        let has_url = !i.url.is_empty();
        if has_url && !(i.url.starts_with("http://") || i.url.starts_with("https://")) {
            return Err(format!(
                "indexer {:?}: url must start with http:// or https://",
                i.id
            ));
        }
        match &i.definition {
            Some(d) => {
                if d.is_empty()
                    || !d
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                    || d.contains("..")
                {
                    return Err(format!("indexer {:?}: bad definition name {d:?}", i.id));
                }
                if i.protocol == Some(Protocol::Newznab) {
                    return Err(format!("indexer {:?}: definitions are torznab", i.id));
                }
            }
            None => {
                if i.protocol.is_none() {
                    return Err(format!(
                        "indexer {:?}: set protocol (newznab or torznab) or a definition",
                        i.id
                    ));
                }
                if !has_url {
                    return Err(format!("indexer {:?}: url is required", i.id));
                }
                if !i.settings.is_empty() {
                    return Err(format!(
                        "indexer {:?}: settings only apply to definitions",
                        i.id
                    ));
                }
            }
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
            protocol: Some(Protocol::Torznab),
            url: url.into(),
            api_key: "secret-key".into(),
            timeout_secs: Some(5),
            ..Default::default()
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

        let def = |name: &str| IndexerSettings {
            id: "d".into(),
            definition: Some(name.into()),
            ..Default::default()
        };
        assert!(validate(&[def("1337x")]).is_ok());
        assert!(validate(&[def("../etc/passwd")]).is_err());
        let mut no_proto = one("n", "http://x");
        no_proto.protocol = None;
        assert!(validate(&[no_proto]).is_err());
    }
}
