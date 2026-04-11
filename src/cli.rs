use std::path::PathBuf;

use crate::compiler::{canonicalize_output_path, compile_file, CompileOptions};

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
        return Err(format!("missing input file\n\n{}", usage()));
    }

    let input = PathBuf::from(&args[0]);
    let mut out_dir = None;
    let mut options = CompileOptions::default();
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
            }
            "--emit-ast" => options.emit_ast = true,
            "--emit-ir" => options.emit_ir = true,
            "--clean" => options.clean = true,
            flag => return Err(format!("unknown flag '{}'", flag)),
        }
        index += 1;
    }

    let out_dir = out_dir.ok_or_else(|| "missing required '--out <directory>'".to_string())?;
    let out_dir = canonicalize_output_path(&out_dir);
    compile_file(&input, &out_dir, &options)?;
    println!("wrote datapack to {}", out_dir.display());
    Ok(())
}

fn usage() -> String {
    "Usage:\n  mcfc build <input-file> --out <directory> [--namespace <name>] [--emit-ast] [--emit-ir] [--clean]".to_string()
}
