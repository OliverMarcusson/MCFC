//! Global entity-death-backed host bridge for MCFC datapacks.
//!
//! Compiled packs create an off-map, named pig and immediately kill it. The death
//! message carries a command-storage RPC record into Minecraft's `latest.log`.
//! This process discovers generated `mcfd.pack.toml` descriptors, tails their
//! launcher-specific logs, and writes replies into each pack inbox.

mod config;
mod handlers;
mod snbt;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use config::Config;

const DESCRIPTOR: &str = "mcfd.pack.toml";
const POLL: Duration = Duration::from_millis(200);
/// How often to re-scan known Minecraft locations for pack descriptors. Cheap,
/// because we only look inside launcher instance directories (see
/// [`minecraft_roots`]) rather than walking whole drives.
const RESCAN: Duration = Duration::from_secs(5);
/// Bounded search depth when locating Minecraft instance roots under a launcher
/// directory (e.g. `PrismLauncher/instances/<name>/minecraft`).
const ROOT_SEARCH_DEPTH: usize = 4;

struct PendingResult {
    snbt: String,
    expires: Instant,
}

#[derive(Clone)]
struct Pack {
    config: Config,
    descriptor: PathBuf,
    log: PathBuf,
}

/// A parsed entity-death request record.
pub struct Request {
    pub pack_id: String,
    pub id: i64,
    pub module: String,
    pub function: String,
    pub args: Vec<snbt::Value>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [service, command] if service == "service" && command == "run" => run_service(),
        [service, command] if service == "service" && command == "install" => install_task(),
        [service, command] if service == "service" && command == "uninstall" => uninstall_task(),
        [service, command] if service == "service" && command == "status" => status(),
        _ => Err("usage: mcfd service <run|install|uninstall|status>".to_string()),
    };
    if let Err(error) = result {
        eprintln!("mcfd: {error}");
        std::process::exit(1);
    }
}

fn run_service() -> Result<(), String> {
    let state = state_dir()?;
    std::fs::write(state.join("status.txt"), "running\n").map_err(|e| e.to_string())?;
    eprintln!("mcfd: global service running; discovering {DESCRIPTOR} in Minecraft instances");

    // Discovery runs on a light timer rather than a recursive whole-drive watch:
    // re-scanning only known launcher instance directories is cheap, so polling
    // every few seconds keeps CPU near zero and never floods the main loop.
    let (scan_tx, scan_rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        if scan_tx.send(discover_packs()).is_err() {
            break;
        }
        std::thread::sleep(RESCAN);
    });
    let mut packs = Vec::new();
    let mut offsets: HashMap<PathBuf, u64> = HashMap::new();
    let mut results: HashMap<PathBuf, HashMap<i64, PendingResult>> = HashMap::new();
    loop {
        while let Ok(updated) = scan_rx.try_recv() {
            packs = updated;
            write_status(&state, &packs);
        }

        let mut by_log: HashMap<&Path, Vec<&Pack>> = HashMap::new();
        for pack in &packs {
            by_log.entry(pack.log.as_path()).or_default().push(pack);
        }
        for (log, candidates) in by_log {
            let offset = offsets.entry(log.to_path_buf()).or_insert(0);
            for line in read_new_lines(log, offset) {
                let Some(request) = parse_marker(&line) else {
                    continue;
                };
                let Some(pack) = candidates
                    .iter()
                    .find(|pack| pack.config.pack_id == request.pack_id)
                else {
                    continue;
                };
                let pending = results.entry(pack.descriptor.clone()).or_default();
                if pending.contains_key(&request.id) {
                    continue;
                }
                eprintln!(
                    "mcfd: {} request #{} {}.{}",
                    pack.config.namespace, request.id, request.module, request.function
                );
                pending.insert(
                    request.id,
                    PendingResult {
                        snbt: handlers::dispatch(&pack.config, &request),
                        expires: Instant::now() + Duration::from_secs(pack.config.result_ttl_secs),
                    },
                );
                if let Err(error) = write_inbox(&pack.config, pending) {
                    eprintln!("mcfd: failed to write {}: {error}", pack.config.namespace);
                }
            }
        }
        let now = Instant::now();
        for pack in &packs {
            if let Some(pending) = results.get_mut(&pack.descriptor) {
                let before = pending.len();
                pending.retain(|_, result| result.expires > now);
                if pending.len() != before {
                    let _ = write_inbox(&pack.config, pending);
                }
            }
        }
        std::thread::sleep(POLL);
    }
}

fn discover_packs() -> Vec<Pack> {
    let mut found = Vec::new();
    for path in find_descriptors() {
        let Ok(config) = Config::load(&path) else {
            continue;
        };
        if config.protocol != 2 {
            continue;
        }
        let Some(log) = config.resolve_log(&path) else {
            continue;
        };
        found.push(Pack {
            config,
            descriptor: path,
            log,
        });
    }
    found
}

/// Locate pack descriptors inside discovered Minecraft instances only. Datapacks
/// live at `<instance>/saves/<world>/datapacks/<pack>/` (singleplayer) or
/// `<instance>/<world>/datapacks/<pack>/` (server), so we look exactly there
/// instead of walking entire drives.
fn find_descriptors() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in minecraft_roots() {
        // Singleplayer worlds under `saves/`, and server worlds at the root.
        collect_world_descriptors(&root.join("saves"), &mut found);
        collect_world_descriptors(&root, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

/// For a directory holding worlds, scan each `<world>/datapacks/<pack>/` (and a
/// `datapacks/` directly under `base`) for the descriptor file.
fn collect_world_descriptors(base: &Path, out: &mut Vec<PathBuf>) {
    collect_pack_descriptors(&base.join("datapacks"), out);
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            collect_pack_descriptors(&entry.path().join("datapacks"), out);
        }
    }
}

/// Collect `<datapacks>/<pack>/mcfd.pack.toml` for each pack directory.
fn collect_pack_descriptors(datapacks: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(datapacks) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let descriptor = entry.path().join(DESCRIPTOR);
            if descriptor.is_file() {
                out.push(descriptor);
            }
        }
    }
}

/// Candidate Minecraft instance directories — the folders that contain `logs/`
/// and `saves/`. We probe well-known launcher locations instead of scanning
/// whole drives. Add custom locations with the `MCFD_MINECRAFT_DIRS`
/// environment variable (a `;`-separated list of instance directories).
fn minecraft_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Some(extra) = std::env::var_os("MCFD_MINECRAFT_DIRS") {
        for part in extra.to_string_lossy().split(';') {
            let part = part.trim();
            if !part.is_empty() {
                roots.push(PathBuf::from(part));
            }
        }
    }

    // Launcher install roots to search within (bounded depth).
    let mut bases: Vec<PathBuf> = Vec::new();
    for var in ["APPDATA", "USERPROFILE", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(var).map(PathBuf::from) else {
            continue;
        };
        bases.push(base.join(".minecraft"));
        for launcher in [
            "PrismLauncher",
            "PolyMC",
            "MultiMC",
            "GDLauncher",
            "com.modrinth.theseus",
            "curseforge",
            ".minecraft",
        ] {
            bases.push(base.join(launcher));
        }
    }

    for base in bases {
        collect_minecraft_roots_under(&base, ROOT_SEARCH_DEPTH, &mut roots);
    }

    roots.sort();
    roots.dedup();
    roots
}

/// Bounded search for Minecraft instance roots: a directory is treated as a root
/// once it contains a `logs` or `saves` folder (we stop descending there). This
/// covers `<launcher>/instances/<name>/minecraft`, Modrinth `profiles/<name>`,
/// etc., without an open-ended filesystem walk.
fn collect_minecraft_roots_under(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    // A Minecraft game instance has a `saves/` directory (singleplayer worlds) or
    // an active `logs/latest.log` (servers). A launcher's own top folder may have
    // a `logs/` directory but neither of these, so it is correctly skipped and we
    // descend into its `instances/`.
    if dir.join("saves").is_dir() || dir.join("logs").join("latest.log").is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            collect_minecraft_roots_under(&entry.path(), depth - 1, out);
        }
    }
}

fn parse_marker(line: &str) -> Option<Request> {
    let marker = line.find("[mcfc_rpc]")?;
    let brace = line[marker..].find('{')? + marker;
    let value = snbt::parse_compound(&line[brace..])?;
    let compound = value.as_compound()?;
    if compound.get("mcpipe")?.as_int()? != 1 || compound.get("protocol")?.as_int()? != 2 {
        return None;
    }
    Some(Request {
        pack_id: compound.get("pack")?.as_str()?.to_string(),
        id: compound.get("id")?.as_int()?,
        module: compound.get("mod")?.as_str()?.to_string(),
        function: compound.get("fn")?.as_str()?.to_string(),
        args: match compound.get("args") {
            Some(snbt::Value::List(values)) => values.clone(),
            _ => Vec::new(),
        },
    })
}

fn read_new_lines(path: &Path, offset: &mut u64) -> Vec<String> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    if len < *offset {
        *offset = 0;
    }
    let mut reader = BufReader::new(file);
    if reader.seek(SeekFrom::Start(*offset)).is_err() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut consumed = *offset;
    loop {
        let mut buffer = String::new();
        match reader.read_line(&mut buffer) {
            Ok(0) => break,
            Ok(bytes) if buffer.ends_with('\n') => {
                consumed += bytes as u64;
                lines.push(buffer.trim_end().to_string());
            }
            Ok(_) => break,
            Err(_) => break,
        }
    }
    *offset = consumed;
    lines
}

fn write_inbox(config: &Config, results: &HashMap<i64, PendingResult>) -> Result<(), String> {
    let path = config
        .datapack
        .join("data")
        .join(&config.namespace)
        .join("function")
        .join("rpc")
        .join("inbox.mcfunction");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut body = String::from("# generated by mcfd service\n");
    let mut ids: Vec<_> = results.keys().collect();
    ids.sort();
    for id in ids {
        body.push_str(&format!(
            "data modify storage mcfc:rpc results.{} set value {}\n",
            id, results[id].snbt
        ));
    }
    std::fs::write(path, body).map_err(|e| e.to_string())
}

fn state_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
    let path = PathBuf::from(base).join("MCFC").join("mcfd");
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

fn write_status(state: &Path, packs: &[Pack]) {
    let body = format!("running\ndiscovered_packs={}\n", packs.len());
    let _ = std::fs::write(state.join("status.txt"), body);
}

fn status() -> Result<(), String> {
    let packs = discover_packs();
    println!("mcfd: {} active pack descriptor(s) discovered", packs.len());
    for pack in packs {
        println!("- {} -> {}", pack.config.namespace, pack.log.display());
    }
    Ok(())
}

fn install_task() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let task = format!("\\\"{}\\\" service run", exe.display());
    let status = Command::new("schtasks")
        .args([
            "/Create",
            "/TN",
            "MCFC mcfd",
            "/TR",
            &task,
            "/SC",
            "ONLOGON",
            "/RL",
            "LIMITED",
            "/F",
        ])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("could not create the MCFC mcfd logon task".to_string())
    }
}

fn uninstall_task() -> Result<(), String> {
    // End an active task before removing its registration so Windows releases
    // mcfd.exe and an installer can remove or replace it safely.
    let _ = Command::new("schtasks")
        .args(["/End", "/TN", "MCFC mcfd"])
        .status();
    let status = Command::new("schtasks")
        .args(["/Delete", "/TN", "MCFC mcfd", "/F"])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("could not remove the MCFC mcfd logon task".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entity_death_marker() {
        let request = parse_marker(r#"[12:00:00] [Server thread/INFO]: [mcfc_rpc] {mcpipe:1,protocol:2,pack:"demo",id:5,mod:"time",fn:"now",args:[]} was killed"#).unwrap();
        assert_eq!(request.pack_id, "demo");
        assert_eq!(request.id, 5);
    }

    #[test]
    fn rejects_unmarked_log_line() {
        assert!(parse_marker(r#"{id:5,mod:"time",fn:"now"}"#).is_none());
        assert!(parse_marker(
            r#"[12:00:00] [Server thread/INFO]: {mcpipe:1,protocol:2,pack:"demo",id:5,mod:"time",fn:"now",args:[]}"#
        )
        .is_none());
    }

    /// The datapack puts its full request compound in the pig's resolved custom
    /// name, then kills the pig. Minecraft appends death-message text after the
    /// name, so mcfd must recover the compound and its nested arguments.
    #[test]
    fn parses_whole_compound_marker_with_nested_args() {
        let line = r#"[15:23:01] [Server thread/INFO]: [mcfc_rpc] {args:["https://munin-sou.se/api/v1/cyber/quotes/random",["quote.text","quote.author.name","quote.source"]],fn:"get_json_strings",id:9,mcpipe:1b,mod:"http",namespace:"cyber_quotes",pack:"cyber_quotes",protocol:2,v:1} was killed by magic"#;
        let request = parse_marker(line).expect("whole-compound marker should parse");
        assert_eq!(request.pack_id, "cyber_quotes");
        assert_eq!(request.id, 9);
        assert_eq!(request.module, "http");
        assert_eq!(request.function, "get_json_strings");
        assert_eq!(request.args.len(), 2);
        assert_eq!(
            request.args[0].as_str(),
            Some("https://munin-sou.se/api/v1/cyber/quotes/random")
        );
        match &request.args[1] {
            snbt::Value::List(paths) => assert_eq!(paths.len(), 3),
            other => panic!("expected nested path list, got {other:?}"),
        }
    }

    /// A key missing its `:` (the malformed `{mcpipe,1,...}` that previously
    /// reached the log) must be rejected rather than silently mis-parsed.
    #[test]
    fn rejects_marker_with_missing_colon() {
        assert!(parse_marker(r#"[mcfc_rpc] {mcpipe,1,protocol:2,pack:"demo",id:5}"#).is_none());
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mcfd_disc_{}_{}_{}",
            tag,
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A launcher's own folder (a bare `logs/` with no `latest.log` and no
    /// `saves/`) must not be mistaken for a game instance; discovery should
    /// descend into `instances/<name>/minecraft` instead.
    #[test]
    fn instance_root_search_skips_launcher_folder() {
        let base = temp_dir("roots");
        let launcher = base.join("PrismLauncher");
        std::fs::create_dir_all(launcher.join("logs")).unwrap();
        std::fs::write(launcher.join("logs").join("PrismLauncher-0.log"), "").unwrap();
        let instance = launcher.join("instances").join("inst").join("minecraft");
        std::fs::create_dir_all(instance.join("saves")).unwrap();
        std::fs::create_dir_all(instance.join("logs")).unwrap();
        std::fs::write(instance.join("logs").join("latest.log"), "").unwrap();

        let mut roots = Vec::new();
        collect_minecraft_roots_under(&launcher, ROOT_SEARCH_DEPTH, &mut roots);
        assert_eq!(roots, vec![instance]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Descriptors are found under `<root>/saves/<world>/datapacks/<pack>/`.
    #[test]
    fn finds_descriptor_in_world_datapacks() {
        let root = temp_dir("desc");
        let pack = root
            .join("saves")
            .join("world")
            .join("datapacks")
            .join("mypack");
        std::fs::create_dir_all(&pack).unwrap();
        let descriptor = pack.join(DESCRIPTOR);
        std::fs::write(&descriptor, "namespace='x'\n").unwrap();

        let mut found = Vec::new();
        collect_world_descriptors(&root.join("saves"), &mut found);
        assert_eq!(found, vec![descriptor]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
