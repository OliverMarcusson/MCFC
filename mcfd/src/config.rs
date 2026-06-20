//! `mcfd.toml` schema. Emitted by `mcfc build` (capabilities + namespace) with the
//! log/datapack paths filled in by the user for their environment.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Datapack namespace (matches the compiled pack).
    pub namespace: String,
    /// Optional override for the Minecraft instance's `logs/latest.log`. When
    /// unset, the log is auto-detected by walking up from the datapack.
    #[serde(default)]
    pub log: Option<PathBuf>,
    /// Path to the datapack root (where `data/<ns>/function/rpc/inbox` is written).
    #[serde(default = "default_datapack")]
    pub datapack: PathBuf,
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
    /// How long a computed result stays in the inbox before being assumed delivered.
    #[serde(default = "default_ttl")]
    pub result_ttl_secs: u64,
    #[serde(default)]
    pub capabilities: Capabilities,
    /// The resolved log path (override or auto-detected). Filled in by `load`.
    #[serde(skip)]
    pub log_path: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub http: Option<HttpCaps>,
    #[serde(default)]
    pub file: Option<FileCaps>,
    #[serde(default)]
    pub kv: Option<KvCaps>,
    #[serde(default)]
    pub db: Option<DbCaps>,
    #[serde(default)]
    pub time: bool,
    #[serde(default)]
    pub rand: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HttpCaps {
    #[serde(default)]
    pub allow_domains: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileCaps {
    pub root: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KvCaps {
    pub root: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DbCaps {
    pub path: PathBuf,
}

fn default_datapack() -> PathBuf {
    PathBuf::from(".")
}

fn default_poll_ms() -> u64 {
    200
}

fn default_ttl() -> u64 {
    10
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let source = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let mut config: Config = toml::from_str(&source).map_err(|error| error.to_string())?;
        // Resolve `datapack` relative to the config file's directory.
        if config.datapack.is_relative() {
            if let Some(parent) = path.parent() {
                config.datapack = parent.join(&config.datapack);
            }
        }
        config.log_path = config.resolve_log(path)?;
        Ok(config)
    }

    /// Determine the log file: an explicit `log` override (resolved relative to the
    /// config file), otherwise auto-detected by walking up from the datapack to the
    /// first `logs/latest.log`. This handles both singleplayer
    /// (`saves/<world>/datapacks/<pack>`) and server (`world/datapacks/<pack>`)
    /// layouts without any configuration.
    fn resolve_log(&self, config_path: &Path) -> Result<PathBuf, String> {
        if let Some(log) = &self.log {
            let resolved = if log.is_relative() {
                config_path.parent().unwrap_or(Path::new(".")).join(log)
            } else {
                log.clone()
            };
            return Ok(resolved);
        }

        let start = std::fs::canonicalize(&self.datapack).unwrap_or_else(|_| self.datapack.clone());
        for ancestor in start.ancestors() {
            let candidate = ancestor.join("logs").join("latest.log");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        Err(format!(
            "could not auto-detect logs/latest.log above '{}'; set `log` in the config",
            start.display()
        ))
    }
}
