//! Minecraft target selectors (`@e[type=pig,limit=1]`), parsed so the compiler
//! can check them, tell players from other entities, and add arguments.

use crate::minecraft_ids::{MinecraftIdCategory, ids_for_category};
use crate::types::RefKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base {
    /// `@a`
    AllPlayers,
    /// `@e`
    Entities,
    /// `@p`
    NearestPlayer,
    /// `@r`
    RandomPlayer,
    /// `@s`
    Executor,
    /// `@n`
    NearestEntity,
    /// A player name or a UUID. Takes no arguments.
    Name(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arg {
    pub key: String,
    /// `tag=!x`
    pub negated: bool,
    /// The text after `=` (and `!`), as written.
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorSpec {
    pub base: Base,
    pub args: Vec<Arg>,
}

/// What an argument's value holds.
#[derive(Clone, Copy)]
enum Kind {
    Double,
    /// A float range that can't be negative.
    Distance,
    FloatRange,
    /// An int range that can't be negative.
    Level,
    Limit,
    GameMode,
    Sort,
    EntityType,
    Resource,
    /// An unquoted word, which may be empty: `tag=`.
    Word,
    /// A word or a quoted string.
    Text,
    Nbt,
    Scores,
    Advancements,
}

/// Every selector argument in Java Edition, what it holds, and whether it may
/// be negated with `!`.
const ARGS: &[(&str, Kind, bool)] = &[
    ("x", Kind::Double, false),
    ("y", Kind::Double, false),
    ("z", Kind::Double, false),
    ("dx", Kind::Double, false),
    ("dy", Kind::Double, false),
    ("dz", Kind::Double, false),
    ("distance", Kind::Distance, false),
    ("x_rotation", Kind::FloatRange, false),
    ("y_rotation", Kind::FloatRange, false),
    ("level", Kind::Level, false),
    ("limit", Kind::Limit, false),
    ("sort", Kind::Sort, false),
    ("gamemode", Kind::GameMode, true),
    ("type", Kind::EntityType, true),
    ("predicate", Kind::Resource, true),
    ("tag", Kind::Word, true),
    ("team", Kind::Text, true),
    ("name", Kind::Text, true),
    ("nbt", Kind::Nbt, true),
    ("scores", Kind::Scores, false),
    ("advancements", Kind::Advancements, false),
];

pub const GAME_MODES: &[&str] = &["survival", "creative", "adventure", "spectator"];
pub const SORTS: &[&str] = &["nearest", "furthest", "random", "arbitrary"];

/// Arguments that may appear any number of times.
const REPEATABLE: &[&str] = &["tag", "nbt", "predicate"];
/// Arguments that may appear once un-negated, or many times negated.
const ONE_POSITIVE: &[&str] = &["type", "team", "name", "gamemode"];

impl SelectorSpec {
    pub fn new(base: Base) -> Self {
        SelectorSpec {
            base,
            args: Vec::new(),
        }
    }

    /// Reads selector text. Arguments aren't checked; see [`SelectorSpec::check`].
    pub fn parse(text: &str) -> Result<SelectorSpec, String> {
        let text = text.trim();
        let Some(rest) = text.strip_prefix('@') else {
            if is_player_name(text) || is_uuid(text) || text.contains("$(") {
                return Ok(SelectorSpec::new(Base::Name(text.to_string())));
            }
            return Err(format!(
                "'{text}' is not a selector (such as '@e[type=minecraft:pig]'), a player name or a UUID"
            ));
        };
        let mut chars = rest.chars();
        let base = match chars.next() {
            Some('a') => Base::AllPlayers,
            Some('e') => Base::Entities,
            Some('p') => Base::NearestPlayer,
            Some('r') => Base::RandomPlayer,
            Some('s') => Base::Executor,
            Some('n') => Base::NearestEntity,
            _ => {
                return Err(format!(
                    "unknown selector '{text}'; use @a, @e, @p, @r, @s or @n"
                ));
            }
        };
        let rest = chars.as_str().trim_start();
        let mut spec = SelectorSpec::new(base);
        if rest.is_empty() {
            return Ok(spec);
        }
        let Some(inner) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) else {
            return Err(format!(
                "expected '[' after '@{}' and ']' at the end",
                &text[1..2]
            ));
        };
        if inner.trim().is_empty() {
            return Ok(spec);
        }
        for part in split_top_level(inner)? {
            let Some((key, value)) = part.split_once('=') else {
                return Err(format!(
                    "selector argument '{}' needs '=value'",
                    part.trim()
                ));
            };
            let key = key.trim();
            let value = value.trim();
            let (negated, value) = match value.strip_prefix('!') {
                Some(value) => (true, value.trim_start()),
                None => (false, value),
            };
            spec.args.push(Arg {
                key: key.to_string(),
                negated,
                value: value.to_string(),
            });
        }
        Ok(spec)
    }

    /// Every problem with the arguments, in order.
    pub fn check(&self) -> Vec<String> {
        let mut checked = SelectorSpec::new(self.base.clone());
        let mut errors = Vec::new();
        for arg in &self.args {
            if let Err(error) = checked.push(arg.clone()) {
                errors.push(error);
                checked.args.push(arg.clone());
            }
        }
        errors
    }

    /// Adds an argument, if Minecraft would accept it here.
    pub fn push(&mut self, arg: Arg) -> Result<(), String> {
        let key = arg.key.as_str();
        let Some((_, kind, negatable)) = ARGS.iter().find(|(name, ..)| *name == key) else {
            let known: Vec<&str> = ARGS.iter().map(|(name, ..)| *name).collect();
            return Err(format!(
                "unknown selector argument '{key}'; the arguments are {}",
                known.join(", ")
            ));
        };
        if self.is_runtime_text() {
            return Err(
                "this selector's text is only known when the pack runs, so it can't take more arguments; build it with Selector.entities() and friends instead"
                    .to_string(),
            );
        }
        if let Base::Name(name) = &self.base {
            return Err(format!(
                "'{name}' is a player name, so it takes no arguments"
            ));
        }
        if arg.negated && !negatable {
            return Err(format!(
                "selector argument '{key}' can't be negated with '!'"
            ));
        }
        let base = self.base_text();
        if key == "type"
            && matches!(
                self.base,
                Base::AllPlayers | Base::NearestPlayer | Base::RandomPlayer
            )
        {
            return Err(format!(
                "'{base}' only selects players, so it can't take 'type'; use '@e[type=...]'"
            ));
        }
        if matches!(key, "limit" | "sort") && self.base == Base::Executor {
            return Err(format!("'@s' is one entity, so it can't take '{key}'"));
        }
        let mut same = self.args.iter().filter(|other| other.key == key);
        if ONE_POSITIVE.contains(&key) {
            if same.any(|other| !other.negated) {
                return Err(format!(
                    "selector argument '{key}' can only be given once without '!'"
                ));
            }
        } else if !REPEATABLE.contains(&key) && same.next().is_some() {
            return Err(format!("selector argument '{key}' can only be given once"));
        }
        if !arg.value.contains("$(") {
            check_value(key, *kind, &arg.value)?;
        }
        self.args.push(arg);
        Ok(())
    }

    pub fn render(&self) -> String {
        let mut out = self.base_text();
        if !self.args.is_empty() {
            let args: Vec<String> = self
                .args
                .iter()
                .map(|arg| {
                    format!(
                        "{}={}{}",
                        arg.key,
                        if arg.negated { "!" } else { "" },
                        arg.value
                    )
                })
                .collect();
            out.push('[');
            out.push_str(&args.join(","));
            out.push(']');
        }
        out
    }

    pub fn base_text(&self) -> String {
        match &self.base {
            Base::AllPlayers => "@a".to_string(),
            Base::Entities => "@e".to_string(),
            Base::NearestPlayer => "@p".to_string(),
            Base::RandomPlayer => "@r".to_string(),
            Base::Executor => "@s".to_string(),
            Base::NearestEntity => "@n".to_string(),
            Base::Name(name) => name.clone(),
        }
    }

    /// A selector whose whole text comes from a runtime string.
    pub fn is_runtime_text(&self) -> bool {
        matches!(&self.base, Base::Name(name) if name.contains("$("))
    }

    fn positive(&self, key: &str) -> Option<&str> {
        self.args
            .iter()
            .find(|arg| arg.key == key && !arg.negated)
            .map(|arg| arg.value.as_str())
    }

    /// Whether every match is a player, none is, or the compiler can't tell.
    pub fn ref_kind(&self) -> RefKind {
        if let Some(ty) = self.positive("type") {
            return if is_player_type(ty) {
                RefKind::Player
            } else if ty.starts_with('#') || ty.contains("$(") {
                RefKind::Unknown
            } else {
                RefKind::NonPlayer
            };
        }
        if self
            .args
            .iter()
            .any(|arg| arg.key == "type" && arg.negated && is_player_type(&arg.value))
        {
            return RefKind::NonPlayer;
        }
        match &self.base {
            Base::AllPlayers | Base::NearestPlayer | Base::RandomPlayer => RefKind::Player,
            // Code running as `@s` is almost always a player's (events, commands).
            Base::Executor => RefKind::Player,
            Base::Name(name) if !name.contains("$(") => RefKind::Player,
            _ => RefKind::Unknown,
        }
    }

    /// Makes the selector match at most one entity, as `getFirst()` needs.
    pub fn limit_one(&mut self) -> Result<(), String> {
        if let Some(limit) = self.args.iter().find(|arg| arg.key == "limit") {
            return if limit.value == "1" {
                Ok(())
            } else {
                Err("the selector must have no limit or 'limit=1'".to_string())
            };
        }
        match self.base {
            Base::Name(_)
            | Base::Executor
            | Base::NearestPlayer
            | Base::RandomPlayer
            | Base::NearestEntity => Ok(()),
            Base::AllPlayers | Base::Entities => {
                self.args.push(arg("limit", false, "1"));
                Ok(())
            }
        }
    }

    /// The same filters, applied to the entity running the command (`@s[...]`),
    /// for `entity.matches(selector)`.
    pub fn as_executor_filter(&self) -> Result<SelectorSpec, String> {
        let base = self.base_text();
        if let Some(arg) = self
            .args
            .iter()
            .find(|arg| matches!(arg.key.as_str(), "limit" | "sort"))
        {
            return Err(format!(
                "matches(...) checks one entity, so the selector can't use '{}'",
                arg.key
            ));
        }
        let mut filter = SelectorSpec::new(Base::Executor);
        match &self.base {
            Base::Entities | Base::Executor => {}
            Base::AllPlayers => filter.args.push(arg("type", false, "minecraft:player")),
            Base::NearestPlayer | Base::RandomPlayer | Base::NearestEntity | Base::Name(_) => {
                return Err(format!(
                    "matches(...) needs an '@a', '@e' or '@s' selector, not '{base}'"
                ));
            }
        }
        filter.args.extend(self.args.iter().cloned());
        Ok(filter)
    }
}

impl SelectorSpec {
    /// Rewrites the name inside each `$(...)` runtime value, in text order.
    pub fn map_markers(&mut self, mut rename: impl FnMut(&str) -> String) {
        if let Base::Name(name) = &mut self.base {
            *name = map_markers(name, &mut rename);
        }
        for arg in &mut self.args {
            arg.value = map_markers(&arg.value, &mut rename);
        }
    }
}

/// Rewrites the name inside each `$(...)` in `text`, in order.
pub fn map_markers(text: &str, mut rename: impl FnMut(&str) -> String) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("$(") {
        out.push_str(&rest[..start + 2]);
        let inner = &rest[start + 2..];
        let mut depth = 1;
        let mut quote: Option<char> = None;
        let mut end = inner.len();
        let mut chars = inner.char_indices();
        while let Some((index, ch)) = chars.next() {
            match (quote, ch) {
                (Some(_), '\\') => {
                    chars.next();
                }
                (Some(open), _) if ch == open => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(ch),
                (None, '(') => depth += 1,
                (None, ')') => {
                    depth -= 1;
                    if depth == 0 {
                        end = index;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push_str(&rename(&inner[..end]));
        rest = &inner[end..];
    }
    out.push_str(rest);
    out
}

pub fn arg(key: &str, negated: bool, value: &str) -> Arg {
    Arg {
        key: key.to_string(),
        negated,
        value: value.to_string(),
    }
}

pub fn is_player_type(value: &str) -> bool {
    matches!(value, "player" | "minecraft:player")
}

pub fn is_player_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 16
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_uuid(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

/// A `team=` or `name=` value: bare when it's one word, quoted otherwise.
pub fn quote_text(value: &str) -> String {
    if !value.is_empty() && value.chars().all(is_word_char) {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Characters Minecraft reads in an unquoted string.
fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '+')
}

/// Splits `inner` at commas that aren't inside quotes, `{}` or `[]`.
fn split_top_level(inner: &str) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut start = 0;
    for (index, ch) in inner.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == open {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!("unmatched '{ch}' in selector"));
                }
            }
            ',' if depth == 0 => {
                parts.push(&inner[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if quote.is_some() {
        return Err("unclosed quote in selector".to_string());
    }
    if depth != 0 {
        return Err("unclosed '{' or '[' in selector".to_string());
    }
    parts.push(&inner[start..]);
    if parts.iter().any(|part| part.trim().is_empty()) {
        return Err("empty selector argument".to_string());
    }
    Ok(parts)
}

fn check_value(key: &str, kind: Kind, value: &str) -> Result<(), String> {
    let bad = |what: &str| Err(format!("'{key}={value}': {what}"));
    match kind {
        Kind::Double => {
            if value.parse::<f64>().is_err() {
                return bad("expected a number");
            }
        }
        Kind::Distance | Kind::FloatRange => {
            let Some((min, max)) = parse_range(value, |text| text.parse::<f64>().ok()) else {
                return bad("expected a number or a range such as '1..5', '..5' or '1..'");
            };
            if matches!(kind, Kind::Distance) && min.or(max).is_some_and(|v| v < 0.0) {
                return bad("a distance can't be negative");
            }
            if let (Some(min), Some(max)) = (min, max)
                && min > max
            {
                return bad("the range's minimum is bigger than its maximum");
            }
        }
        Kind::Level => {
            let Some((min, max)) = parse_range(value, |text| text.parse::<i32>().ok()) else {
                return bad("expected a whole number or a range such as '1..5', '..5' or '1..'");
            };
            if min.or(max).is_some_and(|v| v < 0) {
                return bad("a level can't be negative");
            }
            if let (Some(min), Some(max)) = (min, max)
                && min > max
            {
                return bad("the range's minimum is bigger than its maximum");
            }
        }
        Kind::Limit => {
            if !value.parse::<i32>().is_ok_and(|limit| limit >= 1) {
                return bad("expected a whole number of at least 1");
            }
        }
        Kind::GameMode => {
            if !GAME_MODES.contains(&value) {
                return bad(&format!("expected one of {}", GAME_MODES.join(", ")));
            }
        }
        Kind::Sort => {
            if !SORTS.contains(&value) {
                return bad(&format!("expected one of {}", SORTS.join(", ")));
            }
        }
        Kind::EntityType => {
            if let Some(tag) = value.strip_prefix('#') {
                if !is_resource(tag) {
                    return bad("expected an entity type tag such as '#minecraft:skeletons'");
                }
            } else {
                let id = if value.contains(':') {
                    value.to_string()
                } else {
                    format!("minecraft:{value}")
                };
                if id.starts_with("minecraft:")
                    && !ids_for_category(MinecraftIdCategory::Entity).contains(&id.as_str())
                {
                    return bad("unknown entity type");
                }
                if !is_resource(&id) {
                    return bad("expected an entity type such as 'minecraft:pig'");
                }
            }
        }
        Kind::Resource => {
            if !is_resource(value) {
                return bad("expected an id such as 'my_pack:is_sneaking'");
            }
        }
        Kind::Word => {
            if !value.chars().all(is_word_char) {
                return bad("expected one word (letters, digits, '_', '-', '.', '+')");
            }
        }
        Kind::Text => {
            let quoted = value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')));
            if !quoted && !value.chars().all(is_word_char) {
                return bad("quote text that isn't one word");
            }
        }
        Kind::Nbt => {
            if !(value.starts_with('{') && value.ends_with('}')) {
                return bad("expected an NBT compound such as '{OnGround:1b}'");
            }
        }
        Kind::Scores => {
            let Some(inner) = braced(value) else {
                return bad("expected '{objective=range,...}'");
            };
            for entry in split_top_level(inner).map_err(|error| format!("'{key}': {error}"))? {
                let Some((objective, range)) = entry.split_once('=') else {
                    return bad("each score needs 'objective=range'");
                };
                let range = range.trim();
                if objective.trim().is_empty() || !objective.trim().chars().all(is_word_char) {
                    return bad("expected an objective name before '='");
                }
                if !range.contains("$(")
                    && parse_range(range, |text| text.parse::<i32>().ok()).is_none()
                {
                    return bad("each score needs a whole number or a range such as '1..5'");
                }
            }
        }
        Kind::Advancements => {
            let Some(inner) = braced(value) else {
                return bad("expected '{advancement=true,...}'");
            };
            for entry in split_top_level(inner).map_err(|error| format!("'{key}': {error}"))? {
                let Some((id, done)) = entry.split_once('=') else {
                    return bad("each advancement needs 'id=true' or 'id=false'");
                };
                let done = done.trim();
                if !is_resource(id.trim()) {
                    return bad("expected an advancement id before '='");
                }
                if !matches!(done, "true" | "false") && braced(done).is_none() {
                    return bad("expected 'true', 'false' or '{criterion=true,...}'");
                }
            }
        }
    }
    Ok(())
}

fn braced(value: &str) -> Option<&str> {
    let inner = value.strip_prefix('{')?.strip_suffix('}')?;
    (!inner.trim().is_empty()).then_some(inner)
}

/// `5`, `1..5`, `..5` or `1..`. Each side is `None` when left open.
#[allow(clippy::type_complexity)]
fn parse_range<T: Copy>(
    text: &str,
    number: impl Fn(&str) -> Option<T>,
) -> Option<(Option<T>, Option<T>)> {
    match text.split_once("..") {
        None => {
            let value = number(text)?;
            Some((Some(value), Some(value)))
        }
        Some((min, max)) => {
            let min = if min.is_empty() {
                None
            } else {
                Some(number(min)?)
            };
            let max = if max.is_empty() {
                None
            } else {
                Some(number(max)?)
            };
            (min.is_some() || max.is_some()).then_some((min, max))
        }
    }
}

/// `namespace:path` or `path`, with Minecraft's allowed characters.
fn is_resource(text: &str) -> bool {
    let (namespace, path) = text.split_once(':').unwrap_or(("minecraft", text));
    !namespace.is_empty()
        && !path.is_empty()
        && namespace
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || "_-.".contains(ch))
        && path
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || "_-./".contains(ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(text: &str) -> Vec<String> {
        SelectorSpec::parse(text).unwrap().check()
    }

    #[test]
    fn parses_and_renders() {
        let spec = SelectorSpec::parse("@e[ type=pig , nbt={a:[1,2]}, name=\"a,b\" ]").unwrap();
        assert_eq!(spec.render(), "@e[type=pig,nbt={a:[1,2]},name=\"a,b\"]");
        assert_eq!(SelectorSpec::parse("@a[]").unwrap().render(), "@a");
        assert_eq!(
            SelectorSpec::parse("Steve").unwrap().base,
            Base::Name("Steve".into())
        );
        assert!(SelectorSpec::parse("@x").is_err());
        assert!(SelectorSpec::parse("@e[type=pig").is_err());
        assert!(SelectorSpec::parse("@e[tag]").is_err());
        assert!(SelectorSpec::parse("not a selector").is_err());
    }

    #[test]
    fn checks_arguments() {
        assert!(errors("@e[type=minecraft:pig,tag=a,tag=!b,distance=..5,limit=1]").is_empty());
        assert!(
            errors("@e[scores={kills=5..,deaths=..2},advancements={story/root=true}]").is_empty()
        );
        assert_eq!(errors("@e[colour=red]").len(), 1);
        assert_eq!(errors("@a[type=pig]").len(), 1);
        assert_eq!(errors("@s[limit=2]").len(), 1);
        assert_eq!(errors("@e[limit=1,limit=2]").len(), 1);
        assert_eq!(errors("@e[type=pig,type=cow]").len(), 1);
        assert!(errors("@e[type=!pig,type=!cow]").is_empty());
        assert_eq!(errors("@e[distance=-1..]").len(), 1);
        assert_eq!(errors("@e[distance=5..1]").len(), 1);
        assert_eq!(errors("@e[type=chicke]").len(), 1);
        assert_eq!(errors("@e[gamemode=hardcore]").len(), 1);
        assert_eq!(errors("@e[limit=!1]").len(), 1);
        assert!(errors("@e[tag=$(value)]").is_empty());
    }

    #[test]
    fn classifies_players() {
        let kind = |text: &str| SelectorSpec::parse(text).unwrap().ref_kind();
        assert_eq!(kind("@a"), RefKind::Player);
        assert_eq!(kind("@e[type=minecraft:player]"), RefKind::Player);
        assert_eq!(kind("@e[type=!player]"), RefKind::NonPlayer);
        assert_eq!(kind("@s[type=pig]"), RefKind::NonPlayer);
        assert_eq!(kind("@e[tag=type=player]"), RefKind::Unknown);
        assert_eq!(kind("@e"), RefKind::Unknown);
    }

    #[test]
    fn renames_markers() {
        let mut next = 0;
        let text = map_markers("@e[tag=$(a),name=\"$(f(\")\"))\"]", |_| {
            next += 1;
            format!("p{next}")
        });
        assert_eq!(text, "@e[tag=$(p1),name=\"$(p2)\"]");
    }

    #[test]
    fn limits_and_filters() {
        let mut spec = SelectorSpec::parse("@e[type=pig]").unwrap();
        spec.limit_one().unwrap();
        assert_eq!(spec.render(), "@e[type=pig,limit=1]");
        assert!(
            SelectorSpec::parse("@e[limit=2]")
                .unwrap()
                .limit_one()
                .is_err()
        );
        let filter = SelectorSpec::parse("@a[tag=x]")
            .unwrap()
            .as_executor_filter()
            .unwrap();
        assert_eq!(filter.render(), "@s[type=minecraft:player,tag=x]");
        assert!(
            SelectorSpec::parse("@e[sort=nearest]")
                .unwrap()
                .as_executor_filter()
                .is_err()
        );
    }
}
