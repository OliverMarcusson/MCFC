# Entities and Players

Methods and fields on `entity_ref` and `player_ref`. `teleport`, `damage`, `give`, `clear`, `loot_give`, the message methods and the sound methods also work on an `entity_set`, such as `selector("@a").title("Go")`, and apply to every match. Commands target the reference's selector, run inside `execute as` / `execute at` when the context requires it. For how a reference is classified as a player or a non-player, see [Types: Entities](./types#entities).

## Actions

| Method | Does | Command |
| --- | --- | --- |
| `teleport(to: entity_ref \| block_ref)` | Moves the entity | `teleport` |
| `damage(amount: int)` | Deals damage | `damage` |
| `heal(amount: int)` | Restores health. Non-player references only. | NBT write |
| `effect(id, seconds: int, amplifier: int)` | Applies a status effect | `effect give` |
| `give(id, count: int)`, `give(item_def)` | Gives items | `give` |
| `clear(id, count: int)` | Removes items | `clear` |
| `loot_give(table)` | Gives loot from a loot table | `loot give` |
| `add_tag(name)`, `remove_tag(name)` | Adds or removes a scoreboard tag | `tag` |
| `has_tag(name) -> bool` | Tests for a tag | |
| `debug_entity(label)` | Makes the entity glow for 3 seconds | `effect give ... glowing` |

## Messages and sound

| Method | Command |
| --- | --- |
| `tellraw(msg)` | `tellraw` |
| `title(msg)` | `title ... title` |
| `actionbar(msg)` | `title ... actionbar` |
| `playsound(sound, category)` | `playsound` |
| `stopsound(category, sound)` | `stopsound` |

`msg` is a `string` or a [`text_def`](./builders#text-builders) for formatted text. Strings can use `$(...)` to insert values.

```mcfc
fn celebrate(player: player_ref) -> void:
    let message = text("Quest complete")
    message.color = "gold"
    message.bold = true
    player.tellraw(message)
    player.title("Victory")
    player.playsound("minecraft:entity.player.levelup", "master")
```

## Fields

| Field | Type | Notes |
| --- | --- | --- |
| `position` | `block_ref` | Read-only. It's the block the entity is standing in. |
| `state.*` | declared type | See [`player_state`](./statements#player-state) and [`entity_state`](./statements#entity-state) |
| `tags.<name>` | `bool` | Read or write a tag as a boolean. Players only. |
| `team` | `string` | Write-only. Joins a team. |
| `nbt.*` | `nbt` | Entity NBT. Player NBT is read-only. |
| `mainhand`, `offhand`, `head`, `chest`, `legs`, `feet` | [`item_slot`](./types#item-slot) | Equipment |
| `inventory[0..26]`, `hotbar[0..8]` | [`item_slot`](./types#item-slot) | Players only |

## Reading values

Each call reads the entity's NBT again, so store the result in a `let` if you need it more than once.

| Method | Returns |
| --- | --- |
| `x()`, `y()`, `z()` | `float` position |
| `yaw()`, `pitch()` | `float`, in degrees |
| `look_x()`, `look_y()`, `look_z()` | `float`, the unit vector the entity is facing |
| `health()` | `float` |
| `distance_to(other: entity_ref)` | `float` |
| `food()` | `int`, 0 to 20. Players only. |
| `xp_level()` | `int`. Players only. |
| `game_mode()` | `int`: 0 survival, 1 creative, 2 adventure, 3 spectator. Players only. |
| `selected_slot()` | `int`, 0 to 8. Players only. |
| `dimension()` | `string`, such as `"minecraft:overworld"`. Players only. |

```mcfc
fn main() -> void:
    let player = single(selector("@p"))
    let pig = single(selector("@e[type=minecraft:pig,limit=1]"))
    if player.distance_to(pig) < 8.0 and player.food() < 6:
        player.tellraw("The pig looks tasty")
        pig.heal(2)
```
