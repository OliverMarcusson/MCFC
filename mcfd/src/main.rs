//! `mcfd` — the host-bridge helper daemon for mcfc datapacks.
//!
//! Transport (single-player friendly, no mod loader, no RCON):
//!   * out: the datapack prints a `[mcfc_rpc]{...}` marker line via `tellraw`;
//!     `mcfd` tails `logs/latest.log` and parses it.
//!   * in:  `mcfd` writes `data modify storage mcfc:rpc results.<id> set value {...}`
//!     lines into the datapack's `rpc/inbox` function; the datapack `/reload`s and
//!     runs it (driven by the generated `rpc/pump`).

mod config;
mod handlers;
mod snbt;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use config::Config;

/// A computed result awaiting (or recently) delivery, kept in the inbox until it
/// expires (assumed delivered after a reload cycle).
struct PendingResult {
    /// SNBT compound body, e.g. `{ok:1b,status:200,body:"..."}`.
    snbt: String,
    expires: Instant,
}

fn main() {
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("mcfd.toml"));

    let config = match Config::load(&config_path) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("mcfd: failed to load {}: {}", config_path.display(), error);
            std::process::exit(1);
        }
    };

    eprintln!(
        "mcfd: watching {} for namespace '{}'",
        config.log_path.display(),
        config.namespace
    );

    if let Err(error) = run(&config) {
        eprintln!("mcfd: {}", error);
        std::process::exit(1);
    }
}

fn run(config: &Config) -> Result<(), String> {
    let poll = Duration::from_millis(config.poll_ms);
    let ttl = Duration::from_secs(config.result_ttl_secs);
    let mut results: HashMap<i64, PendingResult> = HashMap::new();
    let mut offset = 0u64;

    loop {
        let mut dirty = false;
        for line in read_new_lines(&config.log_path, &mut offset) {
            if let Some(request) = parse_marker(&line) {
                if results.contains_key(&request.id) {
                    continue; // already handled this id
                }
                eprintln!(
                    "mcfd: request #{} {}.{}",
                    request.id, request.module, request.function
                );
                let snbt = handlers::dispatch(config, &request);
                eprintln!("mcfd: result #{} -> {}", request.id, truncate(&snbt, 160));
                results.insert(
                    request.id,
                    PendingResult {
                        snbt,
                        expires: Instant::now() + ttl,
                    },
                );
                dirty = true;
            }
        }

        let now = Instant::now();
        let before = results.len();
        results.retain(|_, pending| pending.expires > now);
        if results.len() != before {
            dirty = true;
        }

        if dirty {
            if let Err(error) = write_inbox(config, &results) {
                eprintln!("mcfd: failed to write inbox: {}", error);
            }
        }

        std::thread::sleep(poll);
    }
}

/// A parsed host request from a log marker.
pub struct Request {
    pub id: i64,
    pub module: String,
    pub function: String,
    pub args: Vec<snbt::Value>,
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let prefix: String = value.chars().take(max).collect();
        format!("{}…", prefix)
    }
}

fn parse_marker(line: &str) -> Option<Request> {
    let marker = line.find("[mcfc_rpc]")?;
    let rest = &line[marker + "[mcfc_rpc]".len()..];
    let brace = rest.find('{')?;
    let value = snbt::parse_compound(&rest[brace..])?;
    let compound = value.as_compound()?;

    let id = compound.get("id")?.as_int()?;
    let module = compound.get("mod")?.as_str()?.to_string();
    let function = compound.get("fn")?.as_str()?.to_string();
    let args = match compound.get("args") {
        Some(snbt::Value::List(items)) => items.clone(),
        _ => Vec::new(),
    };
    Some(Request {
        id,
        module,
        function,
        args,
    })
}

/// Read newly appended lines since `offset`, advancing it. Resets if the file
/// shrank (log rotation).
fn read_new_lines(path: &std::path::Path, offset: &mut u64) -> Vec<String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    if len < *offset {
        *offset = 0; // log was rotated/truncated
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
            Ok(bytes) => {
                consumed += bytes as u64;
                // Only treat a line as complete if it ended with a newline.
                if buffer.ends_with('\n') {
                    lines.push(buffer.trim_end().to_string());
                } else {
                    // Partial trailing line: rewind so we re-read it next time.
                    consumed -= bytes as u64;
                    break;
                }
            }
            Err(_) => break,
        }
    }
    *offset = consumed;
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_marker_from_log_line() {
        let line = r#"[12:00:00] [Server thread/INFO]: [mcfc_rpc]{id: 5, v: 1, mod: "time", fn: "now", args: []}"#;
        let request = parse_marker(line).expect("should parse marker");
        assert_eq!(request.id, 5);
        assert_eq!(request.module, "time");
        assert_eq!(request.function, "now");
        assert!(request.args.is_empty());
    }

    #[test]
    fn ignores_non_marker_lines() {
        assert!(parse_marker("[12:00:00] [Server thread/INFO]: hello world").is_none());
    }
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
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut body = String::from("# generated by mcfd\n");
    let mut ids: Vec<&i64> = results.keys().collect();
    ids.sort();
    for id in ids {
        let pending = &results[id];
        body.push_str(&format!(
            "data modify storage mcfc:rpc results.{} set value {}\n",
            id, pending.snbt
        ));
    }
    std::fs::write(&path, body).map_err(|error| error.to_string())
}
