//! Global entity-death-backed host bridge for MCFC datapacks.
//!
//! Compiled packs create an off-map, named pig and immediately kill it. The death
//! message carries a command-storage RPC record into Minecraft's `latest.log`.
//! This process discovers generated `mcfd.pack.toml` descriptors, tails their
//! launcher-specific logs, and writes replies into each pack inbox.

mod config;
mod handlers;
mod snbt;

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use config::Config;
use serde::Deserialize;

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

/// Versioned event record emitted by `mcfd-agent` into Minecraft's log. This
/// intentionally travels independently of the SNBT RPC protocol: agent events
/// originate in the JVM rather than in a datapack function.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct AgentEvent {
    protocol: u32,
    event: String,
    source: String,
    payload: String,
    cancelled: bool,
    cancellable: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [service, command] if service == "service" && command == "run" => run_service(),
        [service, command] if service == "service" && command == "install" => install_task(),
        [service, command] if service == "service" && command == "uninstall" => uninstall_task(),
        [service, command] if service == "service" && command == "status" => status(),
        [agent, command] if agent == "agent" && command == "status" => agent_status(),
        [agent, command, pid] if agent == "agent" && command == "attach" => attach_agent(pid),
        _ => Err(
            "usage: mcfd service <run|install|uninstall|status> | mcfd agent <status|attach <pid>>"
                .to_string(),
        ),
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
    // A process receives one attachment attempt per distinct options payload.
    // A fresh Minecraft launch has a new PID and is retried naturally; descriptor
    // changes for a running JVM are retried when their routes/commands change.
    let mut agent_attempts: HashMap<u32, String> = HashMap::new();
    loop {
        while let Ok(updated) = scan_rx.try_recv() {
            packs = updated;
            write_status(&state, &packs);
            ensure_requested_agents_attached(&packs, &mut agent_attempts);
        }

        let mut by_log: HashMap<&Path, Vec<&Pack>> = HashMap::new();
        for pack in &packs {
            by_log.entry(pack.log.as_path()).or_default().push(pack);
        }
        for (log, candidates) in by_log {
            let offset = offsets.entry(log.to_path_buf()).or_insert(0);
            for line in read_new_lines(log, offset) {
                if let Some(event) = parse_agent_event(&line) {
                    for pack in candidates.iter().filter(|pack| pack.config.agent.enabled) {
                        eprintln!(
                            "mcfd: {} agent event {} ({} -> {}, cancelled={}, cancellable={})",
                            pack.config.namespace,
                            event.event,
                            event.source,
                            event.payload,
                            event.cancelled,
                            event.cancellable,
                        );
                    }
                    continue;
                }
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

/// Read the structured suffix of an agent log line. The human-readable prefix
/// is deliberately ignored so it may evolve without breaking the transport.
fn parse_agent_event(line: &str) -> Option<AgentEvent> {
    let marker = "[mcfd-agent]";
    let record = " record=";
    let start = line.find(marker)?;
    let json = &line[start..].split_once(record)?.1;
    let event: AgentEvent = serde_json::from_str(json).ok()?;
    (event.protocol == 1).then_some(event)
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
    let agent_requests = packs
        .iter()
        .filter(|pack| pack.config.agent.enabled)
        .count();
    let body = format!(
        "running\ndiscovered_packs={}\nagent_requests={}\n",
        packs.len(),
        agent_requests
    );
    let _ = std::fs::write(state.join("status.txt"), body);
}

fn status() -> Result<(), String> {
    let packs = discover_packs();
    println!("mcfd: {} active pack descriptor(s) discovered", packs.len());
    for pack in packs {
        let agent = if pack.config.agent.enabled {
            " (agent requested)"
        } else {
            ""
        };
        println!(
            "- {} -> {}{}",
            pack.config.namespace,
            pack.log.display(),
            agent
        );
    }
    Ok(())
}

fn agent_status() -> Result<(), String> {
    let requested: Vec<_> = discover_packs()
        .into_iter()
        .filter(|pack| pack.config.agent.enabled)
        .collect();
    println!(
        "mcfd: {} pack(s) request the optional JVM agent",
        requested.len()
    );
    for pack in requested {
        println!(
            "- {} ({})",
            pack.config.namespace,
            pack.descriptor.display()
        );
    }
    let (agent, launcher) = agent_jars()?;
    println!("agent JAR: {}", agent.display());
    println!("attach launcher: {}", launcher.display());
    println!(
        "dynamic attachment is best-effort; use `mcfd agent attach <pid>` to attach explicitly"
    );
    Ok(())
}

fn attach_agent(pid: &str) -> Result<(), String> {
    let pid = pid
        .parse::<u32>()
        .map_err(|_| format!("invalid JVM pid '{pid}'"))?;
    attach_agent_pid(pid, "")
}

fn attach_agent_pid(pid: u32, options: &str) -> Result<(), String> {
    let (agent, launcher) = agent_jars()?;
    let java = java_for_attach();
    let launcher_arg = launcher.to_string_lossy().to_string();
    let agent_arg = agent.to_string_lossy().to_string();
    let mut command = Command::new(java);
    command.args([
        "--add-modules",
        "jdk.attach",
        "-jar",
        &launcher_arg,
        &pid.to_string(),
        &agent_arg,
    ]);
    if !options.is_empty() {
        command.arg(options);
    }
    let status = command
        .status()
        .map_err(|error| format!("failed to launch the Java Attach API helper: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "agent attachment failed with exit status {status}; set MCFD_JAVA to a JDK with the jdk.attach module"
        ))
    }
}

/// Attach the optional agent only to an unambiguous Java process belonging to
/// the same Minecraft instance as an agent-enabled datapack. The marker is the
/// instance path (`<instance>/logs/latest.log`), which Prism and the official
/// launcher both include in their JVM class path or native-library arguments.
fn ensure_requested_agents_attached(packs: &[Pack], attempts: &mut HashMap<u32, String>) {
    let processes = match running_java_processes() {
        Ok(processes) => processes,
        Err(error) => {
            eprintln!("mcfd: cannot discover Java processes for agent attachment: {error}");
            return;
        }
    };
    let mut requests: BTreeMap<u32, Vec<&Pack>> = BTreeMap::new();
    for pack in packs.iter().filter(|pack| pack.config.agent.enabled) {
        let matches: Vec<_> = processes
            .iter()
            .filter(|process| process_matches_pack(process, pack))
            .collect();
        match matches.as_slice() {
            [] => eprintln!(
                "mcfd: agent requested by '{}' but no matching Minecraft JVM is running",
                pack.config.namespace
            ),
            [process] => requests.entry(process.pid).or_default().push(pack),
            _ => eprintln!(
                "mcfd: agent requested by '{}' but {} Java processes match its instance; not attaching",
                pack.config.namespace,
                matches.len()
            ),
        }
    }
    for (pid, matching_packs) in requests {
        let options = agent_options(&matching_packs);
        if !should_attempt_agent_attach(attempts, pid, &options) {
            continue;
        }
        let names = matching_packs
            .iter()
            .map(|pack| pack.config.namespace.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        match attach_agent_pid(pid, &options) {
            Ok(()) => eprintln!("mcfd: attached optional agent to JVM {} for {}", pid, names),
            Err(error) => eprintln!(
                "mcfd: optional agent attachment for {} failed: {error}",
                names
            ),
        }
    }
}

fn should_attempt_agent_attach(
    attempts: &mut HashMap<u32, String>,
    pid: u32,
    options: &str,
) -> bool {
    if attempts
        .get(&pid)
        .is_some_and(|previous| previous == options)
    {
        return false;
    }
    attempts.insert(pid, options.to_string());
    true
}

fn agent_options(packs: &[&Pack]) -> String {
    let mut deciders = Vec::new();
    let mut routes = Vec::new();
    let mut commands = Vec::new();
    for pack in packs {
        if !pack.config.agent.deciders.is_empty() {
            deciders.push(format!(
                "{}:{}",
                pack.config.namespace,
                pack.config.agent.deciders.join(",")
            ));
        }
        if !pack.config.agent.events.is_empty() {
            routes.push(format!(
                "{}:{}",
                pack.config.namespace,
                pack.config.agent.events.join(",")
            ));
        }
        if !pack.config.agent.commands.is_empty() {
            commands.push(format!(
                "{}:{}",
                pack.config.namespace,
                pack.config.agent.commands.join(",")
            ));
        }
    }
    let mut options = Vec::new();
    if !deciders.is_empty() {
        options.push(format!("deciders={}", deciders.join("|")));
    }
    if !routes.is_empty() {
        options.push(format!("routes={}", routes.join("|")));
    }
    if !commands.is_empty() {
        options.push(format!("commands={}", commands.join("|")));
    }
    options.join(";")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JavaProcess {
    pid: u32,
    command_line: String,
}

fn running_java_processes() -> Result<Vec<JavaProcess>, String> {
    if !cfg!(windows) {
        return Ok(Vec::new());
    }
    // No user-controlled text is interpolated into this script. It emits one
    // tab-delimited record per Java process so parsing stays independent of
    // PowerShell's table formatting and localized headings.
    let script = "Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'java.exe' -or $_.Name -eq 'javaw.exe' } | ForEach-Object { [Console]::WriteLine($_.ProcessId.ToString() + [char]9 + $_.CommandLine) }";
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(parse_java_processes(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn parse_java_processes(output: &str) -> Vec<JavaProcess> {
    output
        .lines()
        .filter_map(|line| {
            let (pid, command_line) = line.split_once('\t')?;
            Some(JavaProcess {
                pid: pid.trim().parse().ok()?,
                command_line: command_line.to_string(),
            })
        })
        .collect()
}

fn process_matches_pack(process: &JavaProcess, pack: &Pack) -> bool {
    let Some(logs) = pack.log.parent() else {
        return false;
    };
    let Some(instance) = logs.parent() else {
        return false;
    };
    let mut markers = vec![normalize_windows_path(&instance.to_string_lossy())];
    // Prism stores game data under `instances/<name>/minecraft`, but starts the
    // JVM with paths rooted at `instances/<name>` (for example its `natives/`
    // directory). Include that parent only for the known Prism layout; doing it
    // for an ordinary `.minecraft` root would be far too broad.
    if instance
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("minecraft"))
    {
        if let Some(prism_instance) = instance.parent() {
            markers.push(normalize_windows_path(&prism_instance.to_string_lossy()));
        }
    }
    let command_line = normalize_windows_path(&process.command_line);
    markers
        .into_iter()
        .any(|marker| !marker.is_empty() && command_line.contains(&marker))
}

fn normalize_windows_path(value: &str) -> String {
    value
        .replace('/', "\\")
        .trim_start_matches("\\\\?\\")
        .to_ascii_lowercase()
}

fn java_for_attach() -> std::ffi::OsString {
    if let Some(java) = std::env::var_os("MCFD_JAVA") {
        return java;
    }
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let candidate = PathBuf::from(home).join("bin").join("java.exe");
        if candidate.is_file() {
            return candidate.into_os_string();
        }
    }
    for root in [
        std::env::var_os("ProgramFiles"),
        std::env::var_os("ProgramW6432"),
    ]
    .into_iter()
    .flatten()
    {
        for vendor in ["Microsoft", "Java"] {
            let directory = PathBuf::from(&root).join(vendor);
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            let mut candidates: Vec<_> = entries
                .flatten()
                .map(|entry| entry.path().join("bin").join("java.exe"))
                .filter(|candidate| candidate.is_file())
                .collect();
            candidates.sort();
            if let Some(candidate) = candidates.pop() {
                return candidate.into_os_string();
            }
        }
    }
    "java".into()
}

fn agent_jars() -> Result<(PathBuf, PathBuf), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let parent = exe
        .parent()
        .ok_or("mcfd executable has no parent directory")?;
    let agent = parent.join("mcfd-agent.jar");
    let launcher = parent.join("mcfd-agent-attach.jar");
    if !agent.is_file() || !launcher.is_file() {
        return Err(format!(
            "agent files are missing beside mcfd.exe (expected '{}' and '{}')",
            agent.display(),
            launcher.display()
        ));
    }
    Ok((agent, launcher))
}

fn install_task() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // `/TR` needs one argument containing a normally quoted executable path.
    // The previous form emitted literal backslashes before each quote (`\"`),
    // which Task Scheduler could not execute for installations under Program
    // Files.
    let task = format!("\"{}\" service run", exe.display());
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
        let _ = remove_run_entry();
        Ok(())
    } else {
        // Some Windows configurations let a standard user install programs but
        // deny task registration.  A per-user Run entry has the same logon
        // behaviour and requires no elevation, so retain autostart instead of
        // leaving an otherwise usable installation dormant.
        install_run_entry(&exe)?;
        eprintln!("mcfd: could not create the logon task; registered per-user startup instead");
        Ok(())
    }
}

fn uninstall_task() -> Result<(), String> {
    // End an active task before removing its registration so Windows releases
    // mcfd.exe and an installer can remove or replace it safely.
    let _ = Command::new("schtasks")
        .args(["/End", "/TN", "MCFC mcfd"])
        .status();
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", "MCFC mcfd", "/F"])
        .status()
        .map_err(|e| e.to_string())?;
    let _ = remove_run_entry();
    Ok(())
}

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "MCFC mcfd";

fn install_run_entry(exe: &Path) -> Result<(), String> {
    let script = state_dir()?.join("mcfd-startup.vbs");
    std::fs::write(&script, startup_script_body(exe)).map_err(|e| e.to_string())?;
    let command = run_entry_command(&script);
    let status = Command::new("reg")
        .args([
            "add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &command, "/f",
        ])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("could not register MCFC mcfd for user logon".to_string())
    }
}

fn startup_script_body(exe: &Path) -> String {
    format!(
        r#"Set shell = CreateObject("WScript.Shell")
shell.Run """{}"" service run", 0, False
"#,
        exe.display()
    )
}

fn run_entry_command(script: &Path) -> String {
    format!("wscript.exe //B //Nologo \"{}\"", script.display())
}

fn remove_run_entry() -> Result<(), String> {
    let _ = Command::new("reg")
        .args(["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])
        .status()
        .map_err(|e| e.to_string())?;
    if let Ok(state) = state_dir() {
        let _ = std::fs::remove_file(state.join("mcfd-startup.vbs"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduled_task_action_quotes_the_executable_without_backslashes() {
        let exe = PathBuf::from(r"C:\Program Files\MCFC\mcfd\mcfd.exe");
        let task = format!("\"{}\" service run", exe.display());
        assert_eq!(task, r#""C:\Program Files\MCFC\mcfd\mcfd.exe" service run"#);
    }

    #[test]
    fn run_entry_uses_a_hidden_wscript_launcher() {
        let exe = PathBuf::from(r"C:\Program Files\MCFC\mcfd\mcfd.exe");
        assert_eq!(
            startup_script_body(&exe),
            "Set shell = CreateObject(\"WScript.Shell\")\nshell.Run \"\"\"C:\\Program Files\\MCFC\\mcfd\\mcfd.exe\"\" service run\", 0, False\n"
        );

        let script = PathBuf::from(r"C:\Users\Oliver\AppData\Local\MCFC\mcfd\mcfd-startup.vbs");
        assert_eq!(
            run_entry_command(&script),
            r#"wscript.exe //B //Nologo "C:\Users\Oliver\AppData\Local\MCFC\mcfd\mcfd-startup.vbs""#
        );
    }

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

    #[test]
    fn parses_versioned_agent_event_record() {
        let line = r#"[21:55:26] [Netty Local IO #2/INFO]: [STDERR]: [mcfd-agent] event=chat source=ServerGamePacketListenerImpl payload=ServerboundChatPacket cancelled=false record={"protocol":1,"event":"chat","source":"ServerGamePacketListenerImpl","payload":"ServerboundChatPacket","cancelled":false,"cancellable":true}"#;
        let event = parse_agent_event(line).expect("agent record should parse");
        assert_eq!(event.event, "chat");
        assert_eq!(event.source, "ServerGamePacketListenerImpl");
        assert_eq!(event.payload, "ServerboundChatPacket");
        assert!(event.cancellable);
        assert!(!event.cancelled);
    }

    #[test]
    fn rejects_unknown_agent_event_protocol() {
        let line = r#"[mcfd-agent] event=chat record={"protocol":2,"event":"chat","source":"x","payload":"y","cancelled":false,"cancellable":true}"#;
        assert!(parse_agent_event(line).is_none());
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

    #[test]
    fn agent_process_matching_is_scoped_to_the_pack_instance() {
        let pack = Pack {
            config: Config {
                protocol: 2,
                pack_id: "demo".to_string(),
                namespace: "demo".to_string(),
                log: None,
                datapack: PathBuf::from(
                    r"C:\Users\Oliver\AppData\Roaming\PrismLauncher\instances\26.2\minecraft\saves\world\datapacks\demo",
                ),
                result_ttl_secs: 300,
                capabilities: config::Capabilities::default(),
                agent: config::AgentConfig {
                    enabled: true,
                    events: Vec::new(),
                    commands: Vec::new(),
                    cancel_events: Vec::new(),
                    deciders: Vec::new(),
                },
                secrets: HashMap::new(),
            },
            descriptor: PathBuf::from("mcfd.pack.toml"),
            log: PathBuf::from(
                r"C:\Users\Oliver\AppData\Roaming\PrismLauncher\instances\26.2\minecraft\logs\latest.log",
            ),
        };
        let matching = JavaProcess {
            pid: 42,
            command_line: "javaw.exe -Djava.library.path=C:/Users/Oliver/AppData/Roaming/PrismLauncher/instances/26.2/natives".to_string(),
        };
        let other = JavaProcess {
            pid: 43,
            command_line: "javaw.exe -Djava.library.path=C:/Users/Oliver/AppData/Roaming/PrismLauncher/instances/other/minecraft/natives".to_string(),
        };
        assert!(process_matches_pack(&matching, &pack));
        assert!(!process_matches_pack(&other, &pack));
    }

    #[test]
    fn parses_tab_delimited_java_process_output() {
        let processes = parse_java_processes(
            "42\tjavaw.exe -Djava.library.path=C:/Prism/instances/demo/natives\n",
        );
        assert_eq!(
            processes,
            vec![JavaProcess {
                pid: 42,
                command_line: "javaw.exe -Djava.library.path=C:/Prism/instances/demo/natives"
                    .to_string(),
            }]
        );
    }

    #[test]
    fn agent_options_merge_cancellation_and_pack_event_routes() {
        let first = Pack {
            config: Config {
                protocol: 2,
                pack_id: "first".to_string(),
                namespace: "first".to_string(),
                log: None,
                datapack: PathBuf::from("first"),
                result_ttl_secs: 300,
                capabilities: config::Capabilities::default(),
                agent: config::AgentConfig {
                    enabled: true,
                    events: vec!["chat".to_string(), "block_break".to_string()],
                    commands: vec!["first_command".to_string()],
                    cancel_events: Vec::new(),
                    deciders: vec!["chat".to_string()],
                },
                secrets: HashMap::new(),
            },
            descriptor: PathBuf::from("first/mcfd.pack.toml"),
            log: PathBuf::from("first/logs/latest.log"),
        };
        let second = Pack {
            config: Config {
                namespace: "second".to_string(),
                agent: config::AgentConfig {
                    enabled: true,
                    events: vec!["inventory_click".to_string()],
                    commands: vec!["second_command".to_string()],
                    cancel_events: Vec::new(),
                    deciders: vec!["block_break".to_string()],
                },
                ..first.config.clone()
            },
            descriptor: PathBuf::from("second/mcfd.pack.toml"),
            log: PathBuf::from("second/logs/latest.log"),
        };
        assert_eq!(
            agent_options(&[&first, &second]),
            "deciders=first:chat|second:block_break;routes=first:chat,block_break|second:inventory_click;commands=first:first_command|second:second_command"
        );
    }

    #[test]
    fn agent_attach_attempts_repeat_when_options_change() {
        let mut attempts = HashMap::new();
        assert!(should_attempt_agent_attach(
            &mut attempts,
            42,
            "routes=first:chat"
        ));
        assert!(!should_attempt_agent_attach(
            &mut attempts,
            42,
            "routes=first:chat"
        ));
        assert!(should_attempt_agent_attach(
            &mut attempts,
            42,
            "routes=first:chat;commands=first:status"
        ));
        assert!(!should_attempt_agent_attach(
            &mut attempts,
            42,
            "routes=first:chat;commands=first:status"
        ));
    }
}
