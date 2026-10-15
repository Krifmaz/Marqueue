//! `scoutline.toml`: the standalone server's settings and indexers.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use scoutline_core::Engine;
use scoutline_core::settings::{self, IndexerSettings};
use scoutline_net::{DEFAULT_USER_AGENT, NetSettings};
use serde::Deserialize;

pub const DEFAULT_PORT: u16 = 9797;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    /// Key Sonarr, Radarr and the web UI use to reach Scoutline.
    pub api_key: String,
    pub user_agent: Option<String>,
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_secs: u64,
    /// Where `<id>.yml` indexer definitions are read from.
    #[serde(default = "default_definitions_dir")]
    pub definitions_dir: PathBuf,
    #[serde(default, rename = "indexer")]
    pub indexers: Vec<IndexerSettings>,
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT))
}

fn default_cache_ttl() -> u64 {
    120
}

fn default_definitions_dir() -> PathBuf {
    PathBuf::from("/config/definitions")
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.api_key.trim().is_empty() {
            bail!("api_key must not be empty");
        }
        settings::validate(&self.indexers).map_err(anyhow::Error::msg)?;
        Ok(())
    }

    pub fn build_engine(&self) -> anyhow::Result<Engine> {
        let net = NetSettings {
            user_agent: self
                .user_agent
                .clone()
                .unwrap_or_else(|| DEFAULT_USER_AGENT.to_owned()),
            ..NetSettings::default()
        };
        let indexers = scoutline_defs::build_indexers(&self.indexers, &self.definitions_dir, &net)
            .map_err(anyhow::Error::msg)?;
        Ok(Engine::builder()
            .net(net)
            .cache_ttl(Duration::from_secs(self.cache_ttl_secs))
            .indexers(indexers)
            .build()?)
    }
}

/// The config written on first start, with a fresh API key.
pub fn starter_config(api_key: &str) -> String {
    format!(
        r#"# Scoutline settings. Restart the container after editing.

listen = "0.0.0.0:{DEFAULT_PORT}"

# Sonarr, Radarr and the web UI use this key to reach Scoutline.
api_key = "{api_key}"

# How long a search answer is reused, in seconds. 0 turns the cache off.
cache_ttl_secs = 120

# Add one [[indexer]] block per Newznab or Torznab indexer.
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
#
# Sites without an API run through a YAML definition (the Cardigann
# format Prowlarr and Jackett use). Put <id>.yml in definitions_dir:
#
# definitions_dir = "/config/definitions"
#
# [[indexer]]
# id = "mytracker"
# definition = "mytracker"            # reads mytracker.yml
# [indexer.settings]                  # the definition's settings
# username = "me"
# password = "secret"
#
# Any Torznab feed works, e.g. one from Jackett or Prowlarr:
#
# [[indexer]]
# id = "tracker"
# protocol = "torznab"
# url = "http://prowlarr:9696/1"
# api_key = "PROWLARR-API-KEY"
"#
    )
}

/// 32 random hex characters.
pub fn random_key() -> anyhow::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scoutline_core::Protocol;

    #[test]
    fn starter_config_parses() {
        let c = Config::parse(&starter_config("abc")).unwrap();
        assert_eq!(c.api_key, "abc");
        assert_eq!(c.listen.port(), DEFAULT_PORT);
        assert!(c.indexers.is_empty());
    }

    #[test]
    fn reads_indexers() {
        let c = Config::parse(
            r#"
            api_key = "k"
            [[indexer]]
            id = "geek"
            protocol = "newznab"
            url = "https://api.nzbgeek.info"
            api_key = "x"
            timeout_secs = 5
            "#,
        )
        .unwrap();
        assert_eq!(c.indexers[0].protocol, Some(Protocol::Newznab));
        let engine = c.build_engine().unwrap();
        let idx = engine.indexers().next().unwrap();
        assert_eq!(idx.name, "geek");
        assert_eq!(idx.timeout, Duration::from_secs(5));
    }

    #[test]
    fn rejects_bad_config() {
        let base = r#"api_key = "k"
            [[indexer]]
            protocol = "torznab"
            url = "http://x"
            api_key = "x"
            "#;
        assert!(Config::parse(&format!("{base}id = \"all\"")).is_err());
        assert!(Config::parse(&format!("{base}id = \"a b\"")).is_err());
        assert!(Config::parse(r#"api_key = """#).is_err());
        assert!(Config::parse("api_key = \"k\"\ntypo = 1").is_err());
    }

    #[test]
    fn random_keys_differ() {
        let a = random_key().unwrap();
        assert_eq!(a.len(), 32);
        assert_ne!(a, random_key().unwrap());
    }
}
