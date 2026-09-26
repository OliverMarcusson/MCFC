# Builtins

These functions are always in scope. Library functions such as `clamp` or `startsWith` live in [`std`](./std).

## Selecting entities

| Function | Returns | Notes |
| --- | --- | --- |
| `Selector.of(s: String)` | `Selector` | A target selector or player name, such as `"@a[tag=red]"`. |
| `single(set: Selector)` | `Entity` | Narrows a selection to one entity. The selector should match one entity, like `@p`, `@s` or `limit=1`. |
| `findFirst(set)` | `Optional<Entity>` | Empty when nothing matches. The argument has to be a literal `Selector.of(...)`, optionally wrapped in `as` or `at`. The compiler adds `limit=1` itself. |
| `exists(e: Entity)` | `boolean` | Whether the entity is still there. |

```mcfc
void tick() {
    var pig = findFirst(Selector.of("@e[type=minecraft:pig]"));
    if (pig.isPresent()) {
        pig.orElse(single(Selector.of("@s"))).addTag("found");
    }
}
```

### `as` and `at`

`as(executor, target)` and `at(origin, target)` build a selector relative to another entity:

```mcfc
void tick() {
    var player = single(Selector.of("@p"));
    var nearest_pig = single(at(player, Selector.of("@e[type=minecraft:pig,sort=nearest,limit=1]")));
    nearest_pig.addTag("nearest");
}
```

To run a whole block as or at an entity, use the [`as` and `at` blocks](./statements#as-and-at).

## Creating things

| Function | Returns | Notes |
| --- | --- | --- |
| `Block.of(pos: String)` | [`Block`](./types#block) | `"~ ~ ~"`, `"10 64 -3"`, and so on |
| `new EntityData(id)` | `EntityData` | [Entity builder](./builders#entity-builders) |
| `new ItemStack(id)` | `ItemStack` | [Item builder](./builders#item-builders) |
| `new BlockData(id)` | `BlockData` | [Block builder](./builders#block-builders) |
| `new Component()`, `new Component(s)` | `Component` | [Text component builder](./builders#text-builders) |
| `new BossBar(id, name)` | [`BossBar`](./types#bossbar) | Creates the bossbar, or returns the existing one with that id |
| `summon(id)`, `summon(id, Nbt)`, `summon(EntityData)` | `Entity` | Summons at the current position |

## Waiting

| Function | Notes |
| --- | --- |
| `sleep(seconds: int)` | Pauses the current function. |
| `sleepTicks(ticks: int)` | Pauses the current function. |

Both have to be used as statements. A function that sleeps also pauses its callers. See [Functions that pause](./statements#functions-that-pause). To wait without holding up the caller, put the sleep inside [`async`](./statements#async).

Under the hood, the rest of the function becomes a separate generated function that runs later through `schedule function`.

## Random

| Function | Returns |
| --- | --- |
| `random()` | `0` to `2147483647` |
| `random(max)` | `0` to `max`, including `max` |
| `random(min, max)` | `min` to `max`, including both |
| `randomWeighted(weights: List<int>)` | An index into `weights`, where each index's chance is proportional to its weight. `weights` must be a literal such as `List.of(3, 1)`. |
| `randomBinomial(n: int, p: float)` | How many of `n` tries succeed, when each succeeds with chance `p`. |

For randomness from the host machine, see [`rand.int`](/runtime/capabilities#calls).

## World

| Function | Returns |
| --- | --- |
| `gameTime()` | `int`, the ticks the world has run (`time query gametime`) |
| `worldTime()` | `int`, the day clock (`time query time`) |
| `borderSize()` | `int`, the world border width in blocks |
| `gamerule(name)` | `int`, the game rule's value, with `true` as `1`. The name must be a literal. |

Block-level reads such as light and biome are methods on [`Block`](./types#block).

## Conversion

Conversions are [casts](./types#casts): `(int) x`, `(float) x`, `(boolean) x`, `(String) x` and `(Player) e`.

| Function | Returns |
| --- | --- |
| `hasData(path)` | `boolean`, whether a path inside a storage value (list, map, record) exists. It doesn't work on entity NBT. |

```mcfc
void inspect(Entity pig) {
    var hp = (int) pig.nbt.Health;
    var glowing = (boolean) pig.nbt.Glowing;
    var counts = Map.of("wood", 2);
    if (hasData(counts["stone"])) {
        debug("$(hp) hp");
    }
}
```

## Debugging

`debug(message: String)` sends `[MCFC debug] message` to every player with `tellraw @a`.
