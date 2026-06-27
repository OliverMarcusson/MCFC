# Event Catalog

MCFC has two event surfaces: vanilla-safe declarations that compile to ordinary datapack behavior, and opt-in agent-backed typed events enabled with `[helper.agent] enabled = true`.

## Under The Hood

Vanilla-safe events are implemented entirely inside the generated datapack. Join detection uses generated tags, death detection uses `deathCount` scoreboard objectives, commands use trigger objectives, and tasks use generated counters or `schedule function`.

Agent-backed events use generated entrypoints under `agent/event/<name>.mcfunction`. The JVM agent writes the current event payload into command storage; the generated wrapper copies it into the typed event parameter slot, resets the handler control slot, and calls the lowered MCFC handler. If the handler calls `event.cancel()`, MCFC writes `decision.cancel` into the generated agent decision storage.

## Vanilla-safe Events

| Declaration | Payload | Cancellation | Notes |
| --- | --- | --- | --- |
| [`event player_join:`](./events/player-join) | none | No | Runs once as every player first seen by the pack. |
| [`event player_death:`](./events/player-death) | none | No | Runs as a player when their `deathCount` score changes. |

Vanilla handlers execute as the affected player. Use `single(selector("@s"))` to get a `player_ref`.

```mcfc
event player_join:
    let player = single(selector("@s"))
    player.tellraw("Welcome!")

event player_death:
    let player = single(selector("@s"))
    player.state.deaths = player.state.deaths + 1
```

## Agent-backed Events

Agent events are version-pinned to Minecraft `26.2`. The generated datapack remains valid without the agent, and `mcfd` performs best-effort dynamic attachment.

| Event | Payload type | Important fields | Cancellation |
| --- | --- | --- | --- |
| [`chat`](./events/chat) | `chat_event` | `player`, `message`, `cancelled` | Yes |
| [`inventory_click`](./events/inventory-click) | `inventory_click_event` | `player`, `container_id`, `state_id`, `slot`, `button`, `cancelled` | Yes |
| [`player_action`](./events/player-action) | `player_action_event` | `player`, `action`, `face`, `x`, `y`, `z`, `cancelled` | Yes |
| [`block_break`](./events/block-break) | `block_break_event` | `player`, `x`, `y`, `z`, `cancelled` | Yes |
| [`player_interact_block`](./events/player-interact-block) | `player_interact_block_event` | `player`, `hand`, `face`, `x`, `y`, `z`, `cancelled` | Yes |
| [`player_interact_item`](./events/player-interact-item) | `player_interact_item_event` | `player`, `hand`, `cancelled` | Yes |
| [`entity_interact`](./events/entity-interact) | `entity_interact_event` | `player`, `target_id`, `hand`, `secondary`, `cancelled` | Yes |
| [`entity_attack`](./events/entity-attack) | `entity_attack_event` | `player`, `target_id`, `cancelled` | Yes |
| [`item_held_change`](./events/item-held-change) | `item_held_change_event` | `player`, `slot`, `cancelled` | Yes |
| [`inventory_close`](./events/inventory-close) | `inventory_close_event` | `player`, `container_id`, `cancelled` | Yes |
| [`player_swing`](./events/player-swing) | `player_swing_event` | `player`, `hand`, `cancelled` | Yes |
| [`player_action_toggle`](./events/player-action-toggle) | `player_action_toggle_event` | `player`, `action`, `entity_id`, `data`, `cancelled` | Yes |
| [`item_rename`](./events/item-rename) | `item_rename_event` | `player`, `name`, `cancelled` | Yes |
| [`trade_select`](./events/trade-select) | `trade_select_event` | `player`, `trade_index`, `cancelled` | Yes |
| [`sign_change`](./events/sign-change) | `sign_change_event` | `player`, `x`, `y`, `z`, `front`, `line_1`-`line_4`, `cancelled` | Yes |
| [`recipe_place`](./events/recipe-place) | `recipe_place_event` | `player`, `container_id`, `recipe`, `use_max_items`, `cancelled` | Yes |
| [`game_mode_request`](./events/game-mode-request) | `game_mode_request_event` | `player`, `mode`, `cancelled` | Yes |
| [`player_respawn_request`](./events/player-respawn-request) | `agent_event` | generic payload | Yes |
| [`book_edit`](./events/book-edit) | `agent_event` | generic payload | Yes |
| [`beacon_effect`](./events/beacon-effect) | `agent_event` | generic payload | Yes |
| [`item_pick`](./events/item-pick) | `agent_event` | generic payload | Yes |
| [`entity_teleport`](./events/entity-teleport) | `agent_event` | generic payload | Yes |
| [`player_abilities`](./events/player-abilities) | `agent_event` | generic payload | Yes |
| [`player_connect`](./events/player-connect) | `agent_event` | generic payload | No |
| [`player_quit`](./events/player-quit) | `agent_event` | generic payload | No |
| [`player_respawn`](./events/player-respawn) | `agent_event` | generic payload | No |
| [`player_damage`](./events/player-damage) | `agent_event` | generic payload | No |
| [`player_teleport`](./events/player-teleport) | `agent_event` | generic payload | No |
| [`player_item_drop`](./events/player-item-drop) | `agent_event` | generic payload | No |
| [`player_item_pickup`](./events/player-item-pickup) | `agent_event` | generic payload | No |
| [`inventory_open`](./events/inventory-open) | `agent_event` | generic payload | No |
| [`game_mode_change`](./events/game-mode-change) | `agent_event` | generic payload | No |

The broader `agent_event` has `player`, `player_name`, `source`, `payload`, and `cancelled` fields.

## Examples

```mcfc
event chat(event: chat_event):
    if event.message == "spark":
        event.cancel()
        event.player.tellraw("Spark accepted")
```

```mcfc
event inventory_click(event: inventory_click_event):
    event.player.state.last_slot = event.slot
    event.player.actionbar("slot=$(event.slot), button=$(event.button)")
```

```mcfc
event block_break(event: block_break_event):
    if event.player.has_tag("protected"):
        event.cancel()
        event.player.tellraw("You cannot break blocks here")
```

```mcfc
event player_interact_block(event: player_interact_block_event):
    if event.hand == "MAIN_HAND":
        event.player.actionbar("Touched $(event.face): $(event.x), $(event.y), $(event.z)")
```

```mcfc
event player_damage(event: agent_event):
    event.player.tellraw("Damage event: $(event.payload)")
```

## Commands

`command name:` always retains its vanilla `/trigger mcfcc_name` fallback. When the agent is attached, the same declaration also reserves the real `/name` root and dispatches it as the player.

```mcfc
command status:
    let player = single(selector("@s"))
    player.tellraw("Ready")
```

::: warning Experimental and version-pinned
Agent callbacks are pinned to Minecraft `26.2`. Cancellable packet-entry events can call `event.cancel()`. Lifecycle callbacks such as damage, teleport, join, quit, and respawn are observation-only and reject cancellation.
:::
