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
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProjectExport {
    pub path: String,
    pub function: String,
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

fn collect_files_recursive(root: &Path, extension: &str, files: &mut Vec<PathBuf>) -> Result<(), String> {
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
