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
- An `int` widens to `float` when a float is expected, including in arithmetic. `EntityData`, `BlockData` and `ItemStack` can be used where an `Nbt` value is expected, which is short for `.asNbt()`.
- `+ - * / %` work on numbers; a mixed `int` and `float` expression has type `float`. `+` also joins a `String` with an `int`, `float`, `boolean` or enum.
- `< <= > >=` work on `int`, `float` and `boolean`. `== !=` work on all of these plus `String`.
- `&&`, `||` and `!` work on `boolean`.
- Precedence, from tightest to loosest: `!` and casts, then `* / %`, then `+ -`, then comparisons, then `&&`, then `||`.
- `Integer`, `Float` and `Boolean` are accepted as type names. Use the boxed spellings inside `<...>`, such as `List<Integer>`.

## Casts

| Cast | From | Does |
| --- | --- | --- |
| `(int) x` | `float`, `Nbt` | Rounds a `float` down (`(int) -2.7` is `-3`); reads an NBT number |
| `(float) x` | `int`, `Nbt` | |
| `(boolean) x` | `Nbt` | |
| `(String) x` | `Nbt` | |
| `(Player) e` | `Entity` | Asserts that `e` is a player |

## `int`

Integer `/` rounds down, and `%` takes the sign of the divisor, the same as Minecraft's scoreboard `/=` and `%=`. So `-7 / 2` is `-4` and `-7 % 3` is `2`. Casting a float with `(int)` also floors: `(int) -2.7` is `-3`. Dividing by `0` leaves the left side unchanged. Overflow wraps.

`&`, `|`, `^`, `<<` and `>>` work like Java's on 32-bit ints, with Java's precedence, and the shift count is taken modulo 32. `>>` keeps the sign. There's no `>>>` or `~`; use `-x - 1` for `~x`. Scoreboards have no bit operations, so `&`, `|` and `^` run a generated loop of up to 32 steps, and `<<` and `>>` multiply or divide by a power of two.

`n.toString()` converts to text. `Integer.parseInt("42")` goes the other way and returns `0` if the text isn't a whole number.

## `float`

```mcfc
float distance(float x, float z) {
    return Math.sqrt(x * x + z * z);
}
```

Literals need a digit on both sides of the point: `1.0`, `0.5`, `-2.5`. A trailing `f` is accepted (`1.5f`, `2f`), and underscores can separate digits (`1_000`, `0xFF`). Floats are 32-bit, which gives about 7 significant digits.

Numeric functions are called through `Math`. `int` arguments widen to `float` when the function needs a float. For `min`, `max`, `abs`, `clamp` and `sign`, use [`std.math`](./std#std-math).

| Call | Returns / notes |
| --- | --- |
| `Math.sqrt(x)`, `Math.pow(x, e)`, `Math.hypot(x, y)` | `float`; `pow` stops the command when both inputs are `0.0`. |
| `Math.sin(x)`, `Math.cos(x)`, `Math.tan(x)` | `float`, taking radians. Entity yaw and pitch are degrees; multiply by `0.017453292`. Same as [`std.math`](./std#std-math) `sin`/`cos`/`tan`. |
| `Math.atan(x)`, `Math.atan2(y, x)`, `Math.asin(x)`, `Math.acos(x)` | `float` radians, accurate to about `1e-6`. They call [`std.math`](./std#std-math), since `/compute` has no inverse trig, so they cost a function call and don't fuse into the expression. |
| `Math.floor(x)`, `Math.ceil(x)`, `Math.trunc(x)` | Rounded `float`. |
| `Math.round(x)` | `int`. |
| `x.toString()` | `"0.5"`, `"-0.25"`, or `"4"` for a whole number. Also available on `int`. |

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
| `equals(other)` | `boolean`, the same comparison as `==` |
| `contains(part)`, `startsWith(prefix)`, `endsWith(suffix)` | `boolean` |
| `indexOf(part)` | The first index, or `-1` |
| `charAt(index)` | One-character `String`; MCFC has no `char` type. |
| `isEmpty()` | `boolean` |
| `replace(target, replacement)` | Every `target` replaced. `target` is plain text. |
| `split(separator)` | `List<String>`. `separator` is plain text, not a regex. Like Java, trailing empty parts are dropped. |
| `toUpperCase()`, `toLowerCase()` | Changes ASCII letters only. |
| `toString()` | The same string |

`String.join(separator, parts)` joins a `List<String>` with `separator` between each part, like Java: `String.join(", ", names)`.

`Integer.parseInt(s)` returns a number or `0` for invalid text. `String.valueOf(x)`, `Integer.toString(x)` and `Float.toString(x)` convert values to text.

**Limits.** Joining strings is safe for any text, including `"`, `\` and newlines. `$(...)` in `mcf(...)` pastes the value into the command as it is, so text players type (chat, item names, signs) can change what the command does. Show such values through a [`Component`](./builders) instead.

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
| `set(i, v)` | Replaces the element at `i`. |
| `isEmpty()` | `boolean` |
| `contains(v)`, `indexOf(v)` | `boolean`, and the index of `v` or `-1` |
| `add(v)`, `add(i, v)`, `removeLast()`, `remove(i)`, `clear()` | Change the list. `removeLast` and `remove` return the removed element. |
| `reverse()`, `sort()` | In place. `sort` works on `List<Integer>` and `List<Float>`, smallest first. |
| `forEach(f)` | Calls `f` with each element. |
| `removeIf(p)`, `replaceAll(f)`, `sort(order)` | Change the list with a [lambda](./statements#lambdas-and-method-references): drop the elements `p` accepts, replace each with `f(v)`, or sort with a [`Comparator`](./std#std-function). |

`List.of()` with no elements needs a declared type: `List<Integer> xs = List.of();`. Methods that change the list need a variable or element (`teams["red"]`), not a function result.

`sort(order)` is an insertion sort, about `n²` calls of `order` for `n` elements, fine for a server's players. `removeIf` returns nothing, unlike Java.

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
| `getOrDefault(key, fallback)` | The value for `key`, or `fallback`. |
| `put(key, value)` | Writes the entry. |
| `containsKey(key)` | `boolean` |
| `isEmpty()` | `boolean` |
| `remove(key)` | Removes `key` |
| `size()` | Number of keys |
| `keySet()` | `List<String>`, in storage order. Returns an empty list if any string value in the map contains `'` or `"`. |

Keys may only use letters, digits and `_`, and can't start with a digit.

## `Optional<T>` {#optional}

This is what `List.get`, `Map.get` and [`Selector.findFirst`](./methods#selecting-and-checking-entities) return.

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
| `isEmpty()` | `boolean` |
| `get()` | The value, or the type's empty value when absent. |
| `orElse(fallback)` | The value, or `fallback` when empty. `fallback` is always evaluated, even when a value is present. |

`Optional<void>` isn't allowed.

## Entities

- `Selector` can match any number of entities. Loop over it with `for`.
- `Entity` is one entity. Get one with `Selector.of(...).getFirst()`, or from a `for` loop over a `Selector`.
- `Player` is an `Entity` that's known to be a player.

Some methods only work on players, and `heal` only works on non-players. The compiler works out which kind a reference is from its selector:

| Selector | Known as |
| --- | --- |
| `@p`, `@a`, `@r`, `@s`, a player name, or `type=player` | player |
| any other `type=...` | non-player |
| anything else | unknown |

`(Player) e` asserts that `e` is a player, and `for (Player p : Selector.of(...))` does the same for a loop. All entity methods and fields are listed in [Entities and Players](./methods).

## `Block`

A block position, created with `Block.of("~ ~ ~")` or read from `entity.position`. The string must be a literal. For a position computed at run time, pass world coordinates as ints: `Block.of(x, 64, z)`. Relative coordinates are resolved where the code runs. To anchor them to an entity, use `at(player, Block.of("~1 ~ ~"))` or an [`at` block](./statements#as-and-at).

| Method | Does |
| --- | --- |
| `setBlock(id \| BlockData)` | Places a block. Placing a `BlockData` also writes its NBT. |
| `fill(to: Block, id \| BlockData)` | Fills the box between two positions (block id and states only) |
| `is(id) -> boolean` | Tests the block at this position |
| `isLoaded() -> boolean` | Whether the chunk here is loaded, for example a few ticks after `world.forceload`. |
| `summon(id)`, `summon(id, Nbt)`, `summon(EntityData) -> Entity` | Summons at this position |
| `spawnItem(ItemStack) -> Entity` | Drops an item stack |
| `spawnParticle(name)`, `spawnParticle(name, count)`, `spawnParticle(name, count, viewers)` | Spawns particles. |
| `lootInsert(table)`, `lootSpawn(table)` | Inserts loot into the container, or spawns it in the world |
| `debugMarker(label)`, `debugMarker(label, block)` | Places a visible marker for debugging |
| `nbt.*` | Block-entity NBT |
| `getLightLevel() -> int` | Light level 0 to 15 |
| `getType() -> String` | The block's id, such as `"minecraft:oak_stairs"`. Found with a binary search over block tags, about 16 commands. |
| `getState(name) -> String` | A block state such as `getState("facing")` → `"east"`, or `""` if the block doesn't have it. `name` must be a literal. |
| `copyTo(destination: Block)` | Copies the block, with its states and block-entity data, to `destination`. |
| `getX()`, `getY()`, `getZ()` `-> int` | World coordinates of the block. Each call summons and removes a marker. |
| `getBiome() -> String`, `inBiome(id) -> boolean` | `inBiome` accepts a `#tag` |
| `getEnvironment(attribute) -> float` | A numeric environment attribute, such as `"gameplay/sky_light_level"`. The id must be a literal. |

```mcfc
void markGround() {
    var below = Block.of("~ ~-1 ~");
    if (below.is("minecraft:grass_block") && below.getLightLevel() < 8) {
        below.setBlock("minecraft:glowstone");
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
    sword.setName("Quest Blade");
    player.hotbar[0] = sword;
    player.head.item = "minecraft:golden_helmet";
    if (player.inventory[3].exists) {
        player.sendMessage(player.inventory[3].id);
    }
    player.inventory[4].clear();
}
```

Assign an `ItemStack` to an inventory or hotbar slot to set it. The slot index can be a variable. `clear()` empties the slot. For equipment slots, write `.item` (an id or `ItemStack`), `.name` or `.count`. `inventory` and `hotbar` only work on players.

## `BossBar`

```mcfc
void show() {
    var bb = new BossBar("mypack:progress", "Progress");
    bb.setMax(10);
    bb.setValue(5);
    bb.setPlayers(Selector.of("@a"));
    bb.setVisible(true);
}
```

Use `getName()`/`setName(...)`, `getValue()`/`setValue(...)`, `getMax()`/`setMax(...)`, `getVisible()`/`setVisible(...)` and `getPlayers()`/`setPlayers(...)`. `bb.remove()` deletes the bossbar. `new BossBar(id, ...)` with an existing id gives you that bossbar.

## `Nbt`

A raw NBT path such as `player.nbt.Health` or `pig.nbt.Tags[0]`. Convert it with `(int)`, `(float)`, `(boolean)` or `(String)`, and check whether it exists with `hasData(...)`. Player NBT is read-only.
