# Builtins

These functions are always in scope. Library functions such as `clamp` or `starts_with` live in [`std`](./std).

## Selecting entities

| Function | Returns | Notes |
| --- | --- | --- |
| `selector(s: string)` | `entity_set` | A target selector or player name, such as `"@a[tag=red]"`. |
| `single(set: entity_set)` | `entity_ref` | Narrows a selection to one entity. The selector should match one entity, like `@p`, `@s` or `limit=1`. |
| `find_first(set)` | `Optional<entity_ref>` | Empty when nothing matches. The argument has to be a literal `selector(...)`, optionally wrapped in `as` or `at`. The compiler adds `limit=1` itself. |
| `exists(e: entity_ref)` | `bool` | Whether the entity is still there. |
| `player_ref(e: entity_ref)` | `player_ref` | Asserts that `e` is a player. |

```mcfc
fn tick() -> void:
    let pig = find_first(selector("@e[type=minecraft:pig]"))
    if pig.isPresent():
        pig.orElse(single(selector("@s"))).add_tag("found")
```

### `as` and `at`

`as(executor, target)` and `at(origin, target)` build a selector relative to another entity:

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))
    let nearest_pig = single(at(player, selector("@e[type=minecraft:pig,sort=nearest,limit=1]")))
    nearest_pig.add_tag("nearest")
```

To run a whole block as or at an entity, use the [`as:` and `at:` statements](./statements#as-and-at).

## Creating things

| Function | Returns | Notes |
| --- | --- | --- |
| `block(pos: string)` | [`block_ref`](./types#block-ref) | `"~ ~ ~"`, `"10 64 -3"`, and so on |
| `entity(id)` | `entity_def` | [Entity builder](./builders#entity-builders) |
| `item(id)` | `item_def` | [Item builder](./builders#item-builders) |
| `block_type(id)` | `block_def` | [Block builder](./builders#block-builders) |
| `text()`, `text(s)` | `text_def` | [Text component builder](./builders#text-builders) |
| `bossbar(id, name)` | [`bossbar`](./types#bossbar) | Creates the bossbar, or returns the existing one with that id |
| `summon(id)`, `summon(id, nbt)`, `summon(entity_def)` | `entity_ref` | Summons at the current position |

## Waiting

| Function | Notes |
| --- | --- |
| `sleep(seconds: int)` | Pauses the current function. |
| `sleep_ticks(ticks: int)` | Pauses the current function. |

Both have to be used as statements. A function that sleeps also pauses its callers. See [Functions that pause](./statements#functions-that-pause). To wait without holding up the caller, put the sleep inside [`async`](./statements#async).

Under the hood, the rest of the function becomes a separate generated function that runs later through `schedule function`.

## Random

| Function | Returns |
| --- | --- |
| `random()` | `0` to `2147483647` |
| `random(max)` | `0` to `max`, including `max` |
| `random(min, max)` | `min` to `max`, including both |
| `random_weighted(weights: array<int>)` | An index into `weights`, where each index's chance is proportional to its weight. `weights` must be a literal such as `[3, 1]`. |
| `random_binomial(n: int, p: float)` | How many of `n` tries succeed, when each succeeds with chance `p`. |

For randomness from the host machine, see [`rand.int`](/runtime/capabilities#calls).

## World

| Function | Returns |
| --- | --- |
| `game_time()` | `int`, the ticks the world has run (`time query gametime`) |
| `world_time()` | `int`, the day clock (`time query time`) |
| `border_size()` | `int`, the world border width in blocks |
| `gamerule(name)` | `int`, the game rule's value, with `true` as `1`. The name must be a literal. |

Block-level reads such as light and biome are methods on [`block_ref`](./types#block-ref).

## Conversion

| Function | Returns |
| --- | --- |
| `int(x)` | From `float` (rounds down) or `nbt` |
| `float(x)` | From `int` or `nbt` |
| `bool(x)`, `string(x)` | From `nbt` |
| `has_data(path)` | `bool`, whether a path inside a storage value (array, dict, struct) exists. It doesn't work on entity NBT. |

```mcfc
fn inspect(pig: entity_ref) -> void:
    let hp = int(pig.nbt.Health)
    let glowing = bool(pig.nbt.Glowing)
    let counts = {"wood": 2}
    if has_data(counts["stone"]):
        debug("$(hp) hp")
```

## Debugging

`debug(message: string)` sends `[MCFC debug] message` to every player with `tellraw @a`.
