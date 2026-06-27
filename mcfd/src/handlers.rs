//! Capability handlers. Each returns the SNBT body of the result compound that
//! gets injected into `mcfc:rpc results.<id>`. Every result carries `ok`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value as JsonValue;

use crate::config::Config;
use crate::snbt::{self, Value};
use crate::Request;

const MAX_BODY: usize = 32 * 1024;

pub fn dispatch(config: &Config, request: &Request) -> String {
    match (request.module.as_str(), request.function.as_str()) {
        ("http", "get") => http(config, request, false),
        ("http", "get_json_string") => http_json_string(config, request),
        ("http", "get_json_strings") => http_json_strings(config, request),
        ("http", "post") => http(config, request, true),
        ("file", "read") => file_read(config, request),
        ("file", "write") => file_write(config, request),
        ("kv", "get") => kv_get(config, request),
        ("kv", "set") => kv_set(config, request),
        ("db", "exec") => db_exec(config, request, false),
        ("db", "query") => db_exec(config, request, true),
        ("time", "now") => time_now(config),
        ("rand", "int") => rand_int(config, request),
        // Connectivity probe: always answerable, no capability required, so a
        // datapack can confirm the daemon is reachable.
        ("mcfd", "ping") => "{ok:1b,pong:1b}".to_string(),
        _ => err("unknown host function"),
    }
}

fn ok_only() -> String {
    "{ok:1b}".to_string()
}

fn err(message: &str) -> String {
    format!("{{ok:0b,err:{}}}", snbt::escape_string(message))
}

fn arg_str(request: &Request, index: usize) -> Option<String> {
    request.args.get(index).map(Value::to_arg_string)
}

// ---- http -----------------------------------------------------------------

fn http(config: &Config, request: &Request, post: bool) -> String {
    let Some(caps) = &config.capabilities.http else {
        return err("http capability disabled");
    };
    let Some(url) = arg_str(request, 0) else {
        return err("missing url");
    };
    if !host_allowed(&url, &caps.allow_domains) {
        return err("domain not allowed");
    }

    let response = match http_request(config, caps, &url, post, request) {
        Ok(response) => response,
        Err(message) => return http_err(0, &message, ""),
    };

    match response {
        Ok(resp) => http_result(resp.status(), resp),
        // ureq surfaces non-2xx as Error::Status with the response attached.
        Err(ureq::Error::Status(status, resp)) => http_result(status, resp),
        Err(error) => err(&format!("request failed: {}", error)),
    }
}

/// Fetch JSON and expose a single string field at a dot-separated object path.
/// This keeps JSON parsing on the host while giving datapacks a typed string
/// response suitable for text components.
fn http_json_string(config: &Config, request: &Request) -> String {
    let Some(caps) = &config.capabilities.http else {
        return http_err(0, "http capability disabled", "");
    };
    let Some(url) = arg_str(request, 0) else {
        return http_err(0, "missing url", "");
    };
    let Some(path) = arg_str(request, 1) else {
        return http_err(0, "missing JSON path", "");
    };
    if !host_allowed(&url, &caps.allow_domains) {
        return http_err(0, "domain not allowed", "");
    }

    let response = match http_request(config, caps, &url, false, request) {
        Ok(response) => response,
        Err(message) => return http_err(0, &message, ""),
    };
    match response {
        Ok(resp) => json_string_result(resp.status(), resp, &path),
        Err(ureq::Error::Status(status, resp)) => {
            let body = read_body(resp);
            http_err(status, "HTTP request returned a non-success status", &body)
        }
        Err(error) => http_err(0, &format!("request failed: {}", error), ""),
    }
}

/// Fetch JSON once and expose multiple string fields, in the order of the
/// requested dot-separated object paths. This avoids mixing fields from
/// separate requests when an endpoint returns a random record.
fn http_json_strings(config: &Config, request: &Request) -> String {
    let Some(caps) = &config.capabilities.http else {
        return json_strings_err(0, "http capability disabled");
    };
    let Some(url) = arg_str(request, 0) else {
        return json_strings_err(0, "missing url");
    };
    let Some(Value::List(paths)) = request.args.get(1) else {
        return json_strings_err(0, "missing JSON paths");
    };
    let paths: Vec<String> = paths.iter().map(Value::to_arg_string).collect();
    if paths.is_empty() {
        return json_strings_err(0, "JSON paths must not be empty");
    }
    if !host_allowed(&url, &caps.allow_domains) {
        return json_strings_err(0, "domain not allowed");
    }

    let response = match http_request(config, caps, &url, false, request) {
        Ok(response) => response,
        Err(message) => return json_strings_err(0, &message),
    };
    match response {
        Ok(resp) => json_strings_result(resp.status(), resp, &paths),
        Err(ureq::Error::Status(status, _)) => {
            json_strings_err(status, "HTTP request returned a non-success status")
        }
        Err(error) => json_strings_err(0, &format!("request failed: {}", error)),
    }
}

fn http_request(
    config: &Config,
    caps: &crate::config::HttpCaps,
    url: &str,
    post: bool,
    request: &Request,
) -> Result<Result<ureq::Response, ureq::Error>, String> {
    // Always time-bound the request so a slow/unreachable host can never hang the
    // single-threaded daemon.
    let timeout = Duration::from_secs(10);
    let token = bearer_token(config, caps)?;
    let response = if post {
        let body = arg_str(request, 1).unwrap_or_default();
        let request = ureq::post(url)
            .timeout(timeout)
            .set("Content-Type", "text/plain");
        let request = match &token {
            Some(token) => request.set("Authorization", &format!("Bearer {token}")),
            None => request,
        };
        request.send_string(&body)
    } else {
        let request = ureq::get(url).timeout(timeout);
        match token {
            Some(token) => request
                .set("Authorization", &format!("Bearer {token}"))
                .call(),
            None => request.call(),
        }
    };
    Ok(response)
}

fn bearer_token(config: &Config, caps: &crate::config::HttpCaps) -> Result<Option<String>, String> {
    let Some(name) = &caps.bearer_token_env else {
        return Ok(None);
    };
    let token = config
        .secret(name)
        .ok_or_else(|| format!("Bearer token environment variable '{}' is not set", name))?;
    if token.trim().is_empty() {
        return Err(format!(
            "Bearer token environment variable '{}' is empty",
            name
        ));
    }
    Ok(Some(token))
}

fn http_result(status: u16, resp: ureq::Response) -> String {
    let truncated = read_body(resp);
    format!(
        "{{ok:1b,status:{},body:{}}}",
        status,
        snbt::escape_string(&truncated)
    )
}

fn json_string_result(status: u16, resp: ureq::Response, path: &str) -> String {
    let body = read_body(resp);
    let value = match json_string_at(&body, path) {
        Ok(value) => value,
        Err(message) => return http_err(status, &message, &body),
    };
    format!(
        "{{ok:1b,status:{},body:{}}}",
        status,
        snbt::escape_string(&value.chars().take(MAX_BODY).collect::<String>())
    )
}

fn json_strings_result(status: u16, resp: ureq::Response, paths: &[String]) -> String {
    let body = read_body(resp);
    let values = match json_strings_at(&body, paths) {
        Ok(values) => values,
        Err(message) => return json_strings_err(status, &message),
    };
    let values = values
        .iter()
        .map(|value| snbt::escape_string(value))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{ok:1b,status:{},values:[{}]}}", status, values)
}

fn read_body(resp: ureq::Response) -> String {
    resp.into_string()
        .unwrap_or_default()
        .chars()
        .take(MAX_BODY)
        .collect()
}

fn http_err(status: u16, message: &str, body: &str) -> String {
    format!(
        "{{ok:0b,status:{},body:{},err:{}}}",
        status,
        snbt::escape_string(body),
        snbt::escape_string(message)
    )
}

fn json_strings_err(status: u16, message: &str) -> String {
    format!(
        "{{ok:0b,status:{},values:[],err:{}}}",
        status,
        snbt::escape_string(message)
    )
}

fn json_string_at(body: &str, path: &str) -> Result<String, String> {
    let document: JsonValue =
        serde_json::from_str(body).map_err(|_| "response body is not valid JSON".to_string())?;
    json_string_from_document(&document, path)
}

fn json_strings_at(body: &str, paths: &[String]) -> Result<Vec<String>, String> {
    let document: JsonValue =
        serde_json::from_str(body).map_err(|_| "response body is not valid JSON".to_string())?;
    paths
        .iter()
        .map(|path| json_string_from_document(&document, path))
        .collect()
}

fn json_string_from_document(document: &JsonValue, path: &str) -> Result<String, String> {
    if path.is_empty() || path.split('.').any(str::is_empty) {
        return Err("JSON path must contain dot-separated object keys".to_string());
    }
    let mut value = document;
    for key in path.split('.') {
        value = value
            .get(key)
            .ok_or_else(|| format!("JSON string path '{}' was not found", path))?;
    }
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("JSON path '{}' is not a string", path))
}

fn host_allowed(url: &str, allow: &[String]) -> bool {
    let without_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = without_scheme
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or("");
    allow
        .iter()
        .any(|domain| host == domain || host.ends_with(&format!(".{}", domain)))
}

// ---- file / kv ------------------------------------------------------------

/// Resolve `relative` under `root`, rejecting anything that escapes the sandbox.
fn sandbox(root: &Path, relative: &str) -> Option<PathBuf> {
    let candidate = root.join(relative);
    let root_abs = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    // Canonicalize the parent (the file itself may not exist yet for writes).
    let parent = candidate.parent().unwrap_or(&candidate);
    let parent_abs = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    if parent_abs.starts_with(&root_abs) {
        Some(candidate)
    } else {
        None
    }
}

fn file_read(config: &Config, request: &Request) -> String {
    let Some(caps) = &config.capabilities.file else {
        return err("file capability disabled");
    };
    let Some(rel) = arg_str(request, 0) else {
        return err("missing path");
    };
    let Some(path) = sandbox(&caps.root, &rel) else {
        return err("path escapes sandbox");
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let truncated: String = content.chars().take(MAX_BODY).collect();
            format!("{{ok:1b,content:{}}}", snbt::escape_string(&truncated))
        }
        Err(error) => err(&format!("read failed: {}", error)),
    }
}

fn file_write(config: &Config, request: &Request) -> String {
    let Some(caps) = &config.capabilities.file else {
        return err("file capability disabled");
    };
    let (Some(rel), Some(content)) = (arg_str(request, 0), arg_str(request, 1)) else {
        return err("missing path or content");
    };
    let Some(path) = sandbox(&caps.root, &rel) else {
        return err("path escapes sandbox");
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, content) {
        Ok(()) => ok_only(),
        Err(error) => err(&format!("write failed: {}", error)),
    }
}

fn kv_get(config: &Config, request: &Request) -> String {
    let Some(caps) = &config.capabilities.kv else {
        return err("kv capability disabled");
    };
    let Some(key) = arg_str(request, 0) else {
        return err("missing key");
    };
    let Some(path) = sandbox(&caps.root, &kv_filename(&key)) else {
        return err("bad key");
    };
    match std::fs::read_to_string(&path) {
        Ok(value) => format!("{{ok:1b,value:{}}}", snbt::escape_string(&value)),
        Err(_) => format!("{{ok:1b,value:{}}}", snbt::escape_string("")),
    }
}

fn kv_set(config: &Config, request: &Request) -> String {
    let Some(caps) = &config.capabilities.kv else {
        return err("kv capability disabled");
    };
    let (Some(key), Some(value)) = (arg_str(request, 0), arg_str(request, 1)) else {
        return err("missing key or value");
    };
    let Some(path) = sandbox(&caps.root, &kv_filename(&key)) else {
        return err("bad key");
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, value) {
        Ok(()) => ok_only(),
        Err(error) => err(&format!("write failed: {}", error)),
    }
}

/// Map a key to a safe filename (alphanumerics preserved, others to `_`).
fn kv_filename(key: &str) -> String {
    let safe: String = key
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    format!("{}.kv", safe)
}

// ---- db -------------------------------------------------------------------

fn db_exec(config: &Config, request: &Request, query: bool) -> String {
    let Some(caps) = &config.capabilities.db else {
        return err("db capability disabled");
    };
    let Some(sql) = arg_str(request, 0) else {
        return err("missing sql");
    };
    let params: Vec<String> = match request.args.get(1) {
        Some(Value::List(items)) => items.iter().map(Value::to_arg_string).collect(),
        _ => Vec::new(),
    };

    let connection = match rusqlite::Connection::open(&caps.path) {
        Ok(connection) => connection,
        Err(error) => return err(&format!("open failed: {}", error)),
    };
    let bound: Vec<&dyn rusqlite::ToSql> = params
        .iter()
        .map(|value| value as &dyn rusqlite::ToSql)
        .collect();

    if query {
        db_query(&connection, &sql, &bound)
    } else {
        match connection.execute(&sql, bound.as_slice()) {
            Ok(rows) => format!("{{ok:1b,rows_affected:{}}}", rows),
            Err(error) => err(&format!("exec failed: {}", error)),
        }
    }
}

fn db_query(
    connection: &rusqlite::Connection,
    sql: &str,
    bound: &[&dyn rusqlite::ToSql],
) -> String {
    let mut statement = match connection.prepare(sql) {
        Ok(statement) => statement,
        Err(error) => return err(&format!("prepare failed: {}", error)),
    };
    let columns: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(|name| name.to_string())
        .collect();
    let mut rows = match statement.query(bound) {
        Ok(rows) => rows,
        Err(error) => return err(&format!("query failed: {}", error)),
    };
    let mut out_rows: Vec<String> = Vec::new();
    loop {
        match rows.next() {
            Ok(Some(row)) => {
                let mut fields: Vec<String> = Vec::new();
                for (index, column) in columns.iter().enumerate() {
                    let value: String = row
                        .get::<_, rusqlite::types::Value>(index)
                        .map(value_to_string)
                        .unwrap_or_default();
                    fields.push(format!(
                        "{}:{}",
                        safe_key(column),
                        snbt::escape_string(&value)
                    ));
                }
                out_rows.push(format!("{{{}}}", fields.join(",")));
            }
            Ok(None) => break,
            Err(error) => return err(&format!("row failed: {}", error)),
        }
    }
    format!(
        "{{ok:1b,rows_affected:{},rows:[{}]}}",
        out_rows.len(),
        out_rows.join(",")
    )
}

fn value_to_string(value: rusqlite::types::Value) -> String {
    use rusqlite::types::Value as V;
    match value {
        V::Null => String::new(),
        V::Integer(value) => value.to_string(),
        V::Real(value) => value.to_string(),
        V::Text(value) => value,
        V::Blob(_) => "<blob>".to_string(),
    }
}

fn safe_key(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

// ---- time / rand ----------------------------------------------------------

fn time_now(config: &Config) -> String {
    if !config.capabilities.time {
        return err("time capability disabled");
    }
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    format!(
        "{{ok:1b,unix:{},iso:{}}}",
        unix,
        snbt::escape_string(&iso8601_utc(unix))
    )
}

fn rand_int(config: &Config, request: &Request) -> String {
    if !config.capabilities.rand {
        return err("rand capability disabled");
    }
    let min = request.args.first().and_then(Value::as_int).unwrap_or(0);
    let max = request.args.get(1).and_then(Value::as_int).unwrap_or(0);
    if max < min {
        return err("max < min");
    }
    let span = (max - min + 1) as u64;
    let value = min + (next_random() % span) as i64;
    format!("{{ok:1b,value:{}}}", value)
}

/// Time-seeded xorshift PRNG. Adequate for gameplay randomness; not cryptographic.
fn next_random() -> u64 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = Cell::new(seed());
    }
    STATE.with(|state| {
        let mut x = state.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        state.set(x);
        x
    })
}

fn seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0x9e3779b97f4a7c15);
    nanos | 1
}

/// Minimal UTC ISO-8601 formatting (no external crate).
fn iso8601_utc(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (hour, minute, second) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

/// Convert days-since-epoch to (year, month, day). From Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentConfig, Capabilities, Config, HttpCaps};
    use std::path::PathBuf;

    fn config_with(capabilities: Capabilities) -> Config {
        Config {
            protocol: 2,
            pack_id: "test".to_string(),
            namespace: "test".to_string(),
            log: None,
            datapack: PathBuf::from("."),
            result_ttl_secs: 10,
            capabilities,
            agent: AgentConfig::default(),
            secrets: std::collections::HashMap::new(),
        }
    }

    fn request(module: &str, function: &str, args: Vec<Value>) -> Request {
        Request {
            pack_id: "test".to_string(),
            id: 1,
            module: module.to_string(),
            function: function.to_string(),
            args,
        }
    }

    #[test]
    fn time_now_produces_unix_result() {
        let config = config_with(Capabilities {
            time: true,
            ..Default::default()
        });
        let result = dispatch(&config, &request("time", "now", vec![]));
        assert!(result.starts_with("{ok:1b,unix:"), "got {result}");
        assert!(result.contains("iso:"));
    }

    #[test]
    fn disabled_capability_reports_error() {
        let config = config_with(Capabilities::default());
        let result = dispatch(&config, &request("time", "now", vec![]));
        assert!(result.starts_with("{ok:0b"), "got {result}");
    }

    #[test]
    fn ping_answers_without_any_capability() {
        let config = config_with(Capabilities::default());
        let result = dispatch(&config, &request("mcfd", "ping", vec![]));
        assert_eq!(result, "{ok:1b,pong:1b}");
    }

    #[test]
    fn rand_int_stays_in_range() {
        let config = config_with(Capabilities {
            rand: true,
            ..Default::default()
        });
        for _ in 0..50 {
            let result = dispatch(
                &config,
                &request("rand", "int", vec![Value::Int(1), Value::Int(6)]),
            );
            let value: i64 = result
                .trim_start_matches("{ok:1b,value:")
                .trim_end_matches('}')
                .parse()
                .expect("value");
            assert!((1..=6).contains(&value), "out of range: {value}");
        }
    }

    #[test]
    fn http_blocks_disallowed_domains() {
        assert!(host_allowed(
            "https://api.example.com/x",
            &["api.example.com".to_string()]
        ));
        assert!(host_allowed(
            "https://sub.example.com/x",
            &["example.com".to_string()]
        ));
        assert!(!host_allowed(
            "https://evil.com/x",
            &["api.example.com".to_string()]
        ));
    }

    #[test]
    fn bearer_token_requires_a_present_nonempty_environment_value() {
        let caps = HttpCaps {
            allow_domains: vec!["example.com".to_string()],
            bearer_token_env: Some("MCFD_MISSING_TEST_BEARER_TOKEN".to_string()),
        };
        assert!(bearer_token(&config_with(Capabilities::default()), &caps).is_err());
    }

    #[test]
    fn json_string_helpers_extract_nested_values_in_one_document() {
        let body =
            r#"{"quote":{"text":"Stay curious","author":{"name":"Munin"},"source":"Cyber"}}"#;
        assert_eq!(json_string_at(body, "quote.text").unwrap(), "Stay curious");
        assert_eq!(
            json_strings_at(
                body,
                &["quote.text".to_string(), "quote.author.name".to_string()]
            )
            .unwrap(),
            vec!["Stay curious".to_string(), "Munin".to_string()]
        );
    }

    #[test]
    fn json_string_helpers_reject_invalid_or_non_string_paths() {
        assert!(json_string_at("not json", "quote.text").is_err());
        assert!(json_string_at(r#"{"quote":{"votes":4}}"#, "quote.missing").is_err());
        assert!(json_string_at(r#"{"quote":{"votes":4}}"#, "quote.votes").is_err());
    }

    #[test]
    fn json_string_error_results_preserve_status_and_failure() {
        let result = json_strings_err(404, "HTTP request returned a non-success status");
        assert!(
            result.starts_with("{ok:0b,status:404,values:[]"),
            "got {result}"
        );
    }
}
