# Entities and Players

Methods and fields on `Entity` and `Player`. `teleport`, `damage`, `give`, `clear`, `lootGive`, the message methods and the sound methods also work on a `Selector`, such as `Selector.of("@a").sendTitle("Go")`, and apply to every match. Commands target the reference's selector, run inside `execute as` / `execute at` when the context requires it. For how a reference is classified as a player or a non-player, see [Types: Entities](./types#entities).

## Selecting and checking entities

| Method | Returns | Notes |
| --- | --- | --- |
| `Selector.getFirst()` | `Entity` | Narrows a selector that matches one entity, such as `@p`, `@s`, or `limit=1`. |
| `Selector.findFirst()` | `Optional<Entity>` | Needs a literal `Selector.of(...)`; the compiler adds `limit=1`. |
| `Entity.isValid()` | `boolean` | Whether the entity still exists. |

## Actions

| Method | Does | Command |
| --- | --- | --- |
| `teleport(to: Entity \| Block)` | Moves the entity | `teleport` |
| `damage(amount: int)` | Deals damage | `damage` |
| `heal(amount: int)` | Restores health. Non-player references only. | NBT write |
| `setVelocity(x: float, y: float, z: float)` | Replaces `Motion` on a known non-player entity. | NBT write |
| `addVelocity(x: float, y: float, z: float)` | Adds world-space motion in blocks per tick. | `Motion` NBT, or a generated `apply_impulse` enchantment for players |
| `setHealth(points: float)` | Sets health, capped at max health. Zero or less kills. | `Health` NBT, or a max-health cap and instant heal for players |
| `setFoodLevel(level: int)` | Moves a player's food level toward `level` over ticks. | Status effects and a generated tick function |
| `getAttribute(id: String \| Attribute)` | Reads an entity's effective attribute value as `float`, to 0.001 precision. | `attribute ... get` |
| `setAttribute(id: String \| Attribute, value: float)` | Sets an entity's base attribute value. | `attribute ... base set` |
| `setRotation(yaw: float, pitch: float)` | Sets rotation in degrees. | `rotate` |
| `lookAt(target: Entity \| Block)` | Rotates to face a target's feet or a block position. | `rotate ... facing` |
| `yawTo(target: Entity \| Block)`, `pitchTo(target: Entity \| Block)` | Reads the facing angle toward a target. | Temporary marker and `Rotation` NBT |
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
| `sendMessage(msg)` | `tellraw` |
| `sendTitle(msg)` | `title ... title` |
| `sendActionBar(msg)` | `title ... actionbar` |
| `playSound(sound, category)` | `playsound` |
| `stopSound(category, sound)` | `stopsound` |

`msg` is a `String` or a [`Component`](./builders#text-builders) for formatted text. Strings can use `$(...)` to insert values.

```mcfc
void celebrate(Player player) {
    var message = new Component("Quest complete");
    message.color = "gold";
    message.bold = true;
    player.sendMessage(message);
    player.sendTitle("Victory");
    player.playSound("minecraft:entity.player.levelup", "master");
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
| `getX()`, `getY()`, `getZ()` | `float` position |
| `getYaw()`, `getPitch()` | `float`, in degrees |
| `getLookX()`, `getLookY()`, `getLookZ()` | `float`, the unit vector the entity is facing |
| `getHealth()` | `float` |
| `getCurrentInput().isForward()`, `isBackward()`, `isLeft()`, `isRight()`, `isJump()`, `isSneak()`, `isSprint()` | `boolean`, current movement key state. Players only. |
| `distanceTo(other: Entity)` | `float` |
| `getFoodLevel()` | `int`, 0 to 20. Players only. |
| `getLevel()` | `int`. Players only. |
| `getGameMode()` | `int`: 0 survival, 1 creative, 2 adventure, 3 spectator. Players only. |
| `getSelectedSlot()` | `int`, 0 to 8. Players only. |
| `getDimension()` | `String`, such as `"minecraft:overworld"`. Players only. |

```mcfc
void main() {
    var player = Selector.of("@p").getFirst();
    var pig = Selector.of("@e[type=minecraft:pig,limit=1]").getFirst();
    if (player.distanceTo(pig) < 8.0 && player.getFoodLevel() < 6) {
        player.sendMessage("The pig looks tasty");
        pig.heal(2);
    }
}
```

## Movement, attributes, and input

```mcfc
import std.attribute.Attribute;

void main() {
    var player = (Player) Selector.of("@p").getFirst();
    var pig = Selector.of("@e[type=minecraft:pig,limit=1]").getFirst();
    player.setAttribute(Attribute.GRAVITY, 0.04);
    if (player.getCurrentInput().isJump()) {
        pig.setVelocity(0.0, 0.5, 0.0);
        player.lookAt(pig);
    }
}
```

`Attribute` is imported from `std.attribute`. It defines `MOVEMENT_SPEED`, `JUMP_STRENGTH`, `GRAVITY`, `STEP_HEIGHT`, `SCALE`, `SAFE_FALL_DISTANCE`, and `KNOCKBACK_RESISTANCE`. Pass a string ID for other attributes. `getAttribute` reads the effective value; `setAttribute` changes the base value, so modifiers can make a later read differ.

Players ignore `Motion` writes, so `addVelocity` briefly equips an enchanted saddle and swaps game mode to fire `apply_impulse`. It does not apply to mounted or spectator players, and it uses the saddle slot. To push a player along their view, scale `getLookX()`, `getLookY()`, and `getLookZ()`. On players, `setHealth` lands on the next player tick and the max health cap lasts two ticks. `setFoodLevel` uses saturation or hunger until the requested level is observed; it can take several ticks and can interfere with existing hunger effects. The target is clamped to 0–20.
