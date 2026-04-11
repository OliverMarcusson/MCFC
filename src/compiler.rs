use std::fs;
use std::path::{Path, PathBuf};

use crate::backend::{self, BackendOptions, BuildArtifacts, ExportedFunction};
use crate::diagnostics::Diagnostics;
use crate::ir::{self, IrProgram};
use crate::parser;
use crate::project::{collect_asset_files, collect_source_files, load_manifest};
use crate::types::{self, TypedProgram};

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub namespace: String,
    pub emit_ast: bool,
    pub emit_ir: bool,
    pub clean: bool,
    pub load_tag_values: Option<Vec<String>>,
    pub tick_tag_values: Option<Vec<String>>,
    pub exports: Vec<ExportedFunction>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            namespace: "mcfc".to_string(),
            emit_ast: false,
            emit_ir: false,
            clean: false,
            load_tag_values: None,
            tick_tag_values: None,
            exports: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompileResult {
    pub typed_program: TypedProgram,
    pub ir_program: IrProgram,
    pub artifacts: BuildArtifacts,
}

pub fn compile_source(
    source: &str,
    options: &CompileOptions,
) -> Result<CompileResult, Diagnostics> {
    let ast = parser::parse(source)?;
    let typed_program = types::type_check(&ast)?;
    let ir_program = ir::lower(&typed_program);
    let artifacts = backend::generate(
        &ir_program,
        &BackendOptions {
            namespace: options.namespace.clone(),
            load_tag_values: options.load_tag_values.clone(),
            tick_tag_values: options.tick_tag_values.clone(),
            exports: options.exports.clone(),
        },
    );
    Ok(CompileResult {
        typed_program,
        ir_program,
        artifacts,
    })
}

pub fn compile_file(
    input: &Path,
    out_dir: &Path,
    options: &CompileOptions,
) -> Result<CompileResult, String> {
    let source = fs::read_to_string(input)
        .map_err(|error| format!("failed to read '{}': {}", input.display(), error))?;
    let compiled = compile_source(&source, options)
        .map_err(|diagnostics| render_diagnostics(&diagnostics, &source))?;
    write_output(out_dir, &compiled, options)?;
    Ok(compiled)
}

pub fn compile_project(
    manifest_path: &Path,
    out_dir: &Path,
    options: &CompileOptions,
) -> Result<CompileResult, String> {
    let manifest = load_manifest(manifest_path)?;
    let project_root = manifest_path
        .parent()
        .ok_or_else(|| format!("manifest '{}' has no parent directory", manifest_path.display()))?;
    let source_root = project_root.join(&manifest.source_dir);
    let asset_root = project_root.join(&manifest.asset_dir);
    let sources = collect_source_files(&source_root)?;
    if sources.is_empty() {
        return Err(format!(
            "no '.mcf' files found under '{}'",
            source_root.display()
        ));
    }

    let merged_source = merge_project_sources(&sources)?;
    let mut effective = options.clone();
    effective.namespace = manifest.namespace.clone();
    if !manifest.load.is_empty() {
        effective.load_tag_values = Some(manifest.load.clone());
    }
    if !manifest.tick.is_empty() {
        effective.tick_tag_values = Some(manifest.tick.clone());
    }
    effective.exports = manifest
        .export
        .iter()
        .map(|item| ExportedFunction {
            path: item.path.clone(),
            function: item.function.clone(),
        })
        .collect();

    let mut compiled = compile_source(&merged_source, &effective)
        .map_err(|diagnostics| render_diagnostics(&diagnostics, &merged_source))?;
    copy_project_assets(&asset_root, &mut compiled.artifacts)?;
    write_output(out_dir, &compiled, &effective)?;
    Ok(compiled)
}

pub fn project_default_out_dir(manifest_path: &Path) -> Result<Option<PathBuf>, String> {
    let manifest = load_manifest(manifest_path)?;
    Ok(manifest
        .out_dir
        .map(|relative| manifest_path.parent().unwrap().join(relative)))
}

fn write_output(
    out_dir: &Path,
    compiled: &CompileResult,
    options: &CompileOptions,
) -> Result<(), String> {
    if options.clean && out_dir.exists() {
        fs::remove_dir_all(out_dir).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(out_dir).map_err(|error| error.to_string())?;
    for (relative, contents) in &compiled.artifacts.files {
        let destination = out_dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(destination, contents).map_err(|error| error.to_string())?;
    }
    if options.emit_ast {
        write_debug_file(
            &out_dir.join("debug").join("typed_program.txt"),
            format!("{:#?}\n", compiled.typed_program),
        )?;
    }
    if options.emit_ir {
        write_debug_file(
            &out_dir.join("debug").join("ir.txt"),
            format!("{:#?}\n", compiled.ir_program),
        )?;
    }
    Ok(())
}

fn write_debug_file(path: &Path, contents: String) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, contents).map_err(|error| error.to_string())
}

fn render_diagnostics(diagnostics: &Diagnostics, source: &str) -> String {
    diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.render(source))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn canonicalize_output_path(out_dir: &Path) -> PathBuf {
    out_dir.to_path_buf()
}

fn merge_project_sources(files: &[PathBuf]) -> Result<String, String> {
    let mut merged = String::new();
    for file in files {
        let source = fs::read_to_string(file)
            .map_err(|error| format!("failed to read '{}': {}", file.display(), error))?;
        merged.push_str(&format!("# source: {}\n", file.display()));
        merged.push_str(&source);
        if !source.ends_with('\n') {
            merged.push('\n');
        }
        merged.push('\n');
    }
    Ok(merged)
}

fn copy_project_assets(asset_root: &Path, artifacts: &mut BuildArtifacts) -> Result<(), String> {
    for file in collect_asset_files(asset_root)? {
        let relative = file
            .strip_prefix(asset_root)
            .map_err(|error| error.to_string())?;
        let normalized = relative
            .to_string_lossy()
            .replace('\\', "/");
        let contents = fs::read_to_string(&file)
            .map_err(|error| format!("failed to read '{}': {}", file.display(), error))?;
        artifacts.files.insert(normalized, contents);
    }
    Ok(())
}
