# Builtins

These classes and their methods are always in scope, with no import. Library classes such as `Timer` or `Strings` live in [`std`](./std).

## Selecting entities

| Function | Returns | Notes |
| --- | --- | --- |
| `Selector.of(s: String)` | `Selector` | A target selector or player name, such as `"@a[tag=red]"`. |

Use `selector.getFirst()` or `selector.findFirst()` to select one entity, and `entity.isValid()` to check a reference. See [Entities and Players](./methods#selecting-and-checking-entities).

```mcfc
class Main {
    @Tick
    static void tick() {
        var pig = Selector.of("@e[type=minecraft:pig]").findFirst();
        if (pig.isPresent()) {
            pig.orElse(Selector.of("@s").getFirst()).addTag("found");
        }
    }
}
```

### `Execute.as` and `Execute.at`

`Execute.as(executor, () -> target)` and `Execute.at(origin, () -> target)` build a selector, entity or block position relative to another entity:

```mcfc
class Main {
    @Tick
    static void tick() {
        var player = Selector.of("@p").getFirst();
        var nearest_pig = Execute.at(player, () -> Selector.of("@e[type=minecraft:pig,sort=nearest,limit=1]")).getFirst();
        nearest_pig.addTag("nearest");
    }
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
| `World.summon(id)`, `World.summon(id, Nbt)`, `World.summon(EntityData)` | `Entity` | Summons at the current position |

## Waiting

| Method | Notes |
| --- | --- |
| `Thread.sleep(seconds: int)` | Pauses the current method. |
| `Thread.sleepTicks(ticks: int)` | Pauses the current method. |

Both have to be used as statements. A method that sleeps also pauses its callers. See [Methods that pause](./statements#methods-that-pause). To wait without holding up the caller, put the sleep inside [`Thread.start`](./statements#thread-start).

Unlike Java's `Thread.sleep`, the argument is whole seconds, not milliseconds. Under the hood, the rest of the method becomes a separate generated function that runs later through `schedule function`.

## Random

| Method | Returns |
| --- | --- |
| `Random.nextInt()` | `0` to `2147483647` |
| `Random.nextInt(bound)` | `0` to `bound - 1`. As in Java, the bound itself is never returned. |
| `Random.nextInt(origin, bound)` | `origin` to `bound - 1` |
| `Random.weighted(weights: List<Integer>)` | An index into `weights`, where each index's chance is proportional to its weight. `weights` must be a literal such as `List.of(3, 1)`. |
| `Random.binomial(n: int, p: float)` | How many of `n` tries succeed, when each succeeds with chance `p`. |

These need no import. [`std.random`](./std#std-random) adds `Random.chance`, `nextFloat`, `pick` and `shuffle` after `import std.random.Random;`.

For randomness from the host machine, see [`rand.int`](/runtime/capabilities#calls).

## World

| Method | Returns |
| --- | --- |
| `World.getGameTime()` | `int`, the ticks the world has run (`time query gametime`) |
| `World.getTime()` | `int`, the day clock (`time query time`) |
| `World.getBorderSize()` | `int`, the world border width in blocks |
| `World.getGameRule(name)` | `int`, the game rule's value, with `true` as `1`. The name must be a literal. |

These and `World.summon` need no import. [`std.world`](./std#std-world) adds setters such as `World.setWeather` after `import std.world.World;`.

Block-level reads such as light and biome are methods on [`Block`](./types#block).

## Conversion

Conversions are [casts](./types#casts): `(int) x`, `(float) x`, `(boolean) x`, `(String) x` and `(Player) e`.

| Method | Returns |
| --- | --- |
| `Nbt.has(path)` | `boolean`, whether a path inside a storage value (list, map, record) exists. It doesn't work on entity NBT. |

```mcfc
class Main {
    static void inspect(Entity pig) {
        var hp = (int) pig.nbt.Health;
        var glowing = (boolean) pig.nbt.Glowing;
        var counts = Map.of("wood", 2);
        if (Nbt.has(counts["stone"])) {
            System.out.println("$(hp) hp");
        }
    }
}
```

## Debugging

`System.out.println(message: String)` sends `[MCFC debug] message` to every player with `tellraw @a`.

## Logging

`Log` sends leveled messages to players who opt in with `/tag @s add mcfc.log`, so a pack can keep its logging in place without spamming everyone.

```mcfc
class Main {
    public static void main() {
        Log.setLevel("debug");
        Log.info("arena loaded");
        var alive = 3;
        Log.dump(alive);
        Log.warn("only $(alive) players left");
    }
}
```

| Call | Does |
| --- | --- |
| `Log.debug(msg)`, `Log.info(msg)`, `Log.warn(msg)`, `Log.error(msg)` | Sends `[<namespace> LEVEL] msg` if the level is enabled. |
| `Log.dump(value)` | Shows any value at debug level: a number, or a string, list, map or record as NBT. |
| `Log.setLevel(level)` | `"debug"`, `"info"`, `"warn"`, `"error"` or `"off"`. Levels below it are skipped. The default is `"info"`, and the level is kept across reloads. |

Each pack has its own level, stored in the score `#log.<namespace>` of the `mcfc` objective, so `/scoreboard players set #log.mypack mcfc 0` turns on debug output without a rebuild.
