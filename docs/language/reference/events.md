# Events

An `event` declaration runs its body when something happens in game. There are two kinds:

- **Vanilla events** work in any datapack.
- **Agent events** need the optional [`mcfd-agent`](/runtime/mcfd-agent) and `[helper.agent] enabled = true` in `mcfc.toml`. They carry a typed payload, and some can be cancelled. A pack that uses them still loads without the agent. Its agent handlers just never run.

## Vanilla events

| Declaration | Runs |
| --- | --- |
| `event player_join:` | Once per player, the first time the pack sees them. Tracked with the tag `mcfc_join_<namespace>`, so it doesn't run again on later logins. |
| `event player_death:` | Each time a player dies, detected through a `deathCount` objective. |

Vanilla handlers take no parameter and run as the affected player. Get a `player_ref` with `single(selector("@s"))`:

```mcfc
event player_join:
    let player = single(selector("@s"))
    player.tellraw("Welcome!")

event player_death:
    let player = single(selector("@s"))
    player.state.deaths = player.state.deaths + 1
```

For something that runs repeatedly, use a [`task`](./statements#task) or `fn tick()`. For something players run, use a [`command`](./statements#command).

## Agent events

Declare the payload as the handler's parameter:

```mcfc
event chat(event: chat_event):
    if event.message == "spark":
        event.cancel()
        event.player.tellraw("Spark accepted")
```

Every payload has `player: player_ref` and `cancelled: bool`. On events marked cancellable, `event.cancel()` stops the action from happening. Calling it on any other event is a compile error.

### Typed payloads

| Event | Payload type | Other fields | Cancellable |
| --- | --- | --- | --- |
| `chat` | `chat_event` | `message: string` | Yes |
| `block_break` | `block_break_event` | `x, y, z: int` | Yes |
| `player_interact_block` | `player_interact_block_event` | `hand: string`, `face: string`, `x, y, z: int` | Yes |
| `player_interact_item` | `player_interact_item_event` | `hand: string` | Yes |
| `entity_interact` | `entity_interact_event` | `target_id: int`, `hand: string`, `secondary: bool` | Yes |
| `entity_attack` | `entity_attack_event` | `target_id: int` | Yes |
| `player_action` | `player_action_event` | `action: string`, `face: string`, `x, y, z: int` | Yes |
| `player_action_toggle` | `player_action_toggle_event` | `action: string`, `entity_id: int`, `data: int` | Yes |
| `player_swing` | `player_swing_event` | `hand: string` | Yes |
| `inventory_click` | `inventory_click_event` | `container_id, state_id, slot, button: int` | Yes |
| `inventory_close` | `inventory_close_event` | `container_id: int` | Yes |
| `item_held_change` | `item_held_change_event` | `slot: int` | Yes |
| `item_rename` | `item_rename_event` | `name: string` | Yes |
| `trade_select` | `trade_select_event` | `trade_index: int` | Yes |
| `sign_change` | `sign_change_event` | `x, y, z: int`, `front: bool`, `line_1` to `line_4: string` | Yes |
| `recipe_place` | `recipe_place_event` | `container_id: int`, `recipe: string`, `use_max_items: bool` | Yes |
| `game_mode_request` | `game_mode_request_event` | `mode: string` | Yes |

### Generic payloads

These events use `agent_event`, which has `player_name: string`, `source: string` and `payload: string`. `payload` is the raw event data as text.

| Cancellable | Events |
| --- | --- |
| Yes | `player_respawn_request`, `book_edit`, `beacon_effect`, `item_pick`, `entity_teleport`, `player_abilities` |
| No | `player_connect`, `player_quit`, `player_respawn`, `player_damage`, `player_teleport`, `player_item_drop`, `player_item_pickup`, `inventory_open`, `game_mode_change` |

```mcfc
event player_damage(event: agent_event):
    event.player.tellraw("Damage event: $(event.payload)")
```

Agent events are tied to Minecraft 26.3. The agent sends events that arrive as network packets before the server acts on them, and those can be cancelled. Lifecycle events such as damage, quit and respawn are reported after they happen, so they can't be cancelled.

## Under the hood

Vanilla events are checked by the generated tick function. Agent events call `agent/event/<name>.mcfunction`. The agent writes the payload to command storage, and the wrapper copies it into the handler's parameter before calling it. `event.cancel()` writes `decision.cancel` to storage, and the agent reads it back before letting the action through.
