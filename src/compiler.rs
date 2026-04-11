use std::fs;
use std::path::{Path, PathBuf};

use crate::backend::{self, BackendOptions, BuildArtifacts};
use crate::diagnostics::Diagnostics;
use crate::ir::{self, IrProgram};
use crate::parser;
use crate::types::{self, TypedProgram};

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub namespace: String,
    pub emit_ast: bool,
    pub emit_ir: bool,
    pub clean: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            namespace: "mcfc".to_string(),
            emit_ast: false,
            emit_ir: false,
            clean: false,
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
