# Events

A function annotated with `@Event(NAME)` runs when something happens in game. There are two kinds:

- **Vanilla events** work in any datapack.
- **Agent events** need the optional [`mcfd-agent`](/runtime/mcfd-agent) and `[helper.agent] enabled = true` in `mcfc.toml`. They carry a typed payload, and some can be cancelled. A pack that uses them still loads without the agent. Its agent handlers just never run.

## Vanilla events

| Annotation | Runs |
| --- | --- |
| `@Event(PLAYER_JOIN)` | Once per player, the first time the pack sees them. Tracked with the tag `mcfc_join_<namespace>`, so it doesn't run again on later logins. |
| `@Event(PLAYER_DEATH)` | Each time a player dies, detected through a `deathCount` objective. |

Vanilla handlers run as the affected player. They take no parameter, or one `Player` parameter that is bound to that player:

```mcfc
@Event(PLAYER_JOIN)
void onPlayerJoin(Player player) {
    player.tellraw("Welcome!");
}

@Event(PLAYER_DEATH)
void onPlayerDeath() {
    var player = single(selector("@s"));
    player.state.deaths = player.state.deaths + 1;
}
```

For something that runs repeatedly, use [`@Every`](./statements#every-and-after) or `void tick()`. For something players run, use [`@Command`](./statements#command).

## Agent events

Declare the payload as the handler's only parameter:

```mcfc
@Event(CHAT)
void onChat(ChatEvent event) {
    if (event.message == "spark") {
        event.cancel();
        event.player.tellraw("Spark accepted");
    }
}
```

Every payload has `player: Player` and `cancelled: boolean`. On events marked cancellable, `event.cancel()` stops the action from happening. Calling it on any other event is a compile error.

### Typed payloads

| Event | Payload type | Other fields | Cancellable |
| --- | --- | --- | --- |
| `CHAT` | `ChatEvent` | `message: String` | Yes |
| `BLOCK_BREAK` | `BlockBreakEvent` | `x, y, z: int` | Yes |
| `PLAYER_INTERACT_BLOCK` | `PlayerInteractBlockEvent` | `hand: String`, `face: String`, `x, y, z: int` | Yes |
| `PLAYER_INTERACT_ITEM` | `PlayerInteractItemEvent` | `hand: String` | Yes |
| `ENTITY_INTERACT` | `EntityInteractEvent` | `targetId: int`, `hand: String`, `secondary: boolean` | Yes |
| `ENTITY_ATTACK` | `EntityAttackEvent` | `targetId: int` | Yes |
| `PLAYER_ACTION` | `PlayerActionEvent` | `action: String`, `face: String`, `x, y, z: int` | Yes |
| `PLAYER_ACTION_TOGGLE` | `PlayerActionToggleEvent` | `action: String`, `entityId: int`, `data: int` | Yes |
| `PLAYER_SWING` | `PlayerSwingEvent` | `hand: String` | Yes |
| `INVENTORY_CLICK` | `InventoryClickEvent` | `containerId, stateId, slot, button: int` | Yes |
| `INVENTORY_CLOSE` | `InventoryCloseEvent` | `containerId: int` | Yes |
| `ITEM_HELD_CHANGE` | `ItemHeldChangeEvent` | `slot: int` | Yes |
| `ITEM_RENAME` | `ItemRenameEvent` | `name: String` | Yes |
| `TRADE_SELECT` | `TradeSelectEvent` | `tradeIndex: int` | Yes |
| `SIGN_CHANGE` | `SignChangeEvent` | `x, y, z: int`, `front: boolean`, `line1` to `line4: String` | Yes |
| `RECIPE_PLACE` | `RecipePlaceEvent` | `containerId: int`, `recipe: String`, `useMaxItems: boolean` | Yes |
| `GAME_MODE_REQUEST` | `GameModeRequestEvent` | `mode: String` | Yes |

### Generic payloads

These events use `AgentEvent`, which has `playerName: String`, `source: String` and `payload: String`. `payload` is the raw event data as text.

| Cancellable | Events |
| --- | --- |
| Yes | `PLAYER_RESPAWN_REQUEST`, `BOOK_EDIT`, `BEACON_EFFECT`, `ITEM_PICK`, `ENTITY_TELEPORT`, `PLAYER_ABILITIES` |
| No | `PLAYER_CONNECT`, `PLAYER_QUIT`, `PLAYER_RESPAWN`, `PLAYER_DAMAGE`, `PLAYER_TELEPORT`, `PLAYER_ITEM_DROP`, `PLAYER_ITEM_PICKUP`, `INVENTORY_OPEN`, `GAME_MODE_CHANGE` |

```mcfc
@Event(PLAYER_DAMAGE)
void onPlayerDamage(AgentEvent event) {
    event.player.tellraw("Damage event: $(event.payload)");
}
```

Agent events are tied to Minecraft 26.3. The agent sends events that arrive as network packets before the server acts on them, and those can be cancelled. Lifecycle events such as damage, quit and respawn are reported after they happen, so they can't be cancelled.

## Under the hood

Vanilla events are checked by the generated tick function. Agent events call `agent/event/<name>.mcfunction`, with the event name in lowercase. The agent writes the payload to command storage, and the wrapper copies it into the handler's parameter before calling it. `event.cancel()` writes `decision.cancel` to storage, and the agent reads it back before letting the action through.
