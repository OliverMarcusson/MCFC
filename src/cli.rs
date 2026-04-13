use std::path::PathBuf;

use crate::compiler::{
    CompileOptions, canonicalize_output_path, compile_file, compile_project,
    project_default_out_dir,
};
use crate::project::find_manifest;

pub fn run(args: Vec<String>) -> i32 {
    match try_run(args) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}

fn try_run(args: Vec<String>) -> Result<(), String> {
    if args.len() < 2 {
        return Err(usage());
    }
    match args[1].as_str() {
        "build" => build_command(&args[2..]),
        "--help" | "-h" | "help" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(format!("unknown command '{}'\n\n{}", other, usage())),
    }
}

fn build_command(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err(format!("missing input path\n\n{}", usage()));
    }

    let input = PathBuf::from(&args[0]);
    let mut out_dir = None;
    let mut options = CompileOptions::default();
    let mut namespace_overridden = false;
    let mut index = 1usize;

    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("expected path after '--out'".to_string());
                };
                out_dir = Some(PathBuf::from(value));
            }
            "--namespace" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("expected namespace after '--namespace'".to_string());
                };
                options.namespace = value.clone();
                namespace_overridden = true;
            }
            "--emit-ast" => options.emit_ast = true,
            "--emit-ir" => options.emit_ir = true,
            "--no-optimize" => options.optimize = false,
            "--clean" => options.clean = true,
            flag => return Err(format!("unknown flag '{}'", flag)),
        }
        index += 1;
    }

    let manifest_path = find_manifest(&input)?;
    let inferred_out_dir = match manifest_path.as_deref() {
        Some(manifest) => project_default_out_dir(manifest)?,
        None => None,
    };
    let out_dir = out_dir
        .or(inferred_out_dir)
        .ok_or_else(|| "missing required '--out <directory>'".to_string())?;
    if !namespace_overridden {
        options.namespace = manifest_path
            .as_deref()
            .map_or_else(|| infer_namespace(&input), |_| options.namespace.clone());
    }
    let out_dir = canonicalize_output_path(&out_dir);
    if let Some(manifest_path) = manifest_path {
        compile_project(&manifest_path, &out_dir, &options)?;
    } else {
        compile_file(&input, &out_dir, &options)?;
    }
    println!("wrote datapack to {}", out_dir.display());
    Ok(())
}

fn usage() -> String {
    "Usage:\n  mcfc build <input-file|project-dir|manifest> --out <directory> [--namespace <name>] [--emit-ast] [--emit-ir] [--no-optimize] [--clean]".to_string()
}

fn infer_namespace(input: &std::path::Path) -> String {
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("mcfc");
    let mut namespace = String::with_capacity(stem.len());
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            namespace.push(ch.to_ascii_lowercase());
        } else {
            namespace.push('_');
        }
    }
    if namespace.is_empty() {
        "mcfc".to_string()
    } else {
        namespace
    }
}
