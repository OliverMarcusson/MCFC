# Types

| Type | Holds | Stored in |
| --- | --- | --- |
| [`int`](#int) | 32-bit integer | scoreboard |
| [`float`](#float) | 32-bit decimal | storage |
| [`boolean`](#boolean) | `true` / `false` | scoreboard |
| [`String`](#string) | text | storage |
| [`List<T>`](#list) | ordered list | storage |
| [`Map<String, T>`](#map) | string-keyed map | storage |
| [`Optional<T>`](#optional) | a `T` or nothing | storage |
| record / enum types | declared with [`record`](./statements#record) or [`enum`](./statements#enum) | storage / scoreboard |
| [`Selector`](#entities) | a selector, which can match 0 or more entities | selector text |
| [`Entity`, `Player`](#entities) | one entity or player | selector text |
| [`Block`](#block) | a block position | position text |
| `EntityData`, `BlockData`, `ItemStack`, `Component` | builders, see [Builders](./builders) | storage |
| [`ItemSlot`](#itemslot) | an inventory or equipment slot | the entity |
| [`BossBar`](#bossbar) | a bossbar handle | storage |
| [`Nbt`](#nbt) | a raw NBT path or value | storage / the entity |
| `void` | no value, used only as a return type | |

## Rules

- `var` takes its type from the initializer; `T x = ...;` checks it. Parameters and return types are written out.
- An assignment has to keep the variable's type, and arguments have to match parameter types.
- Types never convert implicitly. `1.5 + 2` is an error, so write `1.5 + (float) 2`. The one exception: `EntityData`, `BlockData` and `ItemStack` can be used where an `Nbt` value is expected, which is short for `.asNbt()`.
- `+ - * / %` work on two `int`s or two `float`s. `+` also joins two `String`s.
- `< <= > >=` work on `int`, `float` and `boolean`. `== !=` work on all of these plus `String`.
- `&&`, `||` and `!` work on `boolean`.
- Precedence, from tightest to loosest: `!` and casts, then `* / %`, then `+ -`, then comparisons, then `&&`, then `||`.

## Casts

| Cast | From | Does |
| --- | --- | --- |
| `(int) x` | `float`, `Nbt` | Rounds a `float` down (`(int) -2.7` is `-3`); reads an NBT number |
| `(float) x` | `int`, `Nbt` | |
| `(boolean) x` | `Nbt` | |
| `(String) x` | `Nbt` | |
| `(Player) e` | `Entity` | Asserts that `e` is a player |

## `int`

`/` rounds down, and `%` takes the sign of the right side, the same as Minecraft's scoreboard `/=` and `%=`. So `-7 / 2` is `-4` and `-7 % 3` is `2`. Dividing by `0` leaves the left side unchanged. Overflow wraps.

`n.toString()` converts to text. `"42".parseInt()` goes the other way and returns `0` if the text isn't a whole number.

## `float`

```mcfc
float distance(float x, float z) {
    return (x * x + z * z).sqrt();
}
```

Literals need a digit on both sides of the point: `1.0`, `0.5`, `-2.5`. Floats are 32-bit, which gives about 7 significant digits.

| Method | Returns |
| --- | --- |
| `sqrt()`, `abs()` | |
| `sin()`, `cos()`, `tan()` | Take radians. Entity `yaw()` and `pitch()` are in degrees, so multiply by `0.017453292`. |
| `floor()`, `ceil()`, `round()`, `trunc()` | Rounded value, still a `float` |
| `pow(e)`, `min(y)`, `max(y)`, `clamp(low, high)`, `hypot(y)` | `hypot` is `sqrt(x*x + y*y)`. `pow` stops the command when both values are `0.0`. |
| `toString()` | `"0.5"`, `"-0.25"`, or `"4"` for a whole number |

Each float expression, however long, compiles to one `/compute` command. Comparisons cost two.

## `boolean`

`true` or `false`. It's stored as a scoreboard 1 or 0.

## `String`

Literals use `"..."`. `$(expr)` inside a literal inserts a value.

```mcfc
void main() {
    var name = "Steve";
    var score = 42;
    var line = "Hi " + name + ", you have " + score.toString() + " points";
    var same = "Hi $(name), you have $(score) points";
}
```

| Method | Returns |
| --- | --- |
| `length()` | Number of characters |
| `substring(start)`, `substring(start, end)` | Substring. `end` is excluded, and a negative index counts from the end. Returns `""` when out of range. |
| `parseInt()` | The number, or `0` |
| `toString()` | Also available on `int` and `float` |

For `startsWith`, `endsWith`, `find` and `contains`, see [`std.str`](./std#std-str).

**Limits.** Joining, `toString()` and `$(...)` go through a Minecraft macro, which pastes the value in without escaping it. As a result:

- A value containing `"` breaks the command, and the result is `""`.
- A `\` is read as an escape, so `\n` becomes a newline.

Player names and ids never contain these characters. Text players type, such as chat, item names and signs, can.

## `List<T>` {#list}

```mcfc
void main() {
    var values = List.of(3, 5);
    values.add(9);
    var third = values.get(2).orElse(0);
    for (var v : values) {
        debug("$(v)");
    }
}
```

| Method | Returns |
| --- | --- |
| `size()` | Number of elements |
| `xs[i]` | The element at `i` |
| `get(i)` | `Optional<T>`, empty if there's no element at `i` |
| `getFirst()`, `getLast()` | The first or last element. An empty list gives the type's empty value, such as `0`. |
| `contains(v)`, `indexOf(v)` | `boolean`, and the index of `v` or `-1` |
| `add(v)`, `add(i, v)`, `removeLast()`, `remove(i)`, `clear()` | Change the list. `removeLast` and `remove` return the removed element. |
| `reverse()`, `sort()` | In place. `sort` works on `List<int>` and `List<float>`, smallest first. |

`List.of()` with no elements needs a declared type: `List<int> xs = List.of();`. Methods that change the list need a variable or element (`teams["red"]`), not a function result.

`contains`, `indexOf` and `reverse` loop over every element. `sort` is a merge sort that does at most 1,000 steps per tick. Small lists finish immediately. Larger ones [pause](./statements#functions-that-pause) the function: 5,000 elements take about 3 seconds.

## `Map<String, T>` {#map}

```mcfc
void main() {
    var counts = Map.of("wood", 2, "stone", 4);
    counts["iron"] = 1;
    var gold = counts.get("gold").orElse(0);
    for (var key : counts.keySet()) {
        debug("$(key)=$(counts[key])");
    }
}
```

`Map.of` takes key, value pairs, and the keys must be string literals. Keys are always `String`.

| Method | Returns |
| --- | --- |
| `m[key]` | The value for `key` |
| `get(key)` | `Optional<T>` |
| `containsKey(key)` | `boolean` |
| `remove(key)` | Removes `key` |
| `size()` | Number of keys |
| `keySet()` | `List<String>`, in storage order. Returns an empty list if any string value in the map contains `'` or `"`. |

Keys may only use letters, digits and `_`, and can't start with a digit.

## `Optional<T>` {#optional}

This is what `List.get`, `Map.get` and [`findFirst`](./builtins#selecting-entities) return.

```mcfc
void main() {
    var maybe = List.of(4, 8).get(3);
    if (maybe.isPresent()) {
        debug("found");
    }
    var count = maybe.orElse(0);
}
```

| Method | Returns |
| --- | --- |
| `isPresent()` | `boolean` |
| `orElse(fallback)` | The value, or `fallback` when empty. `fallback` is always evaluated, even when a value is present. |

`Optional<void>` isn't allowed.

## Entities

- `Selector` can match any number of entities. Loop over it with `for`.
- `Entity` is one entity. Get one with `single(Selector.of(...))`, or from a `for` loop over a `Selector`.
- `Player` is an `Entity` that's known to be a player.

Some methods only work on players, and `heal` only works on non-players. The compiler works out which kind a reference is from its selector:

| Selector | Known as |
| --- | --- |
| `@p`, `@a`, `@r`, `@s`, a player name, or `type=player` | player |
| any other `type=...` | non-player |
| anything else | unknown |

`(Player) e` asserts that `e` is a player, and `for (Player p : Selector.of(...))` does the same for a loop. All entity methods and fields are listed in [Entities and Players](./methods).

## `Block`

A block position, created with `Block.of("~ ~ ~")` or read from `entity.position`. `Block.of(...)` needs a literal string, so a position can't be computed at run time yet. Relative coordinates are resolved where the code runs. To anchor them to an entity, use `at(player, Block.of("~1 ~ ~"))` or an [`at` block](./statements#as-and-at).

| Method | Does |
| --- | --- |
| `setblock(id \| BlockData)` | Places a block. Placing a `BlockData` also writes its NBT. |
| `fill(to: Block, id \| BlockData)` | Fills the box between two positions (block id and states only) |
| `is(id) -> boolean` | Tests the block at this position |
| `summon(id)`, `summon(id, Nbt)`, `summon(EntityData) -> Entity` | Summons at this position |
| `spawnItem(ItemStack) -> Entity` | Drops an item stack |
| `particle(name)`, `particle(name, count)`, `particle(name, count, viewers)` | |
| `lootInsert(table)`, `lootSpawn(table)` | Inserts loot into the container, or spawns it in the world |
| `debugMarker(label)`, `debugMarker(label, block)` | Places a visible marker for debugging |
| `nbt.*` | Block-entity NBT |
| `light() -> int` | Light level 0 to 15 |
| `biome() -> String`, `inBiome(id) -> boolean` | `inBiome` accepts a `#tag` |
| `environment(attribute) -> float` | A numeric environment attribute, such as `"gameplay/sky_light_level"`. The id must be a literal. |

```mcfc
void markGround() {
    var below = Block.of("~ ~-1 ~");
    if (below.is("minecraft:grass_block") && below.light() < 8) {
        below.setblock("minecraft:glowstone");
    }
}
```

Environment attributes: `visual/cloud_height`, `visual/fog_start_distance`, `visual/fog_end_distance`, `visual/sky_fog_end_distance`, `visual/cloud_fog_end_distance`, `visual/water_fog_start_distance`, `visual/water_fog_end_distance`, `visual/moon_angle`, `visual/star_angle`, `visual/sun_angle`, `visual/sky_light_factor`, `visual/star_brightness`, `audio/music_volume`, `gameplay/cat_waking_up_gift_chance`, `gameplay/creature_world_gen_spawn_probability`, `gameplay/surface_slime_spawn_chance`, `gameplay/turtle_egg_hatch_chance`, `gameplay/sky_light_level`.

## `ItemSlot`

`player.inventory[0..26]`, `player.hotbar[0..8]` and the equipment slots `mainhand`, `offhand`, `head`, `chest`, `legs` and `feet`.

| Field | Type |
| --- | --- |
| `exists` | `boolean`, read-only |
| `id` | `String`, read-only |
| `count` | `int` |
| `name` | `String` |
| `Nbt` | `Nbt` |

```mcfc
void equip(Player player) {
    var sword = new ItemStack("minecraft:diamond_sword");
    sword.name = "Quest Blade";
    player.hotbar[0] = sword;
    player.head.item = "minecraft:golden_helmet";
    if (player.inventory[3].exists) {
        player.tellraw(player.inventory[3].id);
    }
    player.inventory[4].clear();
}
```

Assign an `ItemStack` to an inventory or hotbar slot to set it. The slot index can be a variable. `clear()` empties the slot. For equipment slots, write `.item` (an id or `ItemStack`), `.name` or `.count`. `inventory` and `hotbar` only work on players.

## `BossBar`

```mcfc
void show() {
    var bb = new BossBar("mypack:progress", "Progress");
    bb.max = 10;
    bb.value = 5;
    bb.players = Selector.of("@a");
    bb.visible = true;
}
```

The fields are `name` (a `String` or `Component`), `value`, `max`, `visible` and `players`. `bb.remove()` deletes the bossbar. `new BossBar(id, ...)` with an existing id gives you that bossbar.

## `Nbt`

A raw NBT path such as `player.nbt.Health` or `pig.nbt.Tags[0]`. Convert it with `(int)`, `(float)`, `(boolean)` or `(String)`, and check whether it exists with `hasData(...)`. Player NBT is read-only.
