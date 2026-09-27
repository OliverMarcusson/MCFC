# Events

A function annotated with `@EventHandler` runs when something happens in game. Its one parameter's type picks the event, like in Bukkit: `void onJoin(PlayerJoinEvent event)` runs when a player joins. There are two kinds:

- **Vanilla events** work in any datapack.
- **Agent events** need the optional [`mcfd-agent`](/runtime/mcfd-agent) and `[helper.agent] enabled = true` in `mcfc.toml`. They carry a typed payload, and some can be cancelled. A pack that uses them still loads without the agent. Its agent handlers just never run.

## Vanilla events

| Event type | Runs |
| --- | --- |
| `PlayerJoinEvent` | Once per player, the first time the pack sees them. Tracked with the tag `mcfc_join_<namespace>`, so it doesn't run again on later logins. |
| `PlayerDeathEvent` | Each time a player dies, detected through a `deathCount` objective. |
| `BlockPlaceEvent` | When a player places a block. Has `block()`. |
| `PlayerItemConsumeEvent` | When a player finishes eating or drinking an item. |
| `PlayerUseItemEvent` | Every tick while a player uses an item, such as drawing a bow or eating. |
| `PlayerKillEntityEvent` | When a player kills an entity. |
| `PlayerHurtEntityEvent` | When a player hurts an entity, including with projectiles, or left-clicks an interaction entity. Has `entity()`. |
| `PlayerInteractEntityEvent` | When a player right-clicks an entity, including an interaction entity. Has `entity()`. |
| `EntityHurtPlayerEvent` | When an entity hurts a player, directly or with a projectile. Has `entity()`, the attacker. Damage with no attacker, such as falling, doesn't count. |

Vanilla handlers run as the affected player, which is `event.player()`:

```mcfc
@EventHandler
void onPlayerJoin(PlayerJoinEvent event) {
    Player player = event.player();
    player.sendMessage("Welcome!");
}

@EventHandler
void onPlayerDeath(PlayerDeathEvent event) {
    Player player = event.player();
    player.state.deaths = player.state.deaths + 1;
}
```

Minecraft doesn't say which entity or block was involved, so MCFC finds it after the fact. `entity()` is the entity this player hurt this tick for `PlayerHurtEntityEvent`, the attacker for `EntityHurtPlayerEvent` (the shooter for projectiles), and otherwise the first entity along the player's view within their `entity_interaction_range`. It is only valid inside the handler. `block()` is an `Optional<Block>`: the first block along the view within the player's `block_interaction_range`. It is empty or wrong when the placed block is replaceable, such as snow layers or tall grass, because the ray passes through those.

Interaction entities are invisible hitboxes. Summon one to make a clickable area, and handle clicks with `PlayerInteractEntityEvent` and `PlayerHurtEntityEvent`.

For something that runs repeatedly, use [`@Every`](./statements#every-and-after) or `void tick()`. For something players run, use [`@Command`](./statements#command).

## Agent events <Badge type="danger" text="Agent" title="Needs mcfd-agent running beside the server. Not available on Realms." />

These take their payload type as the parameter, the same way:

```mcfc
@EventHandler
void onChat(ChatEvent event) {
    if (event.message() == "spark") {
        event.cancel();
        event.player().sendMessage("Spark accepted");
    }
}
```

Every payload has `player(): Player` and `cancelled(): boolean`. Read payload values through accessor calls such as `event.message()`. On events marked cancellable, `event.cancel()` stops the action from happening. Calling it on any other event is a compile error.

### Typed payloads

| Event type | Other components | Cancellable |
| --- | --- | --- |
| `ChatEvent` | `message: String` | Yes |
| `BlockBreakEvent` | `x, y, z: int` | Yes |
| `PlayerInteractBlockEvent` | `hand: String`, `face: String`, `x, y, z: int` | Yes |
| `PlayerInteractItemEvent` | `hand: String` | Yes |
| `EntityInteractEvent` | `targetId: int`, `hand: String`, `secondary: boolean` | Yes |
| `EntityAttackEvent` | `targetId: int` | Yes |
| `PlayerActionEvent` | `action: String`, `face: String`, `x, y, z: int` | Yes |
| `PlayerActionToggleEvent` | `action: String`, `entityId: int`, `data: int` | Yes |
| `PlayerSwingEvent` | `hand: String` | Yes |
| `InventoryClickEvent` | `containerId, stateId, slot, button: int` | Yes |
| `InventoryCloseEvent` | `containerId: int` | Yes |
| `ItemHeldChangeEvent` | `slot: int` | Yes |
| `ItemRenameEvent` | `name: String` | Yes |
| `TradeSelectEvent` | `tradeIndex: int` | Yes |
| `SignChangeEvent` | `x, y, z: int`, `front: boolean`, `line1` to `line4: String` | Yes |
| `RecipePlaceEvent` | `containerId: int`, `recipe: String`, `useMaxItems: boolean` | Yes |
| `GameModeRequestEvent` | `mode: String` | Yes |

### Generic payloads

These payloads have `playerName(): String`, `source(): String` and `payload(): String` besides `player()` and `cancelled()`. `payload()` is the raw event data as text.

| Cancellable | Events |
| --- | --- |
| Yes | `PlayerRespawnRequestEvent`, `BookEditEvent`, `BeaconEffectEvent`, `ItemPickEvent`, `EntityTeleportEvent`, `PlayerAbilitiesEvent` |
| No | `PlayerConnectEvent`, `PlayerQuitEvent`, `PlayerRespawnEvent`, `PlayerDamageEvent`, `PlayerTeleportEvent`, `PlayerItemDropEvent`, `PlayerItemPickupEvent`, `InventoryOpenEvent`, `GameModeChangeEvent` |

```mcfc
@EventHandler
void onPlayerDamage(PlayerDamageEvent event) {
    event.player().sendMessage("Damage event: $(event.payload())");
}
```

Agent events are tied to Minecraft 26.3. The agent sends events that arrive as network packets before the server acts on them, and those can be cancelled. Lifecycle events such as damage, quit and respawn are reported after they happen, so they can't be cancelled.

## Under the hood

Join and death are checked by the generated tick function. The other vanilla events use an advancement per event, `advancement/mcfc_event/<name>.json`, whose reward function revokes it and calls the handler. Agent events call `agent/event/<name>.mcfunction`, where `<name>` is the event type in snake case without `Event`, such as `block_break`. The agent writes the payload to command storage, and the wrapper copies it into the handler's parameter before calling it. `event.cancel()` writes `decision.cancel` to storage, and the agent reads it back before letting the action through.
