//! Stable editor-facing metadata for the public MCFC language surface.
//! Keep additions here so compiler, language server, and editor tooling share
//! the names users write instead of maintaining independent stale lists.

pub const TOP_LEVEL_DECLARATIONS: &[&str] = &["data", "event", "command", "task"];
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
        "chat" => "chat_event",
        "inventory_click" => "inventory_click_event",
        "player_action" => "player_action_event",
        "block_break" => "block_break_event",
        "player_interact_block" => "player_interact_block_event",
        "player_interact_item" => "player_interact_item_event",
        "entity_interact" => "entity_interact_event",
        "entity_attack" => "entity_attack_event",
        "item_held_change" => "item_held_change_event",
        "inventory_close" => "inventory_close_event",
        "player_swing" => "player_swing_event",
        "player_action_toggle" => "player_action_toggle_event",
        "item_rename" => "item_rename_event",
        "trade_select" => "trade_select_event",
        "sign_change" => "sign_change_event",
        "recipe_place" => "recipe_place_event",
        "game_mode_request" => "game_mode_request_event",
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
        | "game_mode_change" => "agent_event",
        _ => return None,
    })
}
