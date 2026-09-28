# Builtins

These functions are always in scope. Library functions such as `clamp` or `startsWith` live in [`std`](./std).

## Selecting entities

| Function | Returns | Notes |
| --- | --- | --- |
| `Selector.of(s: String)` | `Selector` | A target selector or player name, such as `"@a[tag=red]"`. |

Use `selector.getFirst()` or `selector.findFirst()` to select one entity, and `entity.isValid()` to check a reference. See [Entities and Players](./methods#selecting-and-checking-entities).

```mcfc
void tick() {
    var pig = Selector.of("@e[type=minecraft:pig]").findFirst();
    if (pig.isPresent()) {
        pig.orElse(Selector.of("@s").getFirst()).addTag("found");
    }
}
```

### `Execute.as` and `Execute.at`

`Execute.as(executor, () -> target)` and `Execute.at(origin, () -> target)` build a selector, entity or block position relative to another entity:

```mcfc
void tick() {
    var player = Selector.of("@p").getFirst();
    var nearest_pig = Execute.at(player, () -> Selector.of("@e[type=minecraft:pig,sort=nearest,limit=1]")).getFirst();
    nearest_pig.addTag("nearest");
}
```

To run a block of code as or at an entity, pass a block lambda. See [`Execute.as` and `Execute.at`](./statements#execute-as-and-execute-at).

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

Both have to be used as statements. A function that sleeps also pauses its callers. See [Functions that pause](./statements#functions-that-pause). To wait without holding up the caller, put the sleep inside [`Thread.start`](./statements#thread-start).

Under the hood, the rest of the function becomes a separate generated function that runs later through `schedule function`.

## Random

| Function | Returns |
| --- | --- |
| `random()` | `0` to `2147483647` |
| `random(max)` | `0` to `max`, including `max` |
| `random(min, max)` | `min` to `max`, including both |
| `randomWeighted(weights: List<Integer>)` | An index into `weights`, where each index's chance is proportional to its weight. `weights` must be a literal such as `List.of(3, 1)`. |
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

## Logging

`Log` sends leveled messages to players who opt in with `/tag @s add mcfc.log`, so a pack can keep its logging in place without spamming everyone.

```mcfc
void main() {
    Log.setLevel("debug");
    Log.info("arena loaded");
    var alive = 3;
    Log.dump(alive);
    Log.warn("only $(alive) players left");
}
```

| Call | Does |
| --- | --- |
| `Log.debug(msg)`, `Log.info(msg)`, `Log.warn(msg)`, `Log.error(msg)` | Sends `[<namespace> LEVEL] msg` if the level is enabled. |
| `Log.dump(value)` | Shows any value at debug level: a number, or a string, list, map or record as NBT. |
| `Log.setLevel(level)` | `"debug"`, `"info"`, `"warn"`, `"error"` or `"off"`. Levels below it are skipped. The default is `"info"`, and the level is kept across reloads. |

Each pack has its own level, stored in the score `#log.<namespace>` of the `mcfc` objective, so `/scoreboard players set #log.mypack mcfc 0` turns on debug output without a rebuild.
