//! Stable editor-facing metadata for the public MCFC language surface.
//! Keep additions here so compiler, language server, and editor tooling share
//! the names users write instead of maintaining independent stale lists.

pub const TOP_LEVEL_DECLARATIONS: &[&str] = &["event", "command", "task"];
pub const VANILLA_EVENTS: &[&str] = &["player_join", "player_death"];
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

pub fn agent_event_payload_type(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "chat" => "ChatEvent",
        "inventory_click" => "InventoryClickEvent",
        "player_action" => "PlayerActionEvent",
        "block_break" => "BlockBreakEvent",
        "player_interact_block" => "PlayerInteractBlockEvent",
        "player_interact_item" => "PlayerInteractItemEvent",
        "entity_interact" => "EntityInteractEvent",
        "entity_attack" => "EntityAttackEvent",
        "item_held_change" => "ItemHeldChangeEvent",
        "inventory_close" => "InventoryCloseEvent",
        "player_swing" => "PlayerSwingEvent",
        "player_action_toggle" => "PlayerActionToggleEvent",
        "item_rename" => "ItemRenameEvent",
        "trade_select" => "TradeSelectEvent",
        "sign_change" => "SignChangeEvent",
        "recipe_place" => "RecipePlaceEvent",
        "game_mode_request" => "GameModeRequestEvent",
        "player_respawn_request"
        | "book_edit"
        | "beacon_effect"
        | "item_pick"
        | "entity_teleport"
        | "player_abilities"
        | "player_connect"
        | "player_quit"
        | "player_respawn"
        | "player_damage"
        | "player_teleport"
        | "player_item_drop"
        | "player_item_pickup"
        | "inventory_open"
        | "game_mode_change" => "AgentEvent",
        _ => return None,
    })
}

/// Builtin functions users write in camelCase, with the name the compiler uses.
pub const FUNCTION_NAMES: &[(&str, &str)] = &[
    ("sleepTicks", "sleep_ticks"),
    ("gameTime", "game_time"),
    ("worldTime", "world_time"),
    ("borderSize", "border_size"),
    ("findFirst", "find_first"),
    ("hasData", "has_data"),
    ("randomWeighted", "random_weighted"),
    ("randomBinomial", "random_binomial"),
    ("debugEntity", "debug_entity"),
    ("debugMarker", "debug_marker"),
    ("lootGive", "loot_give"),
    ("lootInsert", "loot_insert"),
    ("lootSpawn", "loot_spawn"),
];

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
    ("parseInt", "parse_int"),
    ("toString", "to_string"),
    ("addTag", "add_tag"),
    ("removeTag", "remove_tag"),
    ("hasTag", "has_tag"),
    ("asNbt", "as_nbt"),
    ("distanceTo", "distance_to"),
    ("gameMode", "game_mode"),
    ("inBiome", "in_biome"),
    ("lookX", "look_x"),
    ("lookY", "look_y"),
    ("lookZ", "look_z"),
    ("selectedSlot", "selected_slot"),
    ("spawnItem", "spawn_item"),
    ("xpLevel", "xp_level"),
    ("lootGive", "loot_give"),
    ("lootInsert", "loot_insert"),
    ("lootSpawn", "loot_spawn"),
    ("debugEntity", "debug_entity"),
    ("debugMarker", "debug_marker"),
];

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
