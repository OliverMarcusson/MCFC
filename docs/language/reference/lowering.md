# How MCFC Compiles

MCFC compiles `.mcf` source into a vanilla datapack. The backend writes generated `.mcfunction` files, scoreboard objectives, command-storage state, tags, schedules, and optional helper descriptors.

## Pipeline

```text
.mcf source
  -> parse and type check
  -> IR lowering
  -> conservative optimization
  -> datapack backend
  -> whole-pack optimization
  -> data/<namespace>/function/*.mcfunction
```

Generated files are deterministic and use reserved generated paths under the pack namespace.

## Entry Points

- `public static void main()` becomes an internal generated function called by `data/<namespace>/function/main.mcfunction`.
- `@Tick` methods are called, in module order, from `data/<namespace>/function/tick.mcfunction`, the datapack tick entrypoint.
- A class with `static` fields gets a generated `clinit` function in the load tag, which sets their initial values once per world.
- Exported methods get public wrapper `.mcfunction` files so Minecraft can call them directly.
- `@EventHandler`, `@Command`, `@Every` and `@After` methods lower to generated dispatcher functions. A listener class's one object is kept in a generated static field.
- Methods nothing can reach are dropped after type checking. The starting points are `main`, `@Tick`, `@EventHandler`, `@Command`, `@Every` and `@After` methods, `[[export]]` functions, and your zero-argument `static void` methods. Unused helpers and unused `std` methods therefore add nothing to the pack, although they are still checked for errors.
- A method compiles under its module path and class. `util.Util.twice` uses generated names such as `generated/util__util__twice__d0__entry` and scoreboard slots such as `$d0_util__Util__twice_x`. Its public wrapper, when it has one, is `data/<namespace>/function/util/util/twice.mcfunction`. Paths are lowercase, so `Main.resetArena` becomes `main/reset_arena`.

## Value Representation

| MCFC value | Runtime representation |
| --- | --- |
| `int` | scoreboard value in the generated `mcfc` objective or a state objective |
| `float` | command storage float tag, computed with one `/compute` command per expression |
| `boolean` | scoreboard value, conventionally `0` or `1` |
| `enum` | scoreboard value assigned by constant declaration order, starting at `0` |
| `String` | command storage |
| `List<T>` / `Map<String, T>` | command storage |
| `Optional<T>` | command-storage compound with `present` byte and `value` when present |
| record | command storage object or decomposed fields, depending on use |
| declared `String`, `float`, or record player/entity state | `<namespace>:state` command storage keyed by the target's UUID |
| `Selector` | selector string plus context |
| `Entity` / `Player` | selector or executor-aware reference |
| `Block` | position string plus context |
| `EntityData`, `BlockData`, `ItemStack`, `Component` | command-storage builder payloads |
| `BossBar` | command-storage handle containing the bossbar id |
| `Nbt` | command-storage path or live NBT path |

Declared `int` and `boolean` player/entity state use managed scoreboard objectives. The command storage for other declared state persists across datapack reloads; setup does not clear it.

Safe collection `get` checks whether the element or key exists at its storage path, then sets the Optional `present` flag and copies its value when found. Dynamic list indices and map keys use generated command macros. `findFirst` checks the one-entity selector with `execute if entity` before setting the flag. `isPresent()` reads the flag into a scoreboard boolean. `orElse(...)` evaluates the fallback first, then replaces it with the stored value if present.

## Control Flow

MCFC uses scoreboard guard slots to model branches, loops, `break`, `continue`, `return`, and suspended execution. Blocks that need to resume later are split into generated continuation functions.

`if`, `switch`, and loops generally become `execute if/unless score ... run function ...` calls into generated block functions. A `switch` stores its value once and lowers its cases into nested branches. Loop counters and guard flags live in scoreboards.

## Commands

`mc("...")` writes the literal command directly into the generated `.mcfunction`.

`mcf("...")` writes a generated macro function. MCFC evaluates each `$(...)` expression into command storage, then calls the macro with `function namespace:path with storage namespace:runtime <path>`.

## Context

`Execute.as(entity, () -> { ... })` and `Execute.at(entity, () -> { ... })` lower to `execute as ... run function ...` or `execute at ... run function ...`. Nested context is carried through generated function calls so references such as `@s` and relative positions keep the intended meaning.

## Async, Sleep, And Host Calls

`Thread.start(() -> { ... })` creates a separate generated function and launches it without waiting. Captured locals are copied into storage/scoreboard slots before launch.

`sleep(...)`, `sleepTicks(...)`, and host bridge calls split the current function at the suspension point. MCFC emits a continuation function and resumes it later with Minecraft `schedule function` or the `mcfd` response pump.

## Builders And NBT

Builder values are assembled in command storage. Calls such as `summon(EntityData)` and `Block.setBlock(BlockData)` render those stored payloads into Minecraft commands and `data modify` operations.

When a builder is used where `Nbt` is expected, MCFC emits the equivalent of reading the builder's `.asNbt()` payload.

## Events And Commands

Vanilla-safe events lower to datapack detectors:

- `PlayerJoinEvent` uses generated player tagging to detect first-seen players.
- `PlayerDeathEvent` uses `deathCount` scoreboard objectives and seen counters.
- `@Command` uses a trigger objective and dispatches matching players.
- `@Every` and `@After` use generated counters or `schedule function`.

Agent-backed events lower to generated `agent/event/<name>.mcfunction` entrypoints. `mcfd-agent` writes the current payload into command storage; the wrapper copies that payload into the typed event parameter slot before calling the handler. If the handler calls `event.cancel()`, MCFC writes a cancellation decision into the generated agent decision storage.


## Optimization

By default MCFC runs a conservative optimization pass. It folds literal expressions, removes self-assignments, drops `while (false)` bodies, and simplifies `if` statements with literal conditions.

After the backend, a whole-pack pass rewrites the emitted commands to run fewer of them:

- drops control-flow guards it can prove are zero, and turns long guarded tails into one early `return`
- inlines short and single-use functions, then deletes functions nothing reaches
- decides conditions and operations on scores holding known constants
- substitutes single-use temporaries and folds `x = x + y` into one operation
- removes writes to scores nothing reads, and merges identical functions

Functions that are public, scheduled or referenced by name keep their files and entry behavior. Pass `--no-optimize` to see the output without these changes. `--emit-ir` writes the intermediate form to `debug/ir.txt`.
