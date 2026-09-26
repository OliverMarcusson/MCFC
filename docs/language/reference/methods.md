# Entities and Players

Methods and fields on `Entity` and `Player`. `teleport`, `damage`, `give`, `clear`, `lootGive`, the message methods and the sound methods also work on a `Selector`, such as `Selector.of("@a").title("Go")`, and apply to every match. Commands target the reference's selector, run inside `execute as` / `execute at` when the context requires it. For how a reference is classified as a player or a non-player, see [Types: Entities](./types#entities).

## Actions

| Method | Does | Command |
| --- | --- | --- |
| `teleport(to: Entity \| Block)` | Moves the entity | `teleport` |
| `damage(amount: int)` | Deals damage | `damage` |
| `heal(amount: int)` | Restores health. Non-player references only. | NBT write |
| `effect(id, seconds: int, amplifier: int)` | Applies a status effect | `effect give` |
| `give(id, count: int)`, `give(ItemStack)` | Gives items | `give` |
| `clear(id, count: int)` | Removes items | `clear` |
| `lootGive(table)` | Gives loot from a loot table | `loot give` |
| `addTag(name)`, `removeTag(name)` | Adds or removes a scoreboard tag | `tag` |
| `hasTag(name) -> boolean` | Tests for a tag | |
| `debugEntity(label)` | Makes the entity glow for 3 seconds | `effect give ... glowing` |

## Messages and sound

| Method | Command |
| --- | --- |
| `tellraw(msg)` | `tellraw` |
| `title(msg)` | `title ... title` |
| `actionbar(msg)` | `title ... actionbar` |
| `playsound(sound, category)` | `playsound` |
| `stopsound(category, sound)` | `stopsound` |

`msg` is a `String` or a [`Component`](./builders#text-builders) for formatted text. Strings can use `$(...)` to insert values.

```mcfc
void celebrate(Player player) {
    var message = new Component("Quest complete");
    message.color = "gold";
    message.bold = true;
    player.tellraw(message);
    player.title("Victory");
    player.playsound("minecraft:entity.player.levelup", "master");
}
```

## Fields

| Field | Type | Notes |
| --- | --- | --- |
| `position` | `Block` | Read-only. It's the block the entity is standing in. |
| `state.*` | declared type | See [`@PlayerState`](./statements#playerstate) and [`@EntityState`](./statements#entitystate) |
| `tags.<name>` | `boolean` | Read or write a tag as a boolean. Players only. |
| `team` | `String` | Write-only. Joins a team. |
| `nbt.*` | `Nbt` | Entity NBT. Player NBT is read-only. |
| `mainhand`, `offhand`, `head`, `chest`, `legs`, `feet` | [`ItemSlot`](./types#itemslot) | Equipment |
| `inventory[0..26]`, `hotbar[0..8]` | [`ItemSlot`](./types#itemslot) | Players only |

## Reading values

Each call reads the entity's NBT again, so store the result in a `var` if you need it more than once.

| Method | Returns |
| --- | --- |
| `x()`, `y()`, `z()` | `float` position |
| `yaw()`, `pitch()` | `float`, in degrees |
| `lookX()`, `lookY()`, `lookZ()` | `float`, the unit vector the entity is facing |
| `health()` | `float` |
| `distanceTo(other: Entity)` | `float` |
| `food()` | `int`, 0 to 20. Players only. |
| `xpLevel()` | `int`. Players only. |
| `gameMode()` | `int`: 0 survival, 1 creative, 2 adventure, 3 spectator. Players only. |
| `selectedSlot()` | `int`, 0 to 8. Players only. |
| `dimension()` | `String`, such as `"minecraft:overworld"`. Players only. |

```mcfc
void main() {
    var player = single(Selector.of("@p"));
    var pig = single(Selector.of("@e[type=minecraft:pig,limit=1]"));
    if (player.distanceTo(pig) < 8.0 && player.food() < 6) {
        player.tellraw("The pig looks tasty");
        pig.heal(2);
    }
}
```
