use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProjectManifest {
    pub namespace: String,
    #[serde(default = "default_source_dir")]
    pub source_dir: String,
    #[serde(default = "default_asset_dir")]
    pub asset_dir: String,
    #[serde(default)]
    pub out_dir: Option<String>,
    #[serde(default)]
    pub load: Vec<String>,
    #[serde(default)]
    pub tick: Vec<String>,
    #[serde(default)]
    pub export: Vec<ProjectExport>,
    #[serde(default)]
    pub helper: Option<HelperConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProjectExport {
    pub path: String,
    pub function: String,
}

/// Configuration for the host-bridge helper that gives a vanilla datapack access
/// to external capabilities (HTTP, files, databases, real time) over the
/// `mcfc:rpc` command-storage protocol. Parsed from the `[helper]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct HelperConfig {
    #[serde(default)]
    pub backend: HelperBackend,
    #[serde(default)]
    pub capabilities: CapabilityConfig,
}

/// Which companion helper performs the external work. They share the `mcfc:rpc`
/// protocol and differ only in transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HelperBackend {
    /// Pure-Rust external binary: log-tail out, `/reload`+inbox in. Works in
    /// single player with no mod loader.
    #[default]
    Mcfd,
    /// Fabric mod reading/writing command storage in-process (follow-up).
    Mod,
    /// JVM dynamic-attach agent (follow-up).
    Agent,
}

impl HelperBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            HelperBackend::Mcfd => "mcfd",
            HelperBackend::Mod => "mod",
            HelperBackend::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "mcfd" => Some(HelperBackend::Mcfd),
            "mod" => Some(HelperBackend::Mod),
            "agent" => Some(HelperBackend::Agent),
            _ => None,
        }
    }
}

/// Per-capability configuration. A capability is "enabled" when its field is
/// present (for the struct capabilities) or `true` (for `time`/`rand`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct CapabilityConfig {
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct HttpCaps {
    /// Allowlist of permitted request domains. Empty denies all (fail closed).
    #[serde(default)]
    pub allow_domains: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FileCaps {
    /// Sandbox root; all file access is confined beneath this directory.
    pub root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KvCaps {
    /// Directory backing the key-value store.
    pub root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DbCaps {
    /// Path to the SQLite database file.
    pub path: String,
}

impl CapabilityConfig {
    /// The set of host-module names this config enables, used both for
    /// compile-time gating of `module.fn()` calls and for emitting the helper
    /// runtime/config.
    pub fn enabled_modules(&self) -> Vec<&'static str> {
        let mut modules = Vec::new();
        if self.http.is_some() {
            modules.push("http");
        }
        if self.file.is_some() {
            modules.push("file");
        }
        if self.kv.is_some() {
            modules.push("kv");
        }
        if self.db.is_some() {
            modules.push("db");
        }
        if self.time {
            modules.push("time");
        }
        if self.rand {
            modules.push("rand");
        }
        modules
    }
}

fn default_source_dir() -> String {
    "src".to_string()
}

fn default_asset_dir() -> String {
    "assets".to_string()
}

pub fn find_manifest(input: &Path) -> Result<Option<PathBuf>, String> {
    if input.is_file() {
        if is_manifest_path(input) {
            return Ok(Some(input.to_path_buf()));
        }
        return Ok(None);
    }

    if !input.is_dir() {
        return Ok(None);
    }

    let mut manifests = fs::read_dir(input)
        .map_err(|error| format!("failed to read '{}': {}", input.display(), error))?
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| is_manifest_path(path))
        .collect::<Vec<_>>();
    manifests.sort();
    Ok(manifests.into_iter().next())
}

pub fn find_manifest_in_ancestors(input: &Path) -> Result<Option<PathBuf>, String> {
    let mut current = if input.is_dir() {
        Some(input)
    } else {
        input.parent()
    };

    while let Some(path) = current {
        if let Some(manifest) = find_manifest(path)? {
            return Ok(Some(manifest));
        }
        current = path.parent();
    }

    Ok(None)
}

pub fn is_manifest_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == "mcfc.toml" || name.ends_with(".mcfc.toml"))
        .unwrap_or(false)
}

pub fn load_manifest(path: &Path) -> Result<ProjectManifest, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read '{}': {}", path.display(), error))?;
    toml::from_str(&source)
        .map_err(|error| format!("failed to parse '{}': {}", path.display(), error))
}

pub fn collect_source_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    collect_files_recursive(root, "mcf", &mut files)?;
    files.sort();
    Ok(files)
}

pub fn collect_asset_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if root.exists() {
        collect_all_files_recursive(root, &mut files)?;
        files.sort();
    }
    Ok(files)
}

fn collect_files_recursive(
    root: &Path,
    extension: &str,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)
        .map_err(|error| format!("failed to read '{}': {}", root.display(), error))?
    {
        let path = entry
            .map_err(|error| format!("failed to read '{}': {}", root.display(), error))?
            .path();
        if path.is_dir() {
            collect_files_recursive(&path, extension, files)?;
        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case(extension))
            .unwrap_or(false)
        {
            files.push(path);
        }
    }
    Ok(())
}

fn collect_all_files_recursive(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root)
        .map_err(|error| format!("failed to read '{}': {}", root.display(), error))?
    {
        let path = entry
            .map_err(|error| format!("failed to read '{}': {}", root.display(), error))?
            .path();
        if path.is_dir() {
            collect_all_files_recursive(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}
