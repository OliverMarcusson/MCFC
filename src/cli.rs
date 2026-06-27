use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use crate::compiler::{
    CompileOptions, canonicalize_output_path, compile_file, compile_project,
    project_default_out_dir,
};
use crate::project::{collect_source_files, find_manifest};

const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(250);
const WATCH_DEBOUNCE_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Debug, Clone)]
struct ParsedCompileCommand {
    input: PathBuf,
    out_dir: Option<PathBuf>,
    options: CompileOptions,
    namespace_overridden: bool,
}

#[derive(Debug, Clone)]
struct BuildTarget {
    input: PathBuf,
    manifest_path: Option<PathBuf>,
    out_dir: PathBuf,
    options: CompileOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    len: u64,
    modified_unix_nanos: Option<u128>,
}

type WatchSnapshot = BTreeMap<PathBuf, FileFingerprint>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelperRuntime {
    None,
    Mcfd,
    McfdAgent,
}

#[derive(Debug, Clone)]
struct NewProjectConfig {
    name: String,
    namespace: String,
    path: PathBuf,
    helper: HelperRuntime,
    force: bool,
}

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
        "new" => new_command(&args[2..]),
        "build" => build_command(&args[2..]),
        "watch" => watch_command(&args[2..]),
        "--help" | "-h" | "help" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(format!("unknown command '{}'\n\n{}", other, usage())),
    }
}

fn new_command(args: &[String]) -> Result<(), String> {
    let config = parse_new_command(args)?;
    create_project(&config)?;
    println!("created MCFC project at {}", config.path.display());
    println!("next: mcfc build {} --clean", config.path.display());
    Ok(())
}

fn build_command(args: &[String]) -> Result<(), String> {
    let target = resolve_build_target(parse_compile_command(args)?)?;
    compile_target(&target)
}

fn watch_command(args: &[String]) -> Result<(), String> {
    let target = resolve_build_target(parse_compile_command(args)?)?;
    let mut snapshot = capture_watch_snapshot(&target)?;

    println!(
        "watching {} -> {}",
        watch_label(&target),
        target.out_dir.display()
    );

    if let Err(message) = compile_target(&target) {
        eprintln!("{message}");
    }

    loop {
        thread::sleep(WATCH_POLL_INTERVAL);
        let current = capture_watch_snapshot(&target)?;
        if current != snapshot {
            snapshot = debounce_watch_snapshot(&target, current)?;
            println!("change detected, recompiling...");
            if let Err(message) = compile_target(&target) {
                eprintln!("{message}");
            }
        }
    }
}

fn parse_compile_command(args: &[String]) -> Result<ParsedCompileCommand, String> {
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

    Ok(ParsedCompileCommand {
        input,
        out_dir,
        options,
        namespace_overridden,
    })
}

fn parse_new_command(args: &[String]) -> Result<NewProjectConfig, String> {
    if args.is_empty() {
        return Err(format!("missing project name\n\n{}", usage()));
    }

    let path = PathBuf::from(&args[0]);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&args[0])
        .to_string();
    let namespace = sanitize_namespace(&name)
        .ok_or_else(|| format!("project name '{}' cannot be used as a namespace", name))?;
    let mut helper = None;
    let mut force = false;
    let mut index = 1usize;

    while index < args.len() {
        match args[index].as_str() {
            "--helper" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("expected helper after '--helper'".to_string());
                };
                helper = Some(parse_helper_runtime(value)?);
            }
            "--force" => force = true,
            flag => return Err(format!("unknown flag '{}'", flag)),
        }
        index += 1;
    }

    let helper = match helper {
        Some(helper) => helper,
        None => prompt_helper_runtime()?,
    };

    Ok(NewProjectConfig {
        name,
        namespace,
        path,
        helper,
        force,
    })
}

fn parse_helper_runtime(value: &str) -> Result<HelperRuntime, String> {
    match value {
        "none" | "plain" | "mcfc" => Ok(HelperRuntime::None),
        "mcfd" => Ok(HelperRuntime::Mcfd),
        "mcfd-agent" => Ok(HelperRuntime::McfdAgent),
        _ => Err(format!(
            "unknown helper '{}'; expected 'none', 'mcfd', or 'mcfd-agent'",
            value
        )),
    }
}

fn prompt_helper_runtime() -> Result<HelperRuntime, String> {
    print!("Helper runtime: [1] none [2] mcfd [3] mcfd + mcfd-agent: ");
    io::stdout()
        .flush()
        .map_err(|error| format!("failed to write prompt: {error}"))?;

    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("failed to read helper choice: {error}"))?;

    match answer.trim() {
        "1" | "none" | "plain" | "mcfc" => Ok(HelperRuntime::None),
        "2" | "mcfd" => Ok(HelperRuntime::Mcfd),
        "3" | "mcfd-agent" | "mcfd + mcfd-agent" => Ok(HelperRuntime::McfdAgent),
        value => Err(format!(
            "unknown helper choice '{}'; expected 1, 2, 3, none, mcfd, or mcfd-agent",
            value
        )),
    }
}

fn create_project(config: &NewProjectConfig) -> Result<(), String> {
    ensure_project_target(&config.path, config.force)?;

    let src_dir = config.path.join("src");
    let assets_dir = config.path.join("assets");
    fs::create_dir_all(&src_dir)
        .map_err(|error| format!("failed to create '{}': {}", src_dir.display(), error))?;
    fs::create_dir_all(&assets_dir)
        .map_err(|error| format!("failed to create '{}': {}", assets_dir.display(), error))?;

    write_new_file(&config.path.join("mcfc.toml"), &manifest_template(config))?;
    write_new_file(&src_dir.join("main.mcf"), &main_template(config))?;
    write_new_file(&assets_dir.join(".gitkeep"), "")?;
    write_new_file(&config.path.join("README.md"), &readme_template(config))?;
    write_new_file(&config.path.join(".gitignore"), &gitignore_template(config))?;

    Ok(())
}

fn ensure_project_target(path: &Path, force: bool) -> Result<(), String> {
    if path.exists() {
        if !path.is_dir() {
            return Err(format!("target '{}' is not a directory", path.display()));
        }
        if !force {
            return Err(format!(
                "target '{}' already exists; use --force with an empty directory",
                path.display()
            ));
        }
        let mut entries = fs::read_dir(path)
            .map_err(|error| format!("failed to read '{}': {}", path.display(), error))?;
        if entries
            .next()
            .transpose()
            .map_err(|error| format!("failed to read entry in '{}': {}", path.display(), error))?
            .is_some()
        {
            return Err(format!(
                "target '{}' is not empty; choose a new project name",
                path.display()
            ));
        }
    } else {
        fs::create_dir_all(path)
            .map_err(|error| format!("failed to create '{}': {}", path.display(), error))?;
    }
    Ok(())
}

fn write_new_file(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents)
        .map_err(|error| format!("failed to write '{}': {}", path.display(), error))
}

fn manifest_template(config: &NewProjectConfig) -> String {
    let agent = match config.helper {
        HelperRuntime::None => {
            return format!(
                r#"namespace = "{namespace}"
source_dir = "src"
asset_dir = "assets"
out_dir = "dist"
"#,
                namespace = config.namespace
            );
        }
        HelperRuntime::Mcfd => "",
        HelperRuntime::McfdAgent => "\n[helper.agent]\nenabled = true\n",
    };
    format!(
        r#"namespace = "{namespace}"
source_dir = "src"
asset_dir = "assets"
out_dir = "dist"

[helper]
backend = "mcfd"
{agent}
[helper.capabilities]
time = true
rand = true
"#,
        namespace = config.namespace,
        agent = agent
    )
}

fn main_template(config: &NewProjectConfig) -> String {
    match config.helper {
        HelperRuntime::None => r#"fn main() -> void:
    let player = single(selector("@p"))
    if exists(player):
        player.tellraw("MCFC is live.")
"#
        .to_string(),
        HelperRuntime::Mcfd => r#"fn main() -> void:
    let player = single(selector("@p"))
    let now = time.now()
    let roll = rand.int(1, 6)
    if exists(player):
        if now.ok and roll.ok:
            player.tellraw("MCFC is live. unix=$(now.unix), roll=$(roll.value)")
        else:
            player.tellraw("MCFC is live, but mcfd did not answer yet.")
"#
        .to_string(),
        HelperRuntime::McfdAgent => r#"# The command has a vanilla /trigger fallback.
# With mcfd-agent attached, it is also available as a root command.
command status:
    let player = single(selector("@s"))
    player.tellraw("MCFC agent project is live.")

# This callback runs when the optional mcfd-agent is attached.
event chat(event: chat_event):
    let player = single(selector("@s"))
    if event.message == "roll":
        let roll = rand.int(1, 6)
        if roll.ok:
            player.tellraw("agent roll=$(roll.value)")

fn main() -> void:
    let player = single(selector("@p"))
    let now = time.now()
    if exists(player):
        if now.ok:
            player.tellraw("MCFC is live. Try /trigger mcfcc_status or say roll after the agent attaches.")
"#
        .to_string(),
    }
}

fn readme_template(config: &NewProjectConfig) -> String {
    let helper_note = match config.helper {
        HelperRuntime::None => "This project uses plain MCFC with no helper runtime.",
        HelperRuntime::Mcfd => "This project uses mcfd for time and random helper calls.",
        HelperRuntime::McfdAgent => {
            "This project uses mcfd and requests the optional mcfd-agent for root commands and event callbacks."
        }
    };
    let run_steps = match config.helper {
        HelperRuntime::None => {
            "1. Copy `dist/` into your world's `datapacks/` folder.\n2. Run `/reload` in Minecraft."
        }
        HelperRuntime::Mcfd | HelperRuntime::McfdAgent => {
            "1. Copy `dist/` into your world's `datapacks/` folder.\n2. Install or start the helper with `mcfd service install`.\n3. Run `/reload` in Minecraft."
        }
    };
    format!(
        r#"# {name}

{helper_note}

## Build

```powershell
mcfc build . --clean
```

## Run

{run_steps}
"#,
        name = config.name,
        helper_note = helper_note,
        run_steps = run_steps
    )
}

fn gitignore_template(config: &NewProjectConfig) -> &'static str {
    match config.helper {
        HelperRuntime::None => "dist/\n",
        HelperRuntime::Mcfd | HelperRuntime::McfdAgent => "dist/\nhost_data/\n.env\n",
    }
}

fn resolve_build_target(parsed: ParsedCompileCommand) -> Result<BuildTarget, String> {
    let manifest_path = find_manifest(&parsed.input)?;
    let inferred_out_dir = match manifest_path.as_deref() {
        Some(manifest) => project_default_out_dir(manifest)?,
        None => None,
    };
    let out_dir = parsed
        .out_dir
        .or(inferred_out_dir)
        .ok_or_else(|| "missing required '--out <directory>'".to_string())?;

    let mut options = parsed.options;
    if !parsed.namespace_overridden {
        options.namespace = manifest_path.as_deref().map_or_else(
            || infer_namespace(&parsed.input),
            |_| options.namespace.clone(),
        );
    }

    Ok(BuildTarget {
        input: parsed.input,
        manifest_path,
        out_dir: canonicalize_output_path(&out_dir),
        options,
    })
}

fn compile_target(target: &BuildTarget) -> Result<(), String> {
    if let Some(manifest_path) = &target.manifest_path {
        compile_project(manifest_path, &target.out_dir, &target.options)?;
    } else {
        compile_file(&target.input, &target.out_dir, &target.options)?;
    }
    println!("wrote datapack to {}", target.out_dir.display());
    Ok(())
}

fn watch_label(target: &BuildTarget) -> String {
    target.manifest_path.as_ref().map_or_else(
        || target.input.display().to_string(),
        |manifest_path| {
            manifest_path
                .parent()
                .map(|parent| parent.display().to_string())
                .unwrap_or_else(|| manifest_path.display().to_string())
        },
    )
}

fn capture_watch_snapshot(target: &BuildTarget) -> Result<WatchSnapshot, String> {
    let mut snapshot = BTreeMap::new();

    if let Some(manifest_path) = &target.manifest_path {
        record_file_fingerprint(manifest_path, &mut snapshot)?;
        let project_root = manifest_path.parent().ok_or_else(|| {
            format!(
                "manifest '{}' has no parent directory",
                manifest_path.display()
            )
        })?;
        for source in collect_source_files(project_root)? {
            record_file_fingerprint(&source, &mut snapshot)?;
        }
    } else {
        record_file_fingerprint(&target.input, &mut snapshot)?;
    }

    Ok(snapshot)
}

fn debounce_watch_snapshot(
    target: &BuildTarget,
    mut snapshot: WatchSnapshot,
) -> Result<WatchSnapshot, String> {
    loop {
        thread::sleep(WATCH_DEBOUNCE_INTERVAL);
        let current = capture_watch_snapshot(target)?;
        if current == snapshot {
            return Ok(current);
        }
        snapshot = current;
    }
}

fn record_file_fingerprint(path: &Path, snapshot: &mut WatchSnapshot) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed to read '{}': {}", path.display(), error))?;
    let modified_unix_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos());

    snapshot.insert(
        path.to_path_buf(),
        FileFingerprint {
            len: metadata.len(),
            modified_unix_nanos,
        },
    );
    Ok(())
}

fn usage() -> String {
    "Usage:\n  mcfc new <project-name> [--helper <none|mcfd|mcfd-agent>] [--force]\n  mcfc build <input-file|project-dir|manifest> [--out <directory>] [--namespace <name>] [--emit-ast] [--emit-ir] [--no-optimize] [--clean]\n  mcfc watch <input-file|project-dir|manifest> [--out <directory>] [--namespace <name>] [--emit-ast] [--emit-ir] [--no-optimize] [--clean]\n\nNote: '--out' may be omitted when building or watching a project manifest that defines 'out_dir'."
        .to_string()
}

fn sanitize_namespace(name: &str) -> Option<String> {
    let mut namespace = String::with_capacity(name.len());
    let mut has_name_char = false;

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            namespace.push(ch.to_ascii_lowercase());
            has_name_char = true;
        } else if ch == '_' {
            namespace.push(ch);
        } else if ch.is_ascii() {
            namespace.push('_');
        }
    }

    if has_name_char { Some(namespace) } else { None }
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

#[cfg(test)]
mod tests {
    use super::{
        capture_watch_snapshot, infer_namespace, parse_compile_command, resolve_build_target,
        sanitize_namespace,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parse_compile_command_collects_shared_flags() {
        let parsed = parse_compile_command(&[
            "demo.mcf".into(),
            "--out".into(),
            "build/out".into(),
            "--namespace".into(),
            "demo".into(),
            "--emit-ast".into(),
            "--emit-ir".into(),
            "--no-optimize".into(),
            "--clean".into(),
        ])
        .expect("arguments should parse");

        assert_eq!(parsed.input, PathBuf::from("demo.mcf"));
        assert_eq!(parsed.out_dir, Some(PathBuf::from("build/out")));
        assert!(parsed.namespace_overridden);
        assert_eq!(parsed.options.namespace, "demo");
        assert!(parsed.options.emit_ast);
        assert!(parsed.options.emit_ir);
        assert!(!parsed.options.optimize);
        assert!(parsed.options.clean);
    }

    #[test]
    fn resolve_build_target_uses_manifest_default_output() {
        let base = temp_path();
        let project = base.join("project");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("demo.mcfc.toml"),
            r#"
namespace = "demo"
out_dir = "dist"
"#,
        )
        .unwrap();

        let target = resolve_build_target(
            parse_compile_command(&[project.display().to_string()])
                .expect("arguments should parse"),
        )
        .expect("target should resolve");

        assert_eq!(target.out_dir, project.join("dist"));
        assert_eq!(target.manifest_path, Some(project.join("demo.mcfc.toml")));
    }

    #[test]
    fn capture_watch_snapshot_tracks_manifest_and_source_files() {
        let base = temp_path();
        let project = base.join("project");
        let src = project.join("src");
        let nested = src.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            project.join("demo.mcfc.toml"),
            r#"
namespace = "demo"
source_dir = "src"
out_dir = "dist"
"#,
        )
        .unwrap();
        fs::write(src.join("main.mcf"), "fn main() -> void:\n    return\n").unwrap();
        fs::write(
            nested.join("helper.mcf"),
            "fn helper() -> void:\n    return\n",
        )
        .unwrap();
        fs::write(project.join("assets.json"), "{}\n").unwrap();

        let target = resolve_build_target(
            parse_compile_command(&[project.display().to_string()])
                .expect("arguments should parse"),
        )
        .expect("target should resolve");
        let snapshot = capture_watch_snapshot(&target).expect("snapshot should succeed");

        assert!(snapshot.contains_key(&project.join("demo.mcfc.toml")));
        assert!(snapshot.contains_key(&src.join("main.mcf")));
        assert!(snapshot.contains_key(&nested.join("helper.mcf")));
        assert!(!snapshot.contains_key(&project.join("assets.json")));
    }

    #[test]
    fn infer_namespace_normalizes_filename() {
        assert_eq!(
            infer_namespace(PathBuf::from("Test Pack!.mcf").as_path()),
            "test_pack_"
        );
    }

    #[test]
    fn sanitize_namespace_normalizes_project_name() {
        assert_eq!(sanitize_namespace("My Pack!"), Some("my_pack_".to_string()));
        assert_eq!(sanitize_namespace("Æøå"), None);
    }

    fn temp_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mcfc_cli_test_{unique}"))
    }
}
