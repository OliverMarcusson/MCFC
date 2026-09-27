//! Whole-pack optimizer. Runs on the emitted datapack, after the backend has
//! turned every function into plain command text, so it sees the real call
//! graph including blocks, continuations and macro helpers.
//!
//! Passes, in order:
//! 1. Guard removal: a forward dataflow analysis tracks which guard flags
//!    (`execute if score <flag> mcfc matches 0 run ...`) may be nonzero at each
//!    line and drops the checks it proves true.
//! 2. Inlining: calls to one-line functions become the line itself; a bare
//!    call to a function with a single caller is replaced by its body.
//! 3. Temp propagation: a backend temp written once and read once has its
//!    source (a score, a storage path or a constant) substituted into the read.
//! 4. Early return: a run of lines all guarded by the same flag becomes one
//!    `return` check followed by unguarded lines.
//! 5. `run execute` chains are flattened.
//! 6. Identical functions are merged.
//! 7. Functions nothing references are dropped, and so are setup lines for
//!    scores nothing reads.
//!
//! The analysis is conservative: anything it cannot classify (dynamic calls,
//! scheduled or string-referenced functions, public entry points) starts with
//! every flag unknown.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub fn optimize(files: &mut BTreeMap<String, String>) {
    let mut pack = Pack::load(files);
    pack.inline_rounds(false);
    pack.insert_early_returns();
    pack.simplify();
    // Copies of small bodies repeat their temps, so they wait until the temps
    // are gone. Simplifying can shrink more bodies under the limit.
    for _ in 0..4 {
        let inlined = pack.inline_rounds(true);
        pack.simplify();
        if !inlined {
            break;
        }
    }
    pack.pool_constants();
    pack.remove_known_guards(true);
    while pack.remove_dead_scores() {}
    pack.flatten_execute_chains();
    pack.merge_identical();
    pack.remove_unreachable();
    pack.store(files);
}

/// A bare call to a body this short is copied in whoever else calls it.
const INLINE_MAX_LINES: usize = 4;

/// An early return pays for itself once it replaces this many guards.
const EARLY_RETURN_MIN_LINES: usize = 3;

struct Pack {
    /// Function id (`ns:path`) to its commands, comments and blank lines removed.
    funcs: BTreeMap<String, Vec<String>>,
    /// Function ids referenced from non-function files (tags, descriptors).
    external_refs: BTreeSet<String>,
    /// Ids of function files present before optimizing, so removed ones can be deleted.
    original: BTreeSet<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RefKind {
    /// `function X`, possibly under `execute`.
    Call {
        /// `with ...` macro arguments follow.
        with_args: bool,
        /// The caller reads the return value (`return run`, `if function`, `store`).
        observed: bool,
        /// The call can run several times from one line (`as`, `at`, macro prefix).
        multi: bool,
    },
    /// `schedule function X` or `schedule clear X`.
    Schedule,
    /// Any other mention, such as a continuation name stored in NBT.
    Other,
}

struct Ref {
    start: usize,
    end: usize,
    id: String,
    kind: RefKind,
}

fn function_id(path: &str) -> Option<String> {
    let rest = path.strip_prefix("data/")?;
    let (namespace, rest) = rest.split_once('/')?;
    let name = rest
        .strip_prefix("function/")?
        .strip_suffix(".mcfunction")?;
    Some(format!("{namespace}:{name}"))
}

fn function_path(id: &str) -> String {
    let (namespace, name) = id.split_once(':').expect("function ids have a namespace");
    format!("data/{namespace}/function/{name}.mcfunction")
}

fn is_id_char(byte: u8, right: bool) -> bool {
    byte.is_ascii_lowercase()
        || byte.is_ascii_digit()
        || matches!(byte, b'_' | b'-' | b'.')
        || (right && byte == b'/')
}

/// Every `namespace:path` token in `text`, in order.
fn id_tokens(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    for (colon, &byte) in bytes.iter().enumerate() {
        if byte != b':' {
            continue;
        }
        let mut start = colon;
        while start > 0 && is_id_char(bytes[start - 1], false) {
            start -= 1;
        }
        let mut end = colon + 1;
        while end < bytes.len() && is_id_char(bytes[end], true) {
            end += 1;
        }
        if start < colon && end > colon + 1 {
            tokens.push((start, end));
        }
    }
    tokens
}

fn classify(line: &str, start: usize, end: usize) -> RefKind {
    let before = &line[..start];
    if before.ends_with("schedule function ") || before.ends_with("schedule clear ") {
        return RefKind::Schedule;
    }
    if !before.ends_with("function ") {
        return RefKind::Other;
    }
    let observed = before.ends_with("return run function ")
        || before.ends_with("if function ")
        || before.ends_with("unless function ")
        || before.contains(" store ");
    let multi = before.contains("$(")
        || [" as ", " at ", " on ", " summon ", " facing entity "]
            .iter()
            .any(|word| before.contains(word));
    RefKind::Call {
        with_args: line[end..].starts_with(" with "),
        observed,
        multi,
    }
}

/// Split `execute if score A mcfc matches 0 [if score B mcfc matches 0 ...] run CMD`
/// into its flag conditions and `CMD`. This is exactly the shape the backend
/// uses for control-flow guards.
fn guard_parts(line: &str) -> Option<(Vec<&str>, &str)> {
    let mut rest = line.strip_prefix("execute ")?;
    let mut conditions = Vec::new();
    loop {
        if let Some(after) = rest.strip_prefix("if score ") {
            let (holder, after) = after.split_once(' ')?;
            if holder.starts_with('@') || holder.contains("$(") {
                return None;
            }
            rest = after.strip_prefix("mcfc matches 0 ")?;
            conditions.push(holder);
        } else {
            let command = rest.strip_prefix("run ")?;
            return (!conditions.is_empty()).then_some((conditions, command));
        }
    }
}

fn join_guard(conditions: &[&str], command: &str) -> String {
    if conditions.is_empty() {
        return command.to_string();
    }
    let mut line = String::from("execute");
    for holder in conditions {
        line.push_str(" if score ");
        line.push_str(holder);
        line.push_str(" mcfc matches 0");
    }
    line.push_str(" run ");
    line.push_str(command);
    line
}

/// Which mcfc scores a line may write. `Unknown` means any of them.
enum Writes<'a> {
    Some(Vec<&'a str>),
    Unknown,
}

fn score_writes(line: &str) -> Writes<'_> {
    writes_of(line, true)
}

/// Every score `line` may write; `skip_zero` leaves out `set H mcfc 0`.
fn writes_of(line: &str, skip_zero: bool) -> Writes<'_> {
    if line.contains("scoreboard objectives remove") {
        return Writes::Unknown;
    }
    const NEEDLES: [&str; 10] = [
        "players set ",
        "players add ",
        "players remove ",
        "players reset ",
        "players operation ",
        "players enable ",
        "players random ",
        "store result score ",
        "store success score ",
        " >< ",
    ];
    let mut holders = Vec::new();
    for needle in NEEDLES {
        let mut from = 0;
        while let Some(found) = line[from..].find(needle) {
            let after = &line[from + found + needle.len()..];
            from += found + needle.len();
            let mut words = after.split_whitespace();
            let Some(holder) = words.next() else { continue };
            let objective = words.next();
            if holder == "*" || holder.contains("$(") {
                return Writes::Unknown;
            }
            if holder.starts_with('@') {
                continue;
            }
            match objective {
                Some(objective) if objective != "mcfc" && !objective.contains("$(") => continue,
                _ => {}
            }
            // Writing 0 can never make a flag nonzero.
            if skip_zero && needle == "players set " && words.next() == Some("0") {
                continue;
            }
            holders.push(holder);
        }
    }
    Writes::Some(holders)
}

/// Whether `line` may change `holder`, directly or through a function it calls.
fn may_write_score(line: &str, holder: &str) -> bool {
    line.contains("function ")
        || match writes_of(line, false) {
            Writes::Unknown => true,
            Writes::Some(holders) => holders.contains(&holder),
        }
}

/// `execute unless score H mcfc matches 0 run return 0`: past it, H is zero.
fn return_check(line: &str) -> Option<&str> {
    line.strip_prefix("execute unless score ")?
        .strip_suffix(" mcfc matches 0 run return 0")
        .filter(|holder| {
            !holder.contains(' ') && !holder.starts_with('@') && !holder.contains("$(")
        })
}

/// `scoreboard players set H mcfc 0` on its own: H is certainly zero afterwards.
fn certain_zero(line: &str) -> Option<&str> {
    line.strip_prefix("scoreboard players set ")?
        .strip_suffix(" mcfc 0")
        .filter(|holder| !holder.contains(' ') && !holder.contains("$("))
}

/// Backend temps: `$d0_f___tmp12` scores and `frames.d0.f.__tmp12` storage.
fn temp_tokens(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    for marker in ["___tmp", ".__tmp"] {
        let mut from = 0;
        while let Some(found) = line[from..].find(marker) {
            let at = from + found;
            from = at + marker.len();
            let mut start = at;
            while start > 0 && !matches!(bytes[start - 1], b' ' | b'"' | b'{' | b',') {
                start -= 1;
            }
            let mut end = from;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            let word_follows =
                end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
            if end > from && !word_follows {
                tokens.push((start, end));
            }
        }
    }
    tokens
}

/// Every `storage <id> <path>` and `storage:"<id>",path:"<path>"` in a line.
fn storage_mentions(line: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut words = line.split_whitespace();
    while let Some(word) = words.next() {
        if word.ends_with("storage")
            && let (Some(storage), Some(path)) = (words.next(), words.next())
        {
            found.push((storage.to_string(), path.to_string()));
        }
    }
    let mut from = 0;
    while let Some(at) = line[from..].find("storage:\"") {
        let rest = &line[from + at + "storage:\"".len()..];
        from += at + 1;
        if let Some((storage, rest)) = rest.split_once('"')
            && let Some(rest) = rest.strip_prefix(",path:\"")
            && let Some((path, _)) = rest.split_once('"')
        {
            found.push((storage.to_string(), path.to_string()));
        }
    }
    found
}

fn paths_overlap(a: &str, b: &str) -> bool {
    let within = |long: &str, short: &str| {
        long.strip_prefix(short)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[', '{']))
    };
    within(a, b) || within(b, a)
}

/// A line that writes a backend temp from something else.
enum TempDef {
    /// `scoreboard players operation T mcfc = S mcfc`
    ScoreCopy { temp: String, source: String },
    /// `scoreboard players set T mcfc C`
    ScoreConst { temp: String, value: i64 },
    /// `data modify storage NS T set from storage NS2 P`
    StorageCopy {
        temp: String,
        temp_storage: String,
        storage: String,
        path: String,
    },
    /// `data modify storage NS T set value V`
    StorageConst {
        temp: String,
        temp_storage: String,
        value: String,
    },
    /// `execute store success score T mcfc <one score clause>`
    Condition { temp: String, clause: String },
}

/// `text` when it is exactly one `if|unless score` clause.
fn score_clause(text: &str) -> Option<&str> {
    let words: Vec<&str> = text.split(' ').collect();
    let single = match words[..] {
        ["if" | "unless", "score", _, "mcfc", "matches", _] => true,
        ["if" | "unless", "score", _, "mcfc", op, _, "mcfc"] => {
            matches!(op, "<" | "<=" | "=" | ">=" | ">")
        }
        _ => false,
    };
    single.then_some(text)
}

/// Whether `value` is in a `matches` range such as `3`, `..-1` or `2..9`.
fn in_range(range: &str, value: i32) -> Option<bool> {
    let bound = |text: &str| -> Option<Option<i32>> {
        if text.is_empty() {
            Some(None)
        } else {
            text.parse().ok().map(Some)
        }
    };
    let (low, high) = match range.split_once("..") {
        Some((low, high)) => (bound(low)?, bound(high)?),
        None => (bound(range)?, bound(range)?),
    };
    Some(low.is_none_or(|low| low <= value) && high.is_none_or(|high| value <= high))
}

/// The outcome of a score clause (as words) when its scores are known.
fn clause_outcome(words: &[&str], known: &HashMap<String, i32>) -> Option<bool> {
    let value = |holder: &str| known.get(holder).copied();
    let holds = match *words {
        [_, "score", x, "mcfc", "matches", range] => in_range(range, value(x)?)?,
        [_, "score", x, "mcfc", op, y, "mcfc"] => {
            let (x, y) = (value(x)?, value(y)?);
            match op {
                "<" => x < y,
                "<=" => x <= y,
                "=" => x == y,
                ">=" => x >= y,
                ">" => x > y,
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(holds == (words[0] == "if"))
}

/// `line` with its score clauses decided by `known`: None when a clause
/// fails, so the line never runs.
fn decide_clauses(line: &str, known: &HashMap<String, i32>) -> Option<String> {
    let Some(rest) = line.strip_prefix("execute ") else {
        return Some(line.to_string());
    };
    // Only `execute <score clauses> run CMD`; stores and the rest stay as they are.
    let Some((prefix, run)) = rest.split_once(" run ") else {
        return Some(line.to_string());
    };
    let words: Vec<&str> = prefix.split(' ').collect();
    let mut kept = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let len = match words[at..] {
            ["if" | "unless", "score", _, "mcfc", "matches", _, ..] => 6,
            ["if" | "unless", "score", _, "mcfc", _, _, "mcfc", ..] => 7,
            _ => return Some(line.to_string()),
        };
        let clause = &words[at..at + len];
        match clause_outcome(clause, known) {
            Some(false) => return None,
            Some(true) => {}
            None => kept.push(clause.join(" ")),
        }
        at += len;
    }
    Some(if kept.is_empty() {
        run.to_string()
    } else {
        format!("execute {} run {run}", kept.join(" "))
    })
}

/// The value `scoreboard players operation A <op> B` leaves in A.
fn operate(op: &str, a: i32, b: i32) -> Option<i32> {
    Some(match op {
        "=" => b,
        "+=" => a.wrapping_add(b),
        "-=" => a.wrapping_sub(b),
        "*=" => a.wrapping_mul(b),
        "<" => a.min(b),
        ">" => a.max(b),
        _ => return None,
    })
}

/// The score holders a clause reads.
fn clause_holders(clause: &str) -> Vec<String> {
    let words: Vec<&str> = clause.split(' ').collect();
    let mut holders = vec![words[2].to_string()];
    if words.len() == 7 {
        holders.push(words[5].to_string());
    }
    holders
}

fn negate_clause(clause: &str) -> String {
    match clause.strip_prefix("if ") {
        Some(rest) => format!("unless {rest}"),
        None => format!("if {}", clause.strip_prefix("unless ").unwrap_or(clause)),
    }
}

impl TempDef {
    fn parse(line: &str) -> Option<Self> {
        let is_temp = |token: &str| temp_tokens(token) == [(0, token.len())];
        if let Some(rest) = line.strip_prefix("scoreboard players operation ") {
            let [temp, "mcfc", "=", source, "mcfc"] = rest.split(' ').collect::<Vec<_>>()[..]
            else {
                return None;
            };
            return (is_temp(temp) && !source.starts_with('@')).then(|| Self::ScoreCopy {
                temp: temp.to_string(),
                source: source.to_string(),
            });
        }
        if let Some(rest) = line.strip_prefix("scoreboard players set ") {
            let [temp, "mcfc", value] = rest.split(' ').collect::<Vec<_>>()[..] else {
                return None;
            };
            return (is_temp(temp) && !temp.contains('.')).then_some(Self::ScoreConst {
                temp: temp.to_string(),
                value: value.parse().ok()?,
            });
        }
        if let Some(rest) = line.strip_prefix("execute store success score ") {
            let (temp, clause) = rest.split_once(" mcfc ")?;
            let clause = score_clause(clause)?;
            return (is_temp(temp) && !temp.contains('.')).then(|| Self::Condition {
                temp: temp.to_string(),
                clause: clause.to_string(),
            });
        }
        let rest = line.strip_prefix("data modify storage ")?;
        let (temp_storage, rest) = rest.split_once(' ')?;
        let (temp, rest) = rest.split_once(' ')?;
        if !is_temp(temp) || !temp.contains('.') {
            return None;
        }
        if let Some(value) = rest.strip_prefix("set value ") {
            return Some(Self::StorageConst {
                temp: temp.to_string(),
                temp_storage: temp_storage.to_string(),
                value: value.to_string(),
            });
        }
        let rest = rest.strip_prefix("set from storage ")?;
        let (storage, path) = rest.split_once(' ')?;
        (!path.contains(' ') && !path.contains("$(")).then(|| Self::StorageCopy {
            temp: temp.to_string(),
            temp_storage: temp_storage.to_string(),
            storage: storage.to_string(),
            path: path.to_string(),
        })
    }

    fn temp(&self) -> &str {
        match self {
            Self::ScoreCopy { temp, .. }
            | Self::ScoreConst { temp, .. }
            | Self::StorageCopy { temp, .. }
            | Self::StorageConst { temp, .. }
            | Self::Condition { temp, .. } => temp,
        }
    }

    /// `line` (the temp's only read) with the temp replaced by its source, or
    /// None when that is not a plain read this can rewrite.
    fn substitute(&self, line: &str) -> Option<String> {
        match self {
            Self::ScoreCopy { temp, source } => match score_writes(line) {
                Writes::Some(holders) if !holders.contains(&temp.as_str()) => {
                    Some(replace_token(line, temp, source))
                }
                _ => None,
            },
            Self::ScoreConst { temp, value } => fold_score_const(line, temp, *value),
            Self::Condition { temp, clause } => {
                // Only a test of the flag itself, not a write or a macro read.
                if !line.starts_with("execute ")
                    || !matches!(score_writes(line), Writes::Some(ref w) if !w.contains(&temp.as_str()))
                {
                    return None;
                }
                for (test, negate) in [("if", false), ("unless", true)] {
                    for (value, flip) in [("1", false), ("0", true)] {
                        let needle = format!(" {test} score {temp} mcfc matches {value}");
                        if let Some(at) = line.find(&needle) {
                            let after = &line[at + needle.len()..];
                            if !after.is_empty() && !after.starts_with(' ') {
                                continue;
                            }
                            let clause = if negate != flip {
                                negate_clause(clause)
                            } else {
                                clause.clone()
                            };
                            return Some(format!("{} {clause}{after}", &line[..at]));
                        }
                    }
                }
                None
            }
            Self::StorageCopy {
                temp,
                temp_storage,
                storage,
                path,
            } => {
                for prefix in [
                    "from storage",
                    "with storage",
                    "data get storage",
                    "if data storage",
                    "unless data storage",
                ] {
                    let needle = format!("{prefix} {temp_storage} {temp}");
                    if let Some(at) = line.find(&needle) {
                        let after = &line[at + needle.len()..];
                        if after.is_empty() || after.starts_with([' ', '.', '[']) {
                            return Some(format!(
                                "{}{prefix} {storage} {path}{after}",
                                &line[..at]
                            ));
                        }
                    }
                }
                let needle = format!("storage:\"{temp_storage}\",path:\"{temp}");
                let at = line.find(&needle)?;
                let after = &line[at + needle.len()..];
                after
                    .starts_with(['"', '.', '['])
                    .then(|| format!("{}storage:\"{storage}\",path:\"{path}{after}", &line[..at]))
            }
            Self::StorageConst {
                temp,
                temp_storage,
                value,
            } => {
                for (from, to) in [
                    ("set from", "set value"),
                    ("append from", "append value"),
                    ("prepend from", "prepend value"),
                ] {
                    let needle = format!(" {from} storage {temp_storage} {temp}");
                    if let Some(head) = line.strip_suffix(&needle) {
                        return Some(format!("{head} {to} {value}"));
                    }
                }
                None
            }
        }
    }
}

fn replace_token(line: &str, token: &str, with: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut last = 0;
    for (start, end) in temp_tokens(line) {
        if &line[start..end] == token {
            out.push_str(&line[last..start]);
            out.push_str(with);
            last = end;
        }
    }
    out.push_str(&line[last..]);
    out
}

/// Fold a constant score into the one command that reads it.
fn fold_score_const(line: &str, temp: &str, value: i64) -> Option<String> {
    if let Some(head) = line.strip_suffix(&format!(" int 1 run scoreboard players get {temp} mcfc"))
    {
        let target = head.strip_prefix("execute store result storage ")?;
        let (storage, path) = target.split_once(' ')?;
        if path.contains(' ') {
            return None;
        }
        return Some(format!(
            "data modify storage {storage} {path} set value {value}"
        ));
    }
    let tail = format!(" {temp} mcfc");
    if let Some(head) = line.strip_suffix(&tail) {
        // `... operation Y mcfc <op> T mcfc`
        let (head, op) = head.rsplit_once(' ')?;
        let (head, target) = head.strip_suffix(" mcfc")?.rsplit_once(' ')?;
        let head = head.strip_suffix("scoreboard players operation")?;
        let command = match (op, value >= 0) {
            ("=", _) => format!("set {target} mcfc {value}"),
            ("+=", true) | ("-=", false) => format!("add {target} mcfc {}", value.abs()),
            ("-=", true) | ("+=", false) => format!("remove {target} mcfc {}", value.abs()),
            _ => return None,
        };
        return Some(format!("{head}scoreboard players {command}"));
    }
    // `if|unless score X mcfc <op> T mcfc` or with T on the left.
    for keyword in ["if score ", "unless score "] {
        let mut from = 0;
        while let Some(at) = line[from..].find(keyword) {
            let start = from + at + keyword.len();
            from = start;
            let words: Vec<&str> = line[start..].splitn(6, ' ').collect();
            let (holder, op, flipped) = match words[..] {
                [left, "mcfc", op, right, "mcfc", ..] if right == temp => (left, op, false),
                [left, "mcfc", op, right, "mcfc", ..] if left == temp => (right, op, true),
                _ => continue,
            };
            let op = match (op, flipped) {
                (op, false) => op,
                ("<", true) => ">",
                ("<=", true) => ">=",
                (">", true) => "<",
                (">=", true) => "<=",
                (op, true) => op,
            };
            let in_range =
                |bound: i64| i64::from(i32::MIN) <= bound && bound <= i64::from(i32::MAX);
            let range = match op {
                "=" => format!("{value}"),
                "<" if in_range(value - 1) => format!("..{}", value - 1),
                "<=" => format!("..{value}"),
                ">" if in_range(value + 1) => format!("{}..", value + 1),
                ">=" => format!("{value}.."),
                _ => return None,
            };
            let consumed = [words[0], words[1], words[2], words[3], words[4]].join(" ");
            return Some(format!(
                "{}{holder} mcfc matches {range}{}",
                &line[..start],
                &line[start + consumed.len()..]
            ));
        }
    }
    None
}

/// Backend frame scores: `$d<depth>_...`.
fn frame_score_tokens(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut from = 0;
    while let Some(found) = line[from..].find("$d") {
        let start = from + found;
        from = start + 2;
        let boundary = start == 0 || matches!(bytes[start - 1], b' ' | b'"');
        if !boundary || !bytes.get(start + 2).is_some_and(u8::is_ascii_digit) {
            continue;
        }
        let mut end = start + 2;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        tokens.push((start, end));
        from = end;
    }
    tokens
}

fn is_score_write_position(before: &str) -> bool {
    [
        "players set ",
        "players add ",
        "players remove ",
        "players reset ",
        "players operation ",
        "store result score ",
        "store success score ",
    ]
    .iter()
    .any(|needle| before.ends_with(needle))
}

/// A macro line that builds an mcfc score holder's name at run time.
fn dynamic_mcfc_holder(line: &str) -> bool {
    line.starts_with('$')
        && line.match_indices(") mcfc").any(|(at, _)| {
            line[..at]
                .rfind("$(")
                .is_some_and(|open| !line[open..at].contains(' '))
        })
}

/// `line` with its writes to `dead` scores removed: `None` when it has none,
/// `Some(None)` when nothing of the line is left.
fn without_dead_write(line: &str, dead: &HashSet<String>) -> Option<Option<String>> {
    // `execute ... store (result|success) score D mcfc ...`
    for kind in ["store result score ", "store success score "] {
        let Some(at) = line.find(kind) else { continue };
        let (holder, after) = line[at + kind.len()..].split_once(' ')?;
        let Some(after) = after.strip_prefix("mcfc ") else {
            continue;
        };
        if !line.starts_with("execute ") || !dead.contains(holder) {
            continue;
        }
        let chain = format!("{}{after}", &line[..at]);
        return Some(match chain.strip_prefix("execute run ") {
            Some(command) => Some(command.to_string()),
            None if !chain.contains(" run ") && pure_conditions(&chain) => None,
            None => Some(chain),
        });
    }
    // The final command writes a dead score.
    let (prefix, command) = match line.rfind(" run ") {
        Some(at) if line.starts_with("execute ") => (&line[..at + 5], &line[at + 5..]),
        _ => ("", line),
    };
    let (prefix, returns) = match (
        prefix.strip_suffix("return run "),
        command.strip_prefix("return run "),
    ) {
        (Some(before), _) => (before, true),
        (None, Some(_)) => (prefix, true),
        (None, None) => (prefix, false),
    };
    let command = command.strip_prefix("return run ").unwrap_or(command);
    let rest = command.strip_prefix("scoreboard players ")?;
    let words: Vec<&str> = rest.split(' ').collect();
    let (verb, holder) = (*words.first()?, *words.get(1)?);
    let writes = matches!(verb, "set" | "add" | "remove" | "reset")
        || verb == "operation" && words.get(3) != Some(&"><");
    if !writes || !dead.contains(holder) || !pure_conditions(prefix) {
        return None;
    }
    if returns {
        // Keep the exit; `set` returned the value it wrote.
        let value = words.get(3).filter(|_| verb == "set")?;
        return Some(Some(format!("{prefix}return {value}")));
    }
    Some(None)
}

/// An `execute` prefix whose subcommands have no side effects.
fn pure_conditions(prefix: &str) -> bool {
    !["store ", "summon ", "if function ", "unless function "]
        .iter()
        .any(|word| prefix.contains(word))
}

#[derive(Clone, PartialEq, Eq)]
struct Bits(Vec<u64>);

impl Bits {
    fn empty(len: usize) -> Self {
        Self(vec![0; len.div_ceil(64)])
    }
    fn full(len: usize) -> Self {
        let mut bits = Self(vec![u64::MAX; len.div_ceil(64)]);
        if !len.is_multiple_of(64) {
            *bits.0.last_mut().unwrap() = (1u64 << (len % 64)) - 1;
        }
        bits
    }
    fn get(&self, index: usize) -> bool {
        self.0[index / 64] & (1 << (index % 64)) != 0
    }
    fn set(&mut self, index: usize) {
        self.0[index / 64] |= 1 << (index % 64);
    }
    fn clear(&mut self, index: usize) {
        self.0[index / 64] &= !(1 << (index % 64));
    }
    /// `self |= other`; true when that changed anything.
    fn union(&mut self, other: &Bits) -> bool {
        let mut changed = false;
        for (word, other) in self.0.iter_mut().zip(&other.0) {
            let next = *word | other;
            changed |= next != *word;
            *word = next;
        }
        changed
    }
}

/// Analysis state between two lines of one function.
struct Walk {
    /// Flags that may be nonzero.
    after: Bits,
    /// When the previous line was guarded: its conditions, and the flags that
    /// may be nonzero if those conditions held (they still hold now).
    guarded: Option<(Vec<String>, Bits)>,
}

impl Walk {
    fn new(entry: &Bits) -> Self {
        Self {
            after: entry.clone(),
            guarded: None,
        }
    }
}

/// Guard-flag facts for one pass of the analysis.
struct Flags {
    index: HashMap<String, usize>,
    /// Flags each function (or anything it calls) may leave nonzero.
    may_write: HashMap<String, Bits>,
    /// Flags that may be nonzero when each function starts.
    entry: HashMap<String, Bits>,
}

impl Pack {
    fn load(files: &BTreeMap<String, String>) -> Self {
        let mut funcs = BTreeMap::new();
        for (path, contents) in files {
            if let Some(id) = function_id(path) {
                let lines = contents
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_string)
                    .collect();
                funcs.insert(id, lines);
            }
        }
        let mut external_refs = BTreeSet::new();
        for (path, contents) in files {
            if function_id(path).is_some() {
                continue;
            }
            for (start, end) in id_tokens(contents) {
                let id = &contents[start..end];
                if funcs.contains_key(id) {
                    external_refs.insert(id.to_string());
                }
            }
        }
        let original = funcs.keys().cloned().collect();
        Self {
            funcs,
            external_refs,
            original,
        }
    }

    fn store(self, files: &mut BTreeMap<String, String>) {
        for id in &self.original {
            if !self.funcs.contains_key(id) {
                files.remove(&function_path(id));
            }
        }
        for (id, lines) in self.funcs {
            let mut contents = lines.join("\n");
            contents.push('\n');
            if lines.is_empty() {
                contents = String::new();
            }
            files.insert(function_path(&id), contents);
        }
    }

    fn refs(&self, line: &str) -> Vec<Ref> {
        id_tokens(line)
            .into_iter()
            .filter(|&(start, end)| self.funcs.contains_key(&line[start..end]))
            .map(|(start, end)| Ref {
                start,
                end,
                id: line[start..end].to_string(),
                kind: classify(line, start, end),
            })
            .collect()
    }

    fn is_dynamic_call(line: &str) -> bool {
        line.contains("function $(")
    }

    /// Public functions, or functions something outside the pack's own calls
    /// can start: other files, the schedule, or a stored name.
    fn entry_unknown(&self) -> BTreeSet<String> {
        let mut unknown: BTreeSet<String> = self
            .funcs
            .keys()
            .filter(|id| !id.split_once(':').unwrap().1.starts_with("generated/"))
            .cloned()
            .collect();
        unknown.extend(self.external_refs.iter().cloned());
        for lines in self.funcs.values() {
            for line in lines {
                for reference in self.refs(line) {
                    if !matches!(reference.kind, RefKind::Call { .. }) {
                        unknown.insert(reference.id);
                    }
                }
            }
        }
        unknown
    }

    /// Functions whose name must stay: public ones, and schedule targets
    /// (two merged schedule targets would replace each other's schedule).
    fn pinned(&self) -> BTreeSet<String> {
        let mut pinned: BTreeSet<String> = self
            .funcs
            .keys()
            .filter(|id| !id.split_once(':').unwrap().1.starts_with("generated/"))
            .cloned()
            .collect();
        pinned.extend(self.external_refs.iter().cloned());
        for lines in self.funcs.values() {
            for line in lines {
                for reference in self.refs(line) {
                    if reference.kind == RefKind::Schedule {
                        pinned.insert(reference.id);
                    }
                }
            }
        }
        pinned
    }

    fn analyze(&self) -> Flags {
        let mut index = HashMap::new();
        for lines in self.funcs.values() {
            for line in lines {
                let holders = guard_parts(line)
                    .map(|(conditions, _)| conditions)
                    .or_else(|| return_check(line).map(|holder| vec![holder]));
                for holder in holders.into_iter().flatten() {
                    let next = index.len();
                    index.entry(holder.to_string()).or_insert(next);
                }
            }
        }
        let count = index.len();
        let all = Bits::full(count);

        // Line-local write sets, before following calls.
        let mut direct: HashMap<&str, Bits> = HashMap::new();
        let mut callees: HashMap<&str, Vec<String>> = HashMap::new();
        for (id, lines) in &self.funcs {
            let mut bits = Bits::empty(count);
            let mut calls = Vec::new();
            for line in lines {
                if Self::is_dynamic_call(line) {
                    bits = all.clone();
                }
                match score_writes(line) {
                    Writes::Unknown => bits = all.clone(),
                    Writes::Some(holders) => {
                        for holder in holders {
                            if let Some(&flag) = index.get(holder) {
                                bits.set(flag);
                            }
                        }
                    }
                }
                for reference in self.refs(line) {
                    if matches!(reference.kind, RefKind::Call { .. }) {
                        calls.push(reference.id);
                    }
                }
            }
            direct.insert(id, bits);
            callees.insert(id, calls);
        }
        let mut may_write: HashMap<String, Bits> = direct
            .iter()
            .map(|(id, bits)| (id.to_string(), bits.clone()))
            .collect();
        loop {
            let mut changed = false;
            for (id, calls) in &callees {
                let mut bits = may_write[*id].clone();
                for callee in calls {
                    bits.union(&may_write[callee]);
                }
                if bits != may_write[*id] {
                    may_write.insert(id.to_string(), bits);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let mut flags = Flags {
            index,
            may_write,
            entry: HashMap::new(),
        };
        let unknown = self.entry_unknown();
        for id in self.funcs.keys() {
            let bits = if unknown.contains(id) {
                all.clone()
            } else {
                Bits::empty(count)
            };
            flags.entry.insert(id.clone(), bits);
        }
        let mut pending: Vec<String> = self.funcs.keys().cloned().collect();
        let mut queued: HashSet<String> = pending.iter().cloned().collect();
        while let Some(id) = pending.pop() {
            queued.remove(&id);
            let mut state = Walk::new(&flags.entry[&id]);
            for line in &self.funcs[&id] {
                let (_, calls) = self.step(&flags, line, &mut state);
                for (callee, at_call) in calls {
                    let entry = flags.entry.get_mut(&callee).unwrap();
                    if entry.union(&at_call) && queued.insert(callee.clone()) {
                        pending.push(callee);
                    }
                }
            }
        }
        flags
    }

    /// Run one line: return it with its provably-true guards dropped, and the
    /// state each call in it starts with. Updates `state` to after the line.
    fn step(&self, flags: &Flags, line: &str, state: &mut Walk) -> (String, Vec<(String, Bits)>) {
        let flag = |holder: &str| flags.index.get(holder).copied();
        let (rewritten, kept) = match guard_parts(line) {
            Some((conditions, command)) => {
                let kept: Vec<String> = conditions
                    .into_iter()
                    .filter(|holder| flag(holder).is_none_or(|index| state.after.get(index)))
                    .map(str::to_string)
                    .collect();
                let refs: Vec<&str> = kept.iter().map(String::as_str).collect();
                (join_guard(&refs, command), kept)
            }
            None => (line.to_string(), Vec::new()),
        };
        // State while the command itself runs. Consecutive lines under the same
        // guard share it, as long as no line in between writes a guard flag.
        let mut running = match &state.guarded {
            Some((conditions, bits)) if !kept.is_empty() && *conditions == kept => bits.clone(),
            _ => {
                let mut bits = state.after.clone();
                for holder in &kept {
                    if let Some(index) = flag(holder) {
                        bits.clear(index);
                    }
                }
                bits
            }
        };

        let mut writes = Bits::empty(flags.index.len());
        if Self::is_dynamic_call(&rewritten) {
            writes = Bits::full(flags.index.len());
        }
        match score_writes(&rewritten) {
            Writes::Unknown => writes = Bits::full(flags.index.len()),
            Writes::Some(holders) => {
                for holder in holders {
                    if let Some(index) = flag(holder) {
                        writes.set(index);
                    }
                }
            }
        }
        let mut calls = Vec::new();
        for reference in self.refs(&rewritten) {
            if let RefKind::Call { multi, .. } = reference.kind {
                let callee_writes = &flags.may_write[&reference.id];
                writes.union(callee_writes);
                let mut at_call = running.clone();
                if multi {
                    at_call.union(callee_writes);
                }
                calls.push((reference.id, at_call));
            }
        }

        let command = guard_parts(&rewritten).map_or(rewritten.as_str(), |(_, command)| command);
        if !command.starts_with("execute")
            && !command.starts_with('$')
            && let Some(index) = certain_zero(command).and_then(flag)
        {
            running.clear(index);
        }
        running.union(&writes);
        let touches_guard = kept
            .iter()
            .any(|holder| flag(holder).is_none_or(|index| writes.get(index)));
        if kept.is_empty() {
            state.after = running;
            state.guarded = None;
        } else {
            state.after.union(&running);
            state.guarded = (!touches_guard).then_some((kept, running));
        }
        // Past a return check, its flag is zero.
        if let Some(index) = return_check(&rewritten).and_then(flag) {
            state.after.clear(index);
            state.guarded = None;
        }
        (rewritten, calls)
    }

    /// Drop guards and early-return checks on flags known to be zero. With
    /// `fuse`, a line that sets a zero flag and is followed by the return
    /// check on it becomes one `return run` line.
    fn remove_known_guards(&mut self, fuse: bool) {
        let flags = self.analyze();
        let keep_value = self.value_observed();
        let ids: Vec<String> = self.funcs.keys().cloned().collect();
        for id in ids {
            let mut state = Walk::new(&flags.entry[&id]);
            let lines = &self.funcs[&id];
            let mut out = Vec::with_capacity(lines.len());
            let mut index = 0;
            while index < lines.len() {
                let line = &lines[index];
                let known_zero = |state: &Walk, holder: &str| {
                    flags
                        .index
                        .get(holder)
                        .is_some_and(|&flag| !state.after.get(flag))
                };
                if return_check(line).is_some_and(|holder| known_zero(&state, holder)) {
                    index += 1;
                    continue;
                }
                let next_check = lines.get(index + 1).and_then(|next| return_check(next));
                let zero_before = next_check.is_some_and(|holder| known_zero(&state, holder));
                let (rewritten, _) = self.step(&flags, line, &mut state);
                let fused = next_check
                    .filter(|_| fuse && zero_before && !keep_value.contains(&id))
                    .and_then(|holder| {
                        let prefix = rewritten
                            .strip_suffix(&format!("scoreboard players set {holder} mcfc 1"))?;
                        (prefix.is_empty()
                            || prefix.starts_with("execute ") && prefix.ends_with(" run "))
                        .then(|| {
                            (
                                holder,
                                format!(
                                    "{prefix}return run scoreboard players set {holder} mcfc 1"
                                ),
                            )
                        })
                    });
                match fused {
                    Some((holder, fused)) => {
                        out.push(fused);
                        index += 2;
                        // Falling through means the flag stayed zero.
                        if let Some(&flag) = flags.index.get(holder) {
                            state.after.clear(flag);
                            state.guarded = None;
                        }
                    }
                    None => {
                        out.push(rewritten);
                        index += 1;
                    }
                }
            }
            self.funcs.insert(id, out);
        }
    }

    /// Functions whose return value something reads, or that anything outside
    /// the pack may call: their early exits must keep returning what they did.
    fn value_observed(&self) -> BTreeSet<String> {
        let mut observed = self.pinned();
        for lines in self.funcs.values() {
            for line in lines {
                for reference in self.refs(line) {
                    if matches!(reference.kind, RefKind::Call { observed: true, .. }) {
                        observed.insert(reference.id);
                    }
                }
            }
        }
        observed
    }

    /// The scores a line writes itself, or None when that is unknowable.
    fn line_writes(line: &str) -> Option<Vec<String>> {
        let macro_command = line.starts_with("$$(") || line.contains("run $(");
        if Self::is_dynamic_call(line) || line.starts_with('$') && macro_command {
            return None;
        }
        match writes_of(line, false) {
            Writes::Unknown => None,
            Writes::Some(holders) => Some(holders.into_iter().map(str::to_string).collect()),
        }
    }

    /// Every score a function may write, including through its calls; None
    /// when that is unknowable.
    fn score_summaries(&self) -> HashMap<String, Option<HashSet<String>>> {
        let mut summaries: HashMap<String, Option<HashSet<String>>> = self
            .funcs
            .keys()
            .map(|id| (id.clone(), Some(HashSet::new())))
            .collect();
        loop {
            let mut changed = false;
            for (id, lines) in &self.funcs {
                let summary = lines.iter().try_fold(HashSet::new(), |mut acc, line| {
                    acc.extend(Self::line_writes(line)?);
                    for reference in self.refs(line) {
                        if matches!(reference.kind, RefKind::Call { .. }) {
                            acc.extend(summaries[&reference.id].clone()?);
                        }
                    }
                    Some(acc)
                });
                if summaries[id] != summary {
                    summaries.insert(id.clone(), summary);
                    changed = true;
                }
            }
            if !changed {
                return summaries;
            }
        }
    }

    fn call_counts(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for lines in self.funcs.values() {
            for line in lines {
                for reference in self.refs(line) {
                    *counts.entry(reference.id).or_insert(0) += 1;
                }
            }
        }
        counts
    }

    fn inlinable(lines: &[String]) -> bool {
        lines.iter().all(|line| {
            !line.starts_with('$') && !line.split_whitespace().any(|word| word == "return")
        })
    }

    /// Inline until nothing changes; true when anything was inlined.
    fn inline_rounds(&mut self, copy_small: bool) -> bool {
        let mut inlined = false;
        for _ in 0..8 {
            // Inlined functions linger until dropped, and would skew use counts.
            self.remove_unreachable();
            self.remove_known_guards(false);
            self.fold_constants();
            if !self.inline_calls(copy_small) {
                break;
            }
            inlined = true;
        }
        self.remove_known_guards(false);
        self.remove_unreachable();
        inlined
    }

    fn simplify(&mut self) {
        while self.fold_constants()
            | self.fuse_bool_sets()
            | self.propagate_temps()
            | self.fold_accumulates()
            | self.remove_dead_scores()
        {}
    }

    /// Returns true when anything changed.
    fn inline_calls(&mut self, copy_small: bool) -> bool {
        let pinned = self.pinned();
        let unknown = self.entry_unknown();
        let counts = self.call_counts();
        let mut changed = false;
        let ids: Vec<String> = self.funcs.keys().cloned().collect();
        for id in ids {
            let lines = self.funcs[&id].clone();
            let mut out = Vec::with_capacity(lines.len());
            for line in lines {
                let refs = self.refs(&line);
                let call = match refs.as_slice() {
                    [only] => only,
                    _ => {
                        out.push(line);
                        continue;
                    }
                };
                let RefKind::Call {
                    with_args: false,
                    observed: false,
                    ..
                } = call.kind
                else {
                    out.push(line);
                    continue;
                };
                if call.id == id {
                    out.push(line);
                    continue;
                }
                let body = &self.funcs[&call.id];
                if !Self::inlinable(body) {
                    out.push(line);
                    continue;
                }
                let bare = call.start == "function ".len() && call.end == line.len();
                let prefix = &line[..call.start - "function ".len()];
                match body.len() {
                    0 if bare
                        || guard_parts(&line)
                            .is_some_and(|(_, cmd)| cmd.starts_with("function ")) =>
                    {
                        changed = true;
                    }
                    1 => {
                        out.push(format!("{prefix}{}", body[0]));
                        changed = true;
                    }
                    len if bare
                        && (copy_small && len <= INLINE_MAX_LINES
                            || counts.get(&call.id) == Some(&1)
                                && !pinned.contains(&call.id)
                                && !unknown.contains(&call.id)) =>
                    {
                        out.extend(body.iter().cloned());
                        changed = true;
                    }
                    _ => out.push(line),
                }
            }
            self.funcs.insert(id, out);
        }
        changed
    }

    fn temp_mentions(&self) -> HashMap<String, usize> {
        let mut mentions: HashMap<String, usize> = HashMap::new();
        for (id, lines) in &self.funcs {
            if id.ends_with(":generated/setup") {
                continue;
            }
            for line in lines {
                for (start, end) in temp_tokens(line) {
                    *mentions.entry(line[start..end].to_string()).or_insert(0) += 1;
                }
            }
        }
        mentions
    }

    /// Substitute single-use backend temps. Returns true when anything changed.
    fn propagate_temps(&mut self) -> bool {
        let mentions = self.temp_mentions();
        let mut changed = false;
        let ids: Vec<String> = self.funcs.keys().cloned().collect();
        for id in ids {
            let mut lines = self.funcs[&id].clone();
            lines.retain(|line| {
                let command = guard_parts(line).map_or(line.as_str(), |(_, command)| command);
                let self_copy = command
                    .strip_prefix("scoreboard players operation ")
                    .and_then(|rest| rest.split_once(" mcfc = "))
                    .is_some_and(|(target, source)| source.strip_suffix(" mcfc") == Some(target));
                changed |= self_copy;
                !self_copy
            });
            let mut index = 0;
            while index < lines.len() {
                // A guarded definition only pairs with a read under the same guard.
                let (conditions, command) =
                    guard_parts(&lines[index]).unwrap_or((Vec::new(), lines[index].as_str()));
                let conditions: Vec<String> = conditions.into_iter().map(str::to_string).collect();
                let Some(def) =
                    TempDef::parse(command).filter(|def| mentions.get(def.temp()) == Some(&2))
                else {
                    index += 1;
                    continue;
                };
                let Some(offset) = lines[index + 1..].iter().position(|line| {
                    temp_tokens(line)
                        .iter()
                        .any(|&(s, e)| &line[s..e] == def.temp())
                }) else {
                    index += 1;
                    continue;
                };
                let use_index = index + 1 + offset;
                let clear = lines[index + 1..use_index].iter().all(|line| {
                    self.keeps_source(&def, line)
                        && (conditions.is_empty() || self.keeps_scores(&conditions, line))
                });
                let rewritten = if conditions.is_empty() {
                    def.substitute(&lines[use_index])
                } else {
                    guard_parts(&lines[use_index])
                        .filter(|(use_conditions, _)| *use_conditions == conditions)
                        .and_then(|(use_conditions, use_command)| {
                            Some(join_guard(&use_conditions, &def.substitute(use_command)?))
                        })
                };
                match rewritten.filter(|_| clear) {
                    Some(rewritten) => {
                        lines[use_index] = rewritten;
                        lines.remove(index);
                        changed = true;
                    }
                    None => index += 1,
                }
            }
            self.funcs.insert(id, lines);
        }
        changed
    }

    /// Track scores set to constants through each function and decide the
    /// conditions and operations that read them.
    fn fold_constants(&mut self) -> bool {
        let summaries = self.score_summaries();
        let mut changed = false;
        let ids: Vec<String> = self.funcs.keys().cloned().collect();
        for id in ids {
            let mut known: HashMap<String, i32> = HashMap::new();
            let mut out = Vec::new();
            for line in &self.funcs[&id] {
                let Some(mut line) = decide_clauses(line, &known) else {
                    continue;
                };
                // A condition-only store of a decided clause is a constant.
                if let Some(rest) = line.strip_prefix("execute store ") {
                    let words: Vec<&str> = rest.split(' ').collect();
                    if let ["success" | "result", "score", holder, "mcfc", clause @ ..] = &words[..]
                    {
                        let decided = score_clause(&clause.join(" "))
                            .and_then(|_| clause_outcome(clause, &known));
                        if let Some(holds) = decided {
                            line = format!(
                                "scoreboard players set {holder} mcfc {}",
                                i32::from(holds)
                            );
                        }
                    }
                }
                if let Some(rest) = line.strip_prefix("scoreboard players operation ")
                    && let [a, "mcfc", op, b, "mcfc"] = rest.split(' ').collect::<Vec<_>>()[..]
                {
                    let (x, y) = (known.get(a).copied(), known.get(b).copied());
                    if let Some(value) = x.zip(y).and_then(|(x, y)| operate(op, x, y)) {
                        line = format!("scoreboard players set {a} mcfc {value}");
                    } else if let Some(y) = y.filter(|_| a != b)
                        && let Some(folded) = fold_score_const(&line, b, i64::from(y))
                    {
                        line = folded;
                    }
                }
                if let Some(head) = line.strip_suffix(" mcfc") {
                    let holder = head.rsplit(' ').next().unwrap_or_default();
                    if let Some(&value) = known
                        .get(holder)
                        .filter(|_| head.contains(" run scoreboard players get "))
                        && let Some(folded) = fold_score_const(&line, holder, i64::from(value))
                    {
                        line = folded;
                    }
                }
                // What stays known after the line.
                let written =
                    self.refs(&line)
                        .iter()
                        .try_fold(Self::line_writes(&line), |acc, reference| {
                            let mut acc = acc?;
                            if matches!(reference.kind, RefKind::Call { .. }) {
                                acc.extend(summaries.get(&reference.id)?.clone()?);
                            }
                            Some(Some(acc))
                        });
                match written.flatten() {
                    None => known.clear(),
                    Some(holders) => {
                        let words: Vec<&str> = line.split(' ').collect();
                        let update = match words[..] {
                            ["scoreboard", "players", "set", holder, "mcfc", value] => {
                                value.parse().ok().map(|value| (holder, value))
                            }
                            [
                                "scoreboard",
                                "players",
                                verb @ ("add" | "remove"),
                                holder,
                                "mcfc",
                                value,
                            ] => value.parse::<i32>().ok().zip(known.get(holder)).map(
                                |(value, &old)| {
                                    let new = if verb == "add" {
                                        old.wrapping_add(value)
                                    } else {
                                        old.wrapping_sub(value)
                                    };
                                    (holder, new)
                                },
                            ),
                            _ => None,
                        };
                        for holder in holders {
                            known.remove(&holder);
                        }
                        // Selector holders name a different entity per context.
                        if let Some((holder, value)) =
                            update.filter(|(holder, _)| !holder.starts_with(['@', '*']))
                        {
                            known.insert(holder.to_string(), value);
                        }
                    }
                }
                out.push(line);
            }
            if out != self.funcs[&id] {
                changed = true;
                self.funcs.insert(id, out);
            }
        }
        changed
    }

    /// `set T 0` then `execute <clause> run set T 1` is one
    /// `execute store success score T mcfc <clause>`.
    fn fuse_bool_sets(&mut self) -> bool {
        let mut changed = false;
        for lines in self.funcs.values_mut() {
            let mut index = 0;
            while index + 1 < lines.len() {
                let fused =
                    lines[index]
                        .strip_prefix("scoreboard players set ")
                        .and_then(|rest| rest.strip_suffix(" mcfc 0"))
                        .filter(|holder| !holder.contains(' '))
                        .and_then(|holder| {
                            let clause = lines[index + 1].strip_prefix("execute ")?.strip_suffix(
                                &format!(" run scoreboard players set {holder} mcfc 1"),
                            )?;
                            let clause = score_clause(clause)?;
                            (!clause_holders(clause).iter().any(|h| h == holder)).then(|| {
                                format!("execute store success score {holder} mcfc {clause}")
                            })
                        });
                match fused {
                    Some(line) => {
                        lines[index] = line;
                        lines.remove(index + 1);
                        changed = true;
                    }
                    None => index += 1,
                }
            }
        }
        changed
    }

    /// `T = X`, `T op= Y`, `X = T` becomes `X op= Y` for a temp T used nowhere
    /// else, as `x = x + y` lowers to.
    fn fold_accumulates(&mut self) -> bool {
        let mentions = self.temp_mentions();
        let mut changed = false;
        let ids: Vec<String> = self.funcs.keys().cloned().collect();
        for id in ids {
            let mut lines = self.funcs[&id].clone();
            let mut index = 0;
            while index < lines.len() {
                match self.accumulate_at(&lines, index, &mentions) {
                    Some((update, rewritten, copy_back)) => {
                        lines[update] = rewritten;
                        lines.remove(copy_back);
                        lines.remove(index);
                        changed = true;
                    }
                    None => index += 1,
                }
            }
            self.funcs.insert(id, lines);
        }
        changed
    }

    /// For a `T = X` at `index`: the line to rewrite, its new text, and the
    /// copy-back line to drop.
    fn accumulate_at(
        &self,
        lines: &[String],
        index: usize,
        mentions: &HashMap<String, usize>,
    ) -> Option<(usize, String, usize)> {
        let (conditions, command) =
            guard_parts(&lines[index]).unwrap_or((Vec::new(), lines[index].as_str()));
        let TempDef::ScoreCopy { temp, source } = TempDef::parse(command)? else {
            return None;
        };
        if mentions.get(&temp) != Some(&3) {
            return None;
        }
        let mentions_word = |line: &str, word: &str| line.split([' ', '"']).any(|w| w == word);
        let mut uses = (index + 1..lines.len()).filter(|&k| mentions_word(&lines[k], &temp));
        let (update, copy_back) = (uses.next()?, uses.next()?);
        let same_guard = |k: usize| {
            guard_parts(&lines[k]).map_or(conditions.is_empty(), |(c, _)| c == conditions)
        };
        if !same_guard(update) || !same_guard(copy_back) {
            return None;
        }
        let command_of = |k: usize| guard_parts(&lines[k]).map_or(lines[k].as_str(), |(_, c)| c);
        let rest = command_of(update)
            .strip_prefix(&format!("scoreboard players operation {temp} mcfc "))?;
        let (op, operand) = rest.split_once(' ')?;
        let operand = operand.strip_suffix(" mcfc")?;
        if !matches!(op, "+=" | "-=" | "*=" | "/=" | "%=" | "<" | ">") || operand == temp {
            return None;
        }
        if command_of(copy_back)
            != format!("scoreboard players operation {source} mcfc = {temp} mcfc")
        {
            return None;
        }
        let mut guarded: Vec<String> = conditions.iter().map(|c| c.to_string()).collect();
        guarded.push(source.clone());
        let clear_before = lines[index + 1..update]
            .iter()
            .all(|line| self.keeps_scores(&guarded, line));
        let clear_after = lines[update + 1..copy_back]
            .iter()
            .all(|line| self.keeps_scores(&guarded, line) && !mentions_word(line, &source));
        (clear_before && clear_after).then(|| {
            (
                update,
                join_guard(
                    &conditions,
                    &format!("scoreboard players operation {source} mcfc {op} {operand} mcfc"),
                ),
                copy_back,
            )
        })
    }

    /// A constant operand of `*=`, `/=`, `%=`, `<` or `>` needs a score. Read it
    /// from one set at load instead of setting a temp each time.
    fn pool_constants(&mut self) {
        const OBJECTIVE: &str = "scoreboard objectives add mcfc dummy";
        // `main` first: once setup is inlined into it, the old setup file
        // lingers until unreachable functions are dropped.
        let Some(init) = [":main", ":generated/setup"]
            .iter()
            .find_map(|suffix| {
                self.funcs.iter().find(|(id, lines)| {
                    id.ends_with(suffix) && lines.iter().any(|line| line == OBJECTIVE)
                })
            })
            .map(|(id, _)| id.clone())
        else {
            return;
        };
        let mentions = self.temp_mentions();
        let mut constants = BTreeSet::new();
        for lines in self.funcs.values_mut() {
            let mut index = 0;
            while index < lines.len() {
                let command = guard_parts(&lines[index]).map_or(lines[index].as_str(), |(_, c)| c);
                let Some(TempDef::ScoreConst { temp, value }) = TempDef::parse(command) else {
                    index += 1;
                    continue;
                };
                let pooled = format!("#mcfc_const{value}");
                let rewritten = (index + 1..lines.len())
                    .find(|&k| lines[k].split(' ').any(|word| word == temp))
                    .filter(|_| mentions.get(&temp) == Some(&2))
                    .and_then(|k| {
                        let (head, rest) = lines[k].split_once("scoreboard players operation ")?;
                        let words: Vec<&str> = rest.split(' ').collect();
                        match words[..] {
                            [target, "mcfc", op, operand, "mcfc"]
                                if operand == temp
                                    && target != temp
                                    && matches!(op, "*=" | "/=" | "%=" | "<" | ">") =>
                            {
                                Some((
                                    k,
                                    format!(
                                        "{head}scoreboard players operation {target} mcfc {op} {pooled} mcfc"
                                    ),
                                ))
                            }
                            _ => None,
                        }
                    });
                match rewritten {
                    Some((k, line)) => {
                        lines[k] = line;
                        lines.remove(index);
                        constants.insert(value);
                    }
                    None => index += 1,
                }
            }
        }
        let lines = self.funcs.get_mut(&init).unwrap();
        let at = lines.iter().position(|line| line == OBJECTIVE).unwrap() + 1;
        for (offset, value) in constants.into_iter().enumerate() {
            lines.insert(
                at + offset,
                format!("scoreboard players set #mcfc_const{value} mcfc {value}"),
            );
        }
    }

    /// Drop writes to backend scores (`$d...`) that nothing reads.
    fn remove_dead_scores(&mut self) -> bool {
        let mut read = HashSet::new();
        let mut written = HashSet::new();
        for lines in self.funcs.values() {
            for line in lines {
                if dynamic_mcfc_holder(line) {
                    return false;
                }
                for (start, end) in frame_score_tokens(line) {
                    let holder = line[start..end].to_string();
                    if is_score_write_position(&line[..start]) {
                        written.insert(holder);
                    } else {
                        read.insert(holder);
                    }
                }
            }
        }
        let dead: HashSet<String> = written.difference(&read).cloned().collect();
        if dead.is_empty() {
            return false;
        }
        let mut changed = false;
        for lines in self.funcs.values_mut() {
            let mut out = Vec::with_capacity(lines.len());
            for line in lines.drain(..) {
                match without_dead_write(&line, &dead) {
                    Some(Some(rewritten)) => {
                        changed = true;
                        out.push(rewritten);
                    }
                    Some(None) => changed = true,
                    None => out.push(line),
                }
            }
            *lines = out;
        }
        changed
    }

    /// True when `line` cannot change any of these scores.
    fn keeps_scores(&self, holders: &[String], line: &str) -> bool {
        if line.starts_with('$')
            || Self::is_dynamic_call(line)
            || self
                .refs(line)
                .iter()
                .any(|reference| matches!(reference.kind, RefKind::Call { .. }))
        {
            return false;
        }
        match writes_of(line, false) {
            Writes::Unknown => false,
            Writes::Some(written) => !written
                .iter()
                .any(|holder| holders.iter().any(|h| h == holder)),
        }
    }

    /// True when `line` cannot change the value `def` copies.
    fn keeps_source(&self, def: &TempDef, line: &str) -> bool {
        match def {
            TempDef::ScoreConst { .. } | TempDef::StorageConst { .. } => true,
            TempDef::Condition { clause, .. } => self.keeps_scores(&clause_holders(clause), line),
            _ if line.starts_with('$')
                || Self::is_dynamic_call(line)
                || self
                    .refs(line)
                    .iter()
                    .any(|reference| matches!(reference.kind, RefKind::Call { .. })) =>
            {
                false
            }
            // Zero writes count here: `t = x; x = 0; x -= t` must keep `t`.
            TempDef::ScoreCopy { source, .. } => match writes_of(line, false) {
                Writes::Unknown => false,
                Writes::Some(holders) => !holders.contains(&source.as_str()),
            },
            TempDef::StorageCopy { storage, path, .. } => {
                storage_mentions(line)
                    .iter()
                    .all(|(other_storage, other_path)| {
                        other_storage != storage || !paths_overlap(other_path, path)
                    })
            }
        }
    }

    fn insert_early_returns(&mut self) {
        let observed = self.value_observed();
        for (id, lines) in self.funcs.iter_mut() {
            if observed.contains(id) {
                continue;
            }
            let conditions: Vec<Vec<String>> = lines
                .iter()
                .map(|line| {
                    guard_parts(line)
                        .map(|(holders, _)| holders.into_iter().map(str::to_string).collect())
                        .unwrap_or_default()
                })
                .collect();
            // For each flag guarding the last line, where its run of guarded lines starts.
            let mut starts: Vec<(usize, String)> = Vec::new();
            if let Some(last) = conditions.last() {
                for holder in last {
                    let start = conditions
                        .iter()
                        .rposition(|holders| !holders.contains(holder))
                        .map_or(0, |index| index + 1);
                    if lines.len() - start >= EARLY_RETURN_MIN_LINES {
                        starts.push((start, holder.clone()));
                    }
                }
            }
            if starts.is_empty() {
                continue;
            }
            let mut out = Vec::with_capacity(lines.len() + starts.len());
            for (index, line) in lines.iter().enumerate() {
                for (start, holder) in &starts {
                    // The run's lines were each guarded, so a line that can set
                    // the flag (such as a call to a returning block) needs the
                    // check again after it.
                    if *start == index
                        || (*start < index && may_write_score(&lines[index - 1], holder))
                    {
                        out.push(format!(
                            "execute unless score {holder} mcfc matches 0 run return 0"
                        ));
                    }
                }
                match guard_parts(line) {
                    Some((holders, command)) => {
                        let kept: Vec<&str> = holders
                            .into_iter()
                            .filter(|holder| {
                                !starts
                                    .iter()
                                    .any(|(start, h)| h == holder && *start <= index)
                            })
                            .collect();
                        out.push(join_guard(&kept, command));
                    }
                    None => out.push(line.clone()),
                }
            }
            *lines = out;
        }
    }

    fn flatten_execute_chains(&mut self) {
        for lines in self.funcs.values_mut() {
            for line in lines.iter_mut() {
                if !line.starts_with("execute ") && !line.starts_with("$execute ") {
                    continue;
                }
                while let Some(run) = line.find(" run ") {
                    if !line[run + 5..].starts_with("execute ") {
                        break;
                    }
                    line.replace_range(run..run + " run execute ".len(), " ");
                }
            }
        }
    }

    fn rename_refs(&mut self, renames: &HashMap<String, String>) {
        let ids: Vec<String> = self.funcs.keys().cloned().collect();
        for id in ids {
            let lines: Vec<String> = self.funcs[&id]
                .iter()
                .map(|line| {
                    let mut out = String::with_capacity(line.len());
                    let mut last = 0;
                    for reference in self.refs(line) {
                        if let Some(target) = renames.get(&reference.id) {
                            out.push_str(&line[last..reference.start]);
                            out.push_str(target);
                            last = reference.end;
                        }
                    }
                    out.push_str(&line[last..]);
                    out
                })
                .collect();
            self.funcs.insert(id, lines);
        }
    }

    fn merge_identical(&mut self) {
        loop {
            let pinned = self.pinned();
            let mut canonical: HashMap<&[String], &String> = HashMap::new();
            let mut renames = HashMap::new();
            for (id, lines) in &self.funcs {
                if pinned.contains(id) {
                    continue;
                }
                // Ids iterate in order, so the first seen is the smallest.
                match canonical.get(lines.as_slice()) {
                    Some(keep) => {
                        renames.insert(id.clone(), (*keep).clone());
                    }
                    None => {
                        canonical.insert(lines, id);
                    }
                }
            }
            if renames.is_empty() {
                return;
            }
            self.rename_refs(&renames);
            for id in renames.keys() {
                self.funcs.remove(id);
            }
        }
    }

    fn remove_unreachable(&mut self) {
        let mut reachable: BTreeSet<String> = self
            .funcs
            .keys()
            .filter(|id| !id.split_once(':').unwrap().1.starts_with("generated/"))
            .cloned()
            .collect();
        reachable.extend(
            self.external_refs
                .iter()
                .filter(|id| self.funcs.contains_key(*id))
                .cloned(),
        );
        let mut pending: Vec<String> = reachable.iter().cloned().collect();
        while let Some(id) = pending.pop() {
            for line in &self.funcs[&id] {
                for reference in self.refs(line) {
                    if reachable.insert(reference.id.clone()) {
                        pending.push(reference.id);
                    }
                }
            }
        }
        self.funcs.retain(|id, _| reachable.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(files: &[(&str, &str)]) -> BTreeMap<String, String> {
        files
            .iter()
            .map(|(name, body)| {
                (
                    format!("data/t/function/{name}.mcfunction"),
                    body.to_string(),
                )
            })
            .collect()
    }

    fn body<'a>(files: &'a BTreeMap<String, String>, name: &str) -> &'a str {
        files
            .get(&format!("data/t/function/{name}.mcfunction"))
            .map(String::as_str)
            .unwrap_or("<missing>")
    }

    #[test]
    fn drops_guards_the_caller_proves_and_keeps_the_rest() {
        let mut files = pack(&[
            (
                "main",
                "scoreboard players set $c mcfc 0\nfunction t:generated/f\n\
                 scoreboard players set $c mcfc 0\nfunction t:generated/f\n",
            ),
            (
                "generated/f",
                "execute if score $c mcfc matches 0 run say a\n\
                 execute if score $c mcfc matches 0 run execute if score $x mcfc matches 1 run function t:generated/g\n\
                 execute if score $c mcfc matches 0 run say b\n\
                 execute if score $c mcfc matches 0 run say c\n\
                 execute if score $c mcfc matches 0 run say d\n",
            ),
            ("generated/g", "say g\nscoreboard players set $c mcfc 1\n"),
        ]);
        optimize(&mut files);
        assert_eq!(
            body(&files, "generated/f"),
            "say a\n\
             execute if score $x mcfc matches 1 run function t:generated/g\n\
             execute unless score $c mcfc matches 0 run return 0\n\
             say b\nsay c\nsay d\n"
        );
    }

    #[test]
    fn entity_loops_keep_guards_their_body_can_trip() {
        let mut files = pack(&[
            (
                "main",
                "scoreboard players set $c mcfc 0\nfunction t:generated/f\n",
            ),
            (
                "generated/f",
                "execute if score $c mcfc matches 0 run execute as @a run function t:generated/body\n",
            ),
            (
                "generated/body",
                "execute if score $c mcfc matches 0 run say hi\n\
                 execute if score $c mcfc matches 0 run scoreboard players set $c mcfc 1\n",
            ),
        ]);
        optimize(&mut files);
        assert_eq!(
            body(&files, "generated/body"),
            "execute if score $c mcfc matches 0 run say hi\n\
             execute if score $c mcfc matches 0 run scoreboard players set $c mcfc 1\n"
        );
    }

    #[test]
    fn scheduled_and_stored_functions_start_unknown() {
        let mut files = pack(&[
            (
                "main",
                "scoreboard players set $c mcfc 0\nschedule function t:generated/later 1t\n\
                 data modify storage t:r fn set value \"t:generated/stored\"\n",
            ),
            (
                "generated/later",
                "execute if score $c mcfc matches 0 run say later\n",
            ),
            (
                "generated/stored",
                "execute if score $c mcfc matches 0 run say stored\n",
            ),
        ]);
        optimize(&mut files);
        assert!(body(&files, "generated/later").starts_with("execute if score $c"));
        assert!(body(&files, "generated/stored").starts_with("execute if score $c"));
    }

    #[test]
    fn propagates_single_use_temps() {
        let mut files = pack(&[(
            "main",
            "scoreboard players operation $d0_m___tmp1 mcfc = $d0_m_x mcfc\n\
             scoreboard players set $d0_m___tmp2 mcfc 10\n\
             execute if score $d0_m___tmp1 mcfc < $d0_m___tmp2 mcfc run say small\n\
             scoreboard players set $d0_m___tmp3 mcfc -2\n\
             scoreboard players operation $d0_m_x mcfc += $d0_m___tmp3 mcfc\n\
             data modify storage t:r frames.d0.m.__tmp4 set from storage t:r frames.d0.m.name\n\
             data modify storage t:r frames.d0.m.other set value 1\n\
             data modify storage t:r out set from storage t:r frames.d0.m.__tmp4\n\
             data modify storage t:r frames.d0.m.__tmp5 set from storage t:r frames.d0.m.name\n\
             data modify storage t:r frames.d0.m set value {}\n\
             data modify storage t:r out set from storage t:r frames.d0.m.__tmp5\n\
             scoreboard players operation $d0_m___tmp6 mcfc = $d0_m_x mcfc\n\
             scoreboard players add $d0_m_x mcfc 1\n\
             say $d0_m___tmp6 mcfc\n",
        )]);
        optimize(&mut files);
        assert_eq!(
            body(&files, "main"),
            "execute if score $d0_m_x mcfc matches ..9 run say small\n\
             scoreboard players remove $d0_m_x mcfc 2\n\
             data modify storage t:r frames.d0.m.other set value 1\n\
             data modify storage t:r out set from storage t:r frames.d0.m.name\n\
             data modify storage t:r frames.d0.m.__tmp5 set from storage t:r frames.d0.m.name\n\
             data modify storage t:r frames.d0.m set value {}\n\
             data modify storage t:r out set from storage t:r frames.d0.m.__tmp5\n\
             scoreboard players operation $d0_m___tmp6 mcfc = $d0_m_x mcfc\n\
             scoreboard players add $d0_m_x mcfc 1\n\
             say $d0_m___tmp6 mcfc\n"
        );
    }

    #[test]
    fn decides_conditions_on_known_scores() {
        let mut files = pack(&[
            (
                "main",
                "scoreboard players set $d0_m_x mcfc 50\n\
                 scoreboard players set $d0_m_lo mcfc 0\n\
                 execute if score $d0_m_x mcfc < $d0_m_lo mcfc run say never\n\
                 execute if score $d0_m_x mcfc > $d0_m_lo mcfc run say always\n\
                 function t:generated/bump\n\
                 execute store result storage t:r v int 1 run scoreboard players get $d0_m_x mcfc\n\
                 execute if score $d0_m_lo mcfc matches 0 run say bumped\n",
            ),
            (
                "generated/bump",
                "scoreboard players add $d0_m_lo mcfc 1\nsay bump\n",
            ),
        ]);
        optimize(&mut files);
        // Inlined `bump` makes `lo` 1; the scores are then dead.
        assert_eq!(
            body(&files, "main"),
            "say always\nsay bump\ndata modify storage t:r v set value 50\n"
        );
    }

    #[test]
    fn folds_a_materialized_condition_into_its_test() {
        let mut files = pack(&[(
            "main",
            "scoreboard players set $d0_m___tmp1 mcfc 0\n\
             execute if score $d0_m_c mcfc matches 0 run scoreboard players set $d0_m___tmp1 mcfc 1\n\
             execute unless score $d0_m___tmp1 mcfc matches 1 run say differ\n\
             scoreboard players set $d0_m___tmp2 mcfc 0\n\
             execute if score $d0_m_c mcfc matches 0 run scoreboard players set $d0_m___tmp2 mcfc 1\n\
             scoreboard players add $d0_m_c mcfc 1\n\
             execute if score $d0_m___tmp2 mcfc matches 1 run say kept\n",
        )]);
        optimize(&mut files);
        assert_eq!(
            body(&files, "main"),
            "execute unless score $d0_m_c mcfc matches 0 run say differ\n\
             execute store success score $d0_m___tmp2 mcfc if score $d0_m_c mcfc matches 0\n\
             scoreboard players add $d0_m_c mcfc 1\n\
             execute if score $d0_m___tmp2 mcfc matches 1 run say kept\n"
        );
    }

    #[test]
    fn fuses_a_flag_set_with_its_return_check() {
        let mut files = pack(&[
            (
                "main",
                "scoreboard players set $d0_m_b mcfc 0\nfunction t:generated/body\n\
                 execute if score $d0_m_b mcfc matches 1 run say broke\n",
            ),
            (
                "generated/body",
                "execute if score $d0_m_j mcfc matches 2 run scoreboard players set $d0_m_b mcfc 1\n\
                 execute unless score $d0_m_b mcfc matches 0 run return 0\n\
                 say a\nsay b\nfunction t:generated/body\n",
            ),
        ]);
        optimize(&mut files);
        assert_eq!(
            body(&files, "generated/body"),
            "execute if score $d0_m_j mcfc matches 2 run return run scoreboard players set $d0_m_b mcfc 1\n\
             say a\nsay b\nfunction t:generated/body\n"
        );
    }

    #[test]
    fn inlines_merges_and_drops_dead_functions() {
        let mut files = pack(&[
            (
                "main",
                "function t:generated/one\nexecute if score $x mcfc matches 1 run function t:generated/a\nfunction t:generated/b\n",
            ),
            ("generated/one", "say one\n"),
            ("generated/a", "$say $(v)\n"),
            ("generated/b", "$say $(v)\n"),
            ("generated/dead", "say dead\n"),
        ]);
        optimize(&mut files);
        assert_eq!(
            body(&files, "main"),
            "say one\nexecute if score $x mcfc matches 1 run function t:generated/a\nfunction t:generated/a\n"
        );
        assert_eq!(body(&files, "generated/b"), "<missing>");
        assert_eq!(body(&files, "generated/dead"), "<missing>");
        assert_eq!(body(&files, "generated/one"), "<missing>");
    }
}
