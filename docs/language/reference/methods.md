# Entities and Players

Methods and fields on `Entity` and `Player`. `teleport`, `remove`, `damage`, `give`, `clear`, `lootGive`, `effect`, `addTag`, `removeTag`, `setGameMode`, `spectate`, `stopSpectating`, the experience methods, the message methods and the sound methods also work on a `Selector`, such as `Selector.of("@a").sendTitle("Go")`, and apply to every match. Commands target the reference's selector, run inside `execute as` / `execute at` when the context requires it. For how a reference is classified as a player or a non-player, see [Types: Entities](./types#entities).

## Selecting and checking entities

| Method | Returns | Notes |
| --- | --- | --- |
| `Selector.getFirst()` | `Entity` | Narrows a selector that matches one entity, such as `@p`, `@s`, or `limit=1`. |
| `Selector.findFirst()` | `Optional<Entity>` | Needs a selector built in the same expression; the compiler adds `limit=1`. |
| `Selector.count()` | `int` | How many entities match. |
| `Selector.exists()` / `Selector.isEmpty()` | `boolean` | Whether anything matches. |
| `Entity.matches(selector)` | `boolean` | Whether the entity passes the selector's filters. The selector must be `@a`, `@e` or `@s`, without `limit` or `sort`. |
| `Entity.isValid()` | `boolean` | Whether the entity still exists. |

To build selectors with methods, such as `Selector.entities().tag("boss").limit(1)`, see [Types: Building selectors](./types#building-selectors).

## Actions

| Method | Does | Command |
| --- | --- | --- |
| `teleport(to: Entity \| Block)` | Moves the entity. A `Block` centres it on the block. | `teleport` |
| `teleport(pos: Vec3, yaw: float, pitch: float)` | Moves to an exact position, facing yaw and pitch in degrees. | `teleport` |
| `remove()` | Removes the entity from the world. Players die instead. | `kill` |
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
| `setOwner(owner: Entity)` | Links this entity to an owner. Works on any entity. | `mcfc_id` / `mcfc_owner` scores |
| `getOwner()` | `Optional<Entity>`, the `setOwner` owner, or else the vanilla owner of a tamed animal or projectile. The result is valid until this line runs again. | Score scan, then `execute on owner` |
| `getTargetBlock(maxDistance: float)` | `Optional<Block>`, the first block along the view that isn't replaceable (air, water, grass and similar). | Generated 0.1-block step function |
| `getTargetEntity(maxDistance: float)` | `Optional<Entity>`, the first entity along the view. Blocks stop the ray. | Generated 0.1-block step function |
| `effect(id, seconds: int, amplifier: int)` | Applies a status effect | `effect give` |
| `give(id, count: int)`, `give(ItemStack)` | Gives items | `give` |
| `clear(id, count: int)` | Removes items | `clear` |
| `lootGive(table)` | Gives loot from a loot table | `loot give` |
| `addTag(name)`, `removeTag(name)` | Adds or removes a scoreboard tag | `tag` |
| `spectate(camera: Entity)` | Views the world from `camera`, as a spectator does. Players in spectator mode only. | `spectate` |
| `stopSpectating()` | Returns the view to the player. | `spectate` |
| `setGameMode(mode: GameMode)` | `GameMode.SURVIVAL`, `CREATIVE`, `ADVENTURE` or `SPECTATOR`, from `import std.gamemode.GameMode;`. | `gamemode` |
| `setLevel(levels: int)` | Sets the experience level | `xp set` |
| `giveExpLevels(levels: int)`, `giveExp(points: int)` | Adds levels, or points that fill the bar. Negative numbers remove. | `xp add` |
| `countItem(id) -> int` | How many of an item the player carries. Players only; see [`std.inventory`](./std#std-inventory) for costs. | `clear ... 0` |
| `hasTag(name) -> boolean` | Tests for a tag | |
| `debugEntity(label)` | Makes the entity glow for 3 seconds | `effect give ... glowing` |

## Messages and sound

| Method | Command |
| --- | --- |
| `sendMessage(msg)` | `tellraw` |
| `sendTitle(msg)` | `title ... title` |
| `sendTitle(title, subtitle)`, `sendTitle(title, subtitle, fadeIn, stay, fadeOut)` | `title ... subtitle`, `title ... times`; times in ticks. `subtitle` is a `String`. |
| `sendActionBar(msg)`, `sendActionBar(msg, priority)` | `title ... actionbar`, [coordinated](#action-bar-priorities) |
| `playSound(sound, category)` | `playsound` |
| `stopSound(category, sound)` | `stopsound` |

`msg` is a `String` or a [`Component`](./builders#text-builders) for formatted text. Strings can use `$(...)` to insert values.

### Action bar priorities

Packs share one action bar, so `sendActionBar` follows the [Smithed Actionbar](https://docs.smithed.dev/libraries/actionbar/) priorities: `"override"`, `"notification"` (the default), `"conditional"` for a HUD shown while something is true, and `"persistent"` for one that is always on. A shown message stays for 20 ticks, and until then only a message of the same or higher priority replaces it. An `"override"` can't be replaced.

```mcfc
class Main {
    static void hud(Player player) {
        player.sendActionBar("Holding a compass", "conditional");
    }
}
```

When the Smithed Actionbar pack is installed, messages go through it. Without it, MCFC runs the same rules on the same scoreboards, so MCFC packs still coordinate with each other.

```mcfc
class Main {
    static void celebrate(Player player) {
        var message = new Component("Quest complete");
        message.color = "gold";
        message.bold = true;
        player.sendMessage(message);
        player.sendTitle("Victory");
        player.playSound("minecraft:entity.player.levelup", "master");
    }
}
```

## Sidebar

`Sidebar` is the scoreboard sidebar every player sees. Lines are numbered from 0 at the top.

```mcfc
class Main {
    static void showScore(Player player) {
        Sidebar.setTitle("Arena");
        Sidebar.setLine(0, "Red: 3");
        Sidebar.setLine(1, "Blue: 5");
        Sidebar.setLine(2, "Time", "4:30");
        Sidebar.removeLine(3);
        player.setSidebarLine(4, "Your coins: 12");
    }
}
```

| Call | Does |
| --- | --- |
| `Sidebar.setTitle(text)` | Sets the title. `text` is a `String` or `Component`. |
| `Sidebar.setLine(line, text)` | Sets or replaces a line. |
| `Sidebar.setLine(line, text, value)` | The same, with `value` right-aligned at the line's end, where a score would show. `value` is a `String` or `Component`. |
| `Sidebar.removeLine(line)` | Removes a line. |
| `Sidebar.clear()` | Removes every line. |

Players also have `setSidebarTitle(text)`, `setSidebarLine(line, text)`, `removeSidebarLine(line)` and `clearSidebar()` for a sidebar only that player sees. <Badge type="danger" text="Agent" title="Needs mcfd-agent running beside the server. Not available on Realms." /> Per-player sidebars need [`mcfd-agent`](/runtime/mcfd-agent) and `[helper.agent] enabled = true`; without it, the same calls change the shared `Sidebar`. A player's sidebar replaces the shared one on their screen and comes back when they rejoin. `clearSidebar()` removes it. `text` is a `String` or a [`Component`](./builders#text-builders) for colors and styles.

The shared sidebar is the `mcfc_sidebar` objective with a blank number format. Line `n` is the fake player `mcfc.line.n` with score `-n`. It's displayed when the pack loads, so another pack displaying its own sidebar objective replaces it.

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
| `getGameMode()` | `GameMode`. Players only. |
| `getSelectedSlot()` | `int`, 0 to 8. Players only. |
| `getDimension()` | `String`, such as `"minecraft:overworld"`. Players only. |

```mcfc
class Main {
    public static void main() {
        var player = Selector.of("@p").getFirst();
        var pig = Selector.of("@e[type=minecraft:pig,limit=1]").getFirst();
        if (player.distanceTo(pig) < 8.0 && player.getFoodLevel() < 6) {
            player.sendMessage("The pig looks tasty");
            pig.heal(2);
        }
    }
}
```

## Display entities

Block, item and text displays are animated by changing their transformation with an interpolation duration set. Changes made in the same tick count as one update, so set the duration and the new transform together:

```mcfc
import std.vec.Vec3;

class Main {
    public static void main() {
        var display = Selector.of("@e[type=minecraft:block_display,limit=1]").getFirst();
        display.setInterpolationDuration(20);
        display.setInterpolationDelay(0);
        display.setScale(new Vec3(2.0, 2.0, 2.0));
        display.setLeftRotation(3.14159, new Vec3(0.0, 1.0, 0.0));

        // Or in one call: grow back to normal size over 40 ticks.
        display.animate(40, new Vec3(0.0, 0.0, 0.0), new Vec3(1.0, 1.0, 1.0));
    }
}
```

| Method | Effect |
| --- | --- |
| `setTranslation(Vec3)`, `setScale(Vec3)` | Sets `transformation.translation` or `transformation.scale`. |
| `setLeftRotation(angle: float, axis: Vec3)` | Sets `transformation.left_rotation`, `angle` radians around `axis`. |
| `setInterpolationDuration(ticks: int)` | How long the next transform change takes. Set it in the same tick as the change. |
| `setInterpolationDelay(ticks: int)` | Ticks to wait before interpolating (`start_interpolation`). |
| `setTeleportDuration(ticks: int)` | Smooths `teleport` over up to 59 ticks. |
| `animate(ticks: int, translation: Vec3, scale: Vec3)` | Sets translation, scale, duration, and a delay of 0 in one call. |

`Vec3` comes from [`std.vec`](./std#std-vec). These methods need a display entity and are errors on a `Player`.

## Movement, attributes, and input

```mcfc
import std.attribute.Attribute;

class Main {
    public static void main() {
        var player = (Player) Selector.of("@p").getFirst();
        var pig = Selector.of("@e[type=minecraft:pig,limit=1]").getFirst();
        player.setAttribute(Attribute.GRAVITY, 0.04);
        if (player.getCurrentInput().isJump()) {
            pig.setVelocity(0.0, 0.5, 0.0);
            player.lookAt(pig);
        }
    }
}
```

`Attribute` is imported from `std.attribute`. It defines `MOVEMENT_SPEED`, `JUMP_STRENGTH`, `GRAVITY`, `STEP_HEIGHT`, `SCALE`, `SAFE_FALL_DISTANCE`, and `KNOCKBACK_RESISTANCE`. Pass a string ID for other attributes. `getAttribute` reads the effective value; `setAttribute` changes the base value, so modifiers can make a later read differ.

Players ignore `Motion` writes, so `addVelocity` briefly equips an enchanted saddle and swaps game mode to fire `apply_impulse`. It does not apply to mounted or spectator players, and it uses the saddle slot. To push a player along their view, scale `getLookX()`, `getLookY()`, and `getLookZ()`. On players, `setHealth` lands on the next player tick and the max health cap lasts two ticks. `setFoodLevel` uses saturation or hunger until the requested level is observed; it can take several ticks and can interfere with existing hunger effects. The target is clamped to 0–20.
