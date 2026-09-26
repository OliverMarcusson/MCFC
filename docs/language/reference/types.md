# Types

| Type | Holds | Stored in |
| --- | --- | --- |
| [`int`](#int) | 32-bit integer | scoreboard |
| [`float`](#float) | 32-bit decimal | storage |
| [`bool`](#bool) | `true` / `false` | scoreboard |
| [`string`](#string) | text | storage |
| [`array<T>`](#array) | ordered list | storage |
| [`dict<T>`](#dict) | string-keyed map | storage |
| [`Optional<T>`](#optional) | a `T` or nothing | storage |
| struct / enum types | declared with [`struct`](./statements#struct) or [`enum`](./statements#enum) | storage / scoreboard |
| [`entity_set`](#entities) | a selector, which can match 0 or more entities | selector text |
| [`entity_ref`, `player_ref`](#entities) | one entity or player | selector text |
| [`block_ref`](#block-ref) | a block position | position text |
| `entity_def`, `block_def`, `item_def`, `text_def` | builders, see [Builders](./builders) | storage |
| [`item_slot`](#item-slot) | an inventory or equipment slot | the entity |
| [`bossbar`](#bossbar) | a bossbar handle | storage |
| [`nbt`](#nbt) | a raw NBT path or value | storage / the entity |
| `void` | no value, used only as a return type | |

## Rules

- `let` takes its type from the initializer. Parameters and return types are written out.
- An assignment has to keep the variable's type, and arguments have to match parameter types.
- Types never convert implicitly. `1.5 + 2` is an error, so write `1.5 + float(2)`. The one exception: `entity_def`, `block_def` and `item_def` can be used where an `nbt` value is expected, which is short for `.as_nbt()`.
- `+ - * / %` work on two `int`s or two `float`s. `+` also joins two `string`s.
- `< <= > >=` work on `int`, `float` and `bool`. `== !=` work on all of these plus `string`.
- `and`, `or` and `not` work on `bool`.
- Precedence, from tightest to loosest: `not`, then `* / %`, then `+ -`, then comparisons, then `and`, then `or`.

## `int`

`/` rounds down, and `%` takes the sign of the right side, the same as Minecraft's scoreboard `/=` and `%=`. So `-7 / 2` is `-4` and `-7 % 3` is `2`. Dividing by `0` leaves the left side unchanged. Overflow wraps.

`n.to_string()` converts to text. `"42".parse_int()` goes the other way and returns `0` if the text isn't a whole number.

## `float`

```mcfc
fn distance(x: float, z: float) -> float:
    return (x * x + z * z).sqrt()
```

Literals need a digit on both sides of the point: `1.0`, `0.5`, `-2.5`. Floats are 32-bit, which gives about 7 significant digits.

| Method | Returns |
| --- | --- |
| `sqrt()`, `abs()` | |
| `sin()`, `cos()`, `tan()` | Take radians. Entity `yaw()` and `pitch()` are in degrees, so multiply by `0.017453292`. |
| `floor()`, `ceil()`, `round()`, `trunc()` | Rounded value, still a `float` |
| `pow(e)`, `min(y)`, `max(y)`, `clamp(low, high)`, `hypot(y)` | `hypot` is `sqrt(x*x + y*y)`. `pow` stops the command when both values are `0.0`. |
| `to_string()` | `"0.5"`, `"-0.25"`, or `"4"` for a whole number |

`float(n)` converts an `int`, and `int(x)` rounds down (`int(-2.7)` is `-3`). Each float expression, however long, compiles to one `/compute` command. Comparisons cost two.

## `bool`

`true` or `false`. It's stored as a scoreboard 1 or 0.

## `string`

Literals use `"..."` or `'...'`. `$(expr)` inside a literal inserts a value.

```mcfc
fn main() -> void:
    let name = "Steve"
    let score = 42
    let line = "Hi " + name + ", you have " + score.to_string() + " points"
    let same = "Hi $(name), you have $(score) points"
```

| Method | Returns |
| --- | --- |
| `len()` | Number of characters |
| `slice(start)`, `slice(start, end)` | Substring. `end` is excluded, and a negative index counts from the end. Returns `""` when out of range. |
| `parse_int()` | The number, or `0` |
| `to_string()` | Also available on `int` and `float` |

For `starts_with`, `ends_with`, `find` and `contains`, see [`std::str`](./std#std-str).

**Limits.** Joining, `to_string()` and `$(...)` go through a Minecraft macro, which pastes the value in without escaping it. As a result:

- A value containing `"` breaks the command, and the result is `""`.
- A `\` is read as an escape, so `\n` becomes a newline.

Player names and ids never contain these characters. Text players type, such as chat, item names and signs, can.

## `array<T>` {#array}

```mcfc
fn main() -> void:
    let values = [3, 5]
    values.push(9)
    let third = values.get(2).orElse(0)
    for v in values:
        debug("$(v)")
```

| Method | Returns |
| --- | --- |
| `len()` | Number of elements |
| `xs[i]` | The element at `i` |
| `get(i)` | `Optional<T>`, empty if there's no element at `i` |
| `first()`, `last()` | The first or last element. An empty array gives the type's empty value, such as `0`. |
| `contains(v)`, `index_of(v)` | `bool`, and the index of `v` or `-1` |
| `push(v)`, `pop()`, `insert(i, v)`, `remove(i)`, `clear()` | Change the array. `pop` and `remove` return the removed element. |
| `reverse()`, `sort()` | In place. `sort` works on `array<int>` and `array<float>`, smallest first. |

Methods that change the array need a variable or element (`teams["red"]`), not a function result.

`contains`, `index_of` and `reverse` loop over every element. `sort` is a merge sort that does at most 1,000 steps per tick. Small arrays finish immediately. Larger ones [pause](./statements#functions-that-pause) the function: 5,000 elements take about 3 seconds.

## `dict<T>` {#dict}

```mcfc
fn main() -> void:
    let counts = {"wood": 2, "stone": 4}
    counts["iron"] = 1
    let gold = counts.get("gold").orElse(0)
    for key in counts.keys():
        debug("$(key)=$(counts[key])")
```

| Method | Returns |
| --- | --- |
| `d[key]` | The value for `key` |
| `get(key)` | `Optional<T>` |
| `has(key)` | `bool` |
| `remove(key)` | Removes `key` |
| `len()` | Number of keys |
| `keys()` | `array<string>`, in storage order. Returns `[]` if any string value in the dict contains `'` or `"`. |

Keys may only use letters, digits and `_`, and can't start with a digit.

## `Optional<T>` {#optional}

This is what `array.get`, `dict.get` and [`find_first`](./builtins#selecting-entities) return.

```mcfc
fn main() -> void:
    let maybe = [4, 8].get(3)
    if maybe.isPresent():
        debug("found")
    let count = maybe.orElse(0)
```

| Method | Returns |
| --- | --- |
| `isPresent()` | `bool` |
| `orElse(fallback)` | The value, or `fallback` when empty. `fallback` is always evaluated, even when a value is present. |

`Optional<void>` isn't allowed.

## Entities

- `entity_set` is a selector. It can match any number of entities. Loop over it with `for`.
- `entity_ref` is one entity. Get one with `single(selector(...))`, or from a `for` loop over an `entity_set`.
- `player_ref` is an `entity_ref` that's known to be a player.

Some methods only work on players, and `heal` only works on non-players. The compiler works out which kind a reference is from its selector:

| Selector | Known as |
| --- | --- |
| `@p`, `@a`, `@r`, `@s`, a player name, or `type=player` | player |
| any other `type=...` | non-player |
| anything else | unknown |

`player_ref(e)` asserts that `e` is a player. All entity methods and fields are listed in [Entities and Players](./methods).

## `block_ref`

A block position, created with `block("~ ~ ~")` or read from `entity.position`. `block(...)` needs a literal string, so a position can't be computed at run time yet. Relative coordinates are resolved where the code runs. To anchor them to an entity, use `at(player, block("~1 ~ ~"))` or an [`at:` block](./statements#as-and-at).

| Method | Does |
| --- | --- |
| `setblock(id \| block_def)` | Places a block. Placing a `block_def` also writes its NBT. |
| `fill(to: block_ref, id \| block_def)` | Fills the box between two positions (block id and states only) |
| `is(id) -> bool` | Tests the block at this position |
| `summon(id)`, `summon(id, nbt)`, `summon(entity_def) -> entity_ref` | Summons at this position |
| `spawn_item(item_def) -> entity_ref` | Drops an item stack |
| `particle(name)`, `particle(name, count)`, `particle(name, count, viewers)` | |
| `loot_insert(table)`, `loot_spawn(table)` | Inserts loot into the container, or spawns it in the world |
| `debug_marker(label)`, `debug_marker(label, block)` | Places a visible marker for debugging |
| `nbt.*` | Block-entity NBT |
| `light() -> int` | Light level 0 to 15 |
| `biome() -> string`, `in_biome(id) -> bool` | `in_biome` accepts a `#tag` |
| `environment(attribute) -> float` | A numeric environment attribute, such as `"gameplay/sky_light_level"`. The id must be a literal. |

```mcfc
fn mark_ground() -> void:
    let below = block("~ ~-1 ~")
    if below.is("minecraft:grass_block") and below.light() < 8:
        below.setblock("minecraft:glowstone")
```

Environment attributes: `visual/cloud_height`, `visual/fog_start_distance`, `visual/fog_end_distance`, `visual/sky_fog_end_distance`, `visual/cloud_fog_end_distance`, `visual/water_fog_start_distance`, `visual/water_fog_end_distance`, `visual/moon_angle`, `visual/star_angle`, `visual/sun_angle`, `visual/sky_light_factor`, `visual/star_brightness`, `audio/music_volume`, `gameplay/cat_waking_up_gift_chance`, `gameplay/creature_world_gen_spawn_probability`, `gameplay/surface_slime_spawn_chance`, `gameplay/turtle_egg_hatch_chance`, `gameplay/sky_light_level`.

## `item_slot`

`player.inventory[0..26]`, `player.hotbar[0..8]` and the equipment slots `mainhand`, `offhand`, `head`, `chest`, `legs` and `feet`.

| Field | Type |
| --- | --- |
| `exists` | `bool`, read-only |
| `id` | `string`, read-only |
| `count` | `int` |
| `name` | `string` |
| `nbt` | `nbt` |

```mcfc
fn equip(player: player_ref) -> void:
    let sword = item("minecraft:diamond_sword")
    sword.name = "Quest Blade"
    player.hotbar[0] = sword
    player.head.item = "minecraft:golden_helmet"
    if player.inventory[3].exists:
        player.tellraw(player.inventory[3].id)
    player.inventory[4].clear()
```

Assign an `item_def` to an inventory or hotbar slot to set it. The slot index can be a variable. `clear()` empties the slot. For equipment slots, write `.item` (an id or `item_def`), `.name` or `.count`. `inventory` and `hotbar` only work on players.

## `bossbar`

```mcfc
fn show() -> void:
    let bb = bossbar("mypack:progress", "Progress")
    bb.max = 10
    bb.value = 5
    bb.players = selector("@a")
    bb.visible = true
```

The fields are `name` (a `string` or `text_def`), `value`, `max`, `visible` and `players`. `bb.remove()` deletes the bossbar. Calling `bossbar(id, ...)` with an existing id gives you that bossbar.

## `nbt`

A raw NBT path such as `player.nbt.Health` or `pig.nbt.Tags[0]`. Convert it with `int(...)`, `float(...)`, `bool(...)` or `string(...)`, and check whether it exists with `has_data(...)`. Player NBT is read-only.
