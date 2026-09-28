//! Stable editor-facing metadata for the public MCFC language surface.
//! Keep additions here so compiler, language server, and editor tooling share
//! the names users write instead of maintaining independent stale lists.

pub const VANILLA_EVENTS: &[&str] = &[
    "player_join",
    "player_death",
    "block_place",
    "player_item_consume",
    "player_use_item",
    "player_kill_entity",
    "player_hurt_entity",
    "player_interact_entity",
    "entity_hurt_player",
];

/// Vanilla events raised by an advancement trigger, with the trigger they use.
pub const ADVANCEMENT_EVENTS: &[(&str, &str)] = &[
    ("block_place", "placed_block"),
    ("player_item_consume", "consume_item"),
    ("player_use_item", "using_item"),
    ("player_kill_entity", "player_killed_entity"),
    ("player_hurt_entity", "player_hurt_entity"),
    ("player_interact_entity", "player_interacted_with_entity"),
    ("entity_hurt_player", "entity_hurt_player"),
];

/// Vanilla events whose payload also has `block()`, the block involved.
pub fn vanilla_event_has_block(kind: &str) -> bool {
    kind == "block_place"
}

/// Vanilla events whose payload also has `entity()`, the other entity involved.
pub fn vanilla_event_has_entity(kind: &str) -> bool {
    matches!(
        kind,
        "player_hurt_entity" | "player_interact_entity" | "entity_hurt_player"
    )
}
pub const AGENT_EVENTS: &[&str] = &[
    "chat",
    "inventory_click",
    "player_action",
    "block_break",
    "player_interact_block",
    "player_interact_item",
    "entity_interact",
    "entity_attack",
    "item_held_change",
    "inventory_close",
    "player_swing",
    "player_action_toggle",
    "player_respawn_request",
    "item_rename",
    "trade_select",
    "sign_change",
    "book_edit",
    "beacon_effect",
    "recipe_place",
    "item_pick",
    "entity_teleport",
    "game_mode_request",
    "player_abilities",
    "player_connect",
    "player_quit",
    "player_respawn",
    "player_damage",
    "player_teleport",
    "player_item_drop",
    "player_item_pickup",
    "inventory_open",
    "game_mode_change",
];

/// Agent events whose payload carries only the shared fields
/// (`player`, `playerName`, `source`, `payload`, `cancelled`).
pub const GENERIC_AGENT_EVENTS: &[&str] = &[
    "player_respawn_request",
    "book_edit",
    "beacon_effect",
    "item_pick",
    "entity_teleport",
    "player_abilities",
    "player_connect",
    "player_quit",
    "player_respawn",
    "player_damage",
    "player_teleport",
    "player_item_drop",
    "player_item_pickup",
    "inventory_open",
    "game_mode_change",
];

/// The parameter type that selects an event: `player_join` is `PlayerJoinEvent`.
pub fn event_type_name(kind: &str) -> String {
    let mut name: String = kind
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect();
    name.push_str("Event");
    name
}

/// The event an `@EventHandler` parameter type listens to.
pub fn event_kind_for_type(ty: &str) -> Option<&'static str> {
    VANILLA_EVENTS
        .iter()
        .chain(AGENT_EVENTS)
        .copied()
        .find(|kind| event_type_name(kind) == ty)
}

/// Builtin functions users write in camelCase, with the name the compiler uses.
pub const FUNCTION_NAMES: &[(&str, &str)] = &[
    ("sleepTicks", "sleep_ticks"),
    ("gameTime", "game_time"),
    ("worldTime", "world_time"),
    ("borderSize", "border_size"),
    ("hasData", "has_data"),
    ("randomWeighted", "random_weighted"),
    ("randomBinomial", "random_binomial"),
    ("debugEntity", "debug_entity"),
    ("debugMarker", "debug_marker"),
    ("lootGive", "loot_give"),
    ("lootInsert", "loot_insert"),
    ("lootSpawn", "loot_spawn"),
];

/// Builtins that used to be free functions, with the internal name and the
/// class method users write now: `sleep(1)` is `Thread.sleep(1)`. Only `std`
/// may still call them bare.
pub const CLASS_BUILTINS: &[(&str, &str, &str)] = &[
    ("debug", "debug", "System.out.println"),
    ("sleep", "sleep", "Thread.sleep"),
    ("sleepTicks", "sleep_ticks", "Thread.sleepTicks"),
    ("random", "random", "Random.nextInt"),
    ("randomWeighted", "random_weighted", "Random.weighted"),
    ("randomBinomial", "random_binomial", "Random.binomial"),
    ("gameTime", "game_time", "World.getGameTime"),
    ("worldTime", "world_time", "World.getTime"),
    ("borderSize", "border_size", "World.getBorderSize"),
    ("gamerule", "gamerule", "World.getGameRule"),
    ("summon", "summon", "World.summon"),
    ("hasData", "has_data", "Nbt.has"),
    ("mc", "mc", "Commands.run"),
    ("mcf", "mcf", "Commands.run"),
];

/// The class method a free builtin is written as now, by written or internal name.
pub fn class_builtin(name: &str) -> Option<&'static str> {
    CLASS_BUILTINS
        .iter()
        .find(|(written, internal, _)| *written == name || *internal == name)
        .map(|(_, _, class)| *class)
}

/// Methods users write with Java names, with the name the compiler uses.
/// `add` is `push` with one argument and `insert` with two.
pub const METHOD_NAMES: &[(&str, &str)] = &[
    ("size", "len"),
    ("length", "len"),
    ("removeLast", "pop"),
    ("getFirst", "first"),
    ("getLast", "last"),
    ("indexOf", "index_of"),
    ("containsKey", "has"),
    ("keySet", "keys"),
    ("substring", "slice"),
    ("toString", "to_string"),
    ("addTag", "add_tag"),
    ("removeTag", "remove_tag"),
    ("hasTag", "has_tag"),
    ("asNbt", "as_nbt"),
    ("distanceTo", "distance_to"),
    ("inBiome", "in_biome"),
    ("spawnItem", "spawn_item"),
    ("lootGive", "loot_give"),
    ("lootInsert", "loot_insert"),
    ("lootSpawn", "loot_spawn"),
    ("debugEntity", "debug_entity"),
    ("debugMarker", "debug_marker"),
];

/// Entity, selector and block methods with Bukkit-style names, and the name the
/// compiler uses. They are mapped by the type checker, not the parser, so a
/// record component such as `title()` keeps its own name.
pub const ENTITY_METHOD_NAMES: &[(&str, &str)] = &[
    ("sendMessage", "tellraw"),
    ("sendTitle", "title"),
    ("sendActionBar", "actionbar"),
    ("playSound", "playsound"),
    ("stopSound", "stopsound"),
    ("setBlock", "setblock"),
    ("spawnParticle", "particle"),
    ("getLightLevel", "light"),
    ("getBiome", "biome"),
    ("getEnvironment", "environment"),
    ("getX", "x"),
    ("getY", "y"),
    ("getZ", "z"),
    ("getYaw", "yaw"),
    ("getPitch", "pitch"),
    ("getLookX", "look_x"),
    ("getLookY", "look_y"),
    ("getLookZ", "look_z"),
    ("getHealth", "health"),
    ("getFoodLevel", "food"),
    ("getLevel", "xp_level"),
    ("getSelectedSlot", "selected_slot"),
    ("getDimension", "dimension"),
];

/// Names that used to mean an entity or block method, with the name to use now.
pub const OLD_ENTITY_METHOD_NAMES: &[(&str, &str)] = &[
    ("tellraw", "sendMessage"),
    ("title", "sendTitle"),
    ("actionbar", "sendActionBar"),
    ("playsound", "playSound"),
    ("stopsound", "stopSound"),
    ("setblock", "setBlock"),
    ("particle", "spawnParticle"),
    ("light", "getLightLevel"),
    ("biome", "getBiome"),
    ("environment", "getEnvironment"),
    ("x", "getX"),
    ("y", "getY"),
    ("z", "getZ"),
    ("yaw", "getYaw"),
    ("pitch", "getPitch"),
    ("lookX", "getLookX"),
    ("lookY", "getLookY"),
    ("lookZ", "getLookZ"),
    ("health", "getHealth"),
    ("food", "getFoodLevel"),
    ("xpLevel", "getLevel"),
    ("gameMode", "getGameMode"),
    ("selectedSlot", "getSelectedSlot"),
    ("dimension", "getDimension"),
];

/// Properties of builders and bossbars, read with `getName()` and written with
/// `setName(...)`. Raw Minecraft data (`nbt.*`, `states.*`, text fields) keeps
/// field syntax.
pub fn property_names(ty: &crate::ast::Type) -> &'static [&'static str] {
    use crate::ast::Type;
    match ty {
        Type::Bossbar => &["name", "value", "max", "visible", "players"],
        Type::ItemDef => &["count", "name"],
        Type::EntityDef => &["name", "nameVisible", "noAi", "silent", "glowing", "tags"],
        Type::BlockDef => &["name", "lock", "lootTable", "lootSeed"],
        _ => &[],
    }
}

/// `setMax` gives `max`; `getLootTable` gives `lootTable`.
pub fn accessor_property(method: &str, prefix: &str) -> Option<String> {
    let rest = method.strip_prefix(prefix)?;
    let mut chars = rest.chars();
    let first = chars.next()?;
    first
        .is_ascii_uppercase()
        .then(|| first.to_ascii_lowercase().to_string() + chars.as_str())
}

/// `max` gives `Max`.
pub fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

pub fn internal_function_name(name: &str) -> &str {
    FUNCTION_NAMES
        .iter()
        .find(|(java, _)| *java == name)
        .map_or(name, |(_, internal)| internal)
}

pub fn internal_method_name(name: &str, arg_count: usize) -> &str {
    match (name, arg_count) {
        ("add", 2) => "insert",
        ("add", _) => "push",
        _ => METHOD_NAMES
            .iter()
            .find(|(java, _)| *java == name)
            .map_or(name, |(_, internal)| internal),
    }
}

/// Every Java method name that maps to `internal`: `len` is both `size` and `length`.
pub fn java_method_names(internal: &str) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = METHOD_NAMES
        .iter()
        .filter(|(_, name)| *name == internal)
        .map(|(java, _)| *java)
        .collect();
    if matches!(internal, "push" | "insert") {
        names.push("add");
    }
    names
}

/// The name to suggest when source uses an internal name directly.
pub fn java_name_for(internal: &str, is_method: bool) -> Option<&'static str> {
    if is_method && matches!(internal, "push" | "insert") {
        return Some("add");
    }
    let table = if is_method {
        METHOD_NAMES
    } else {
        FUNCTION_NAMES
    };
    table
        .iter()
        .find(|(_, name)| *name == internal)
        .map(|(java, _)| *java)
}

/// How a builtin or method reads in source, for messages: `bool` is `(boolean) x`,
/// `selector` is `Selector.of(...)`, `sleep_ticks` is `Thread.sleepTicks(...)`.
pub fn display_call(internal: &str) -> String {
    let cast = match internal {
        "int" => Some("int"),
        "float" => Some("float"),
        "bool" => Some("boolean"),
        "string" => Some("String"),
        "player_ref" => Some("Player"),
        _ => None,
    };
    if let Some(ty) = cast {
        return format!("({ty}) x");
    }
    if let Some(class) = class_builtin(internal) {
        return format!("{class}(...)");
    }
    let name = match internal {
        "selector" => "Selector.of",
        "block" => "Block.of",
        "item" => "new ItemStack",
        "entity" => "new EntityData",
        "block_type" => "new BlockData",
        "text" => "new Component",
        "bossbar" => "new BossBar",
        "single" => "getFirst",
        "find_first" => "findFirst",
        "exists" => "isValid",
        _ => ENTITY_METHOD_NAMES
            .iter()
            .find(|(_, name)| *name == internal)
            .map(|(java, _)| *java)
            .or_else(|| java_name_for(internal, true))
            .or_else(|| java_name_for(internal, false))
            .unwrap_or(internal),
    };
    format!("{name}(...)")
}
