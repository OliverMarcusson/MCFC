# How MCFC Lowers To `mcfunction`

MCFC compiles `.mcf` source into a vanilla datapack. The backend writes generated `.mcfunction` files, scoreboard objectives, command-storage state, tags, schedules, and optional helper descriptors.

## Pipeline

```text
.mcf source
  -> parse and type check
  -> IR lowering
  -> conservative optimization
  -> datapack backend
  -> data/<namespace>/function/*.mcfunction
```

Generated files are deterministic and use reserved generated paths under the pack namespace.

## Entry Points

- `fn main() -> void:` becomes an internal generated function called by `data/<namespace>/function/main.mcfunction`.
- `fn tick() -> void:` becomes the datapack tick entrypoint.
- Exported functions get public wrapper `.mcfunction` files so Minecraft can call them directly.
- Bukkit-style `event`, `command`, and `task` declarations lower to generated dispatcher functions.

## Value Representation

| MCFC value | Runtime representation |
| --- | --- |
| `int` | scoreboard value in the generated `mcfc` objective or a state objective |
| `bool` | scoreboard value, conventionally `0` or `1` |
| `string` | command storage |
| `array<T>` / `dict<T>` | command storage |
| `struct` | command storage object or decomposed fields, depending on use |
| `entity_set` | selector string plus context |
| `entity_ref` / `player_ref` | selector or executor-aware reference |
| `block_ref` | position string plus context |
| `entity_def`, `block_def`, `item_def`, `text_def` | command-storage builder payloads |
| `bossbar` | command-storage handle containing the bossbar id |
| `nbt` | command-storage path or live NBT path |

## Control Flow

MCFC uses scoreboard guard slots to model branches, loops, `break`, `continue`, `return`, and suspended execution. Blocks that need to resume later are split into generated continuation functions.

`if`, `match`, and loops generally become `execute if/unless score ... run function ...` calls into generated block functions. Loop counters and guard flags live in scoreboards.

## Commands

`mc "..."` writes the literal command directly into the generated `.mcfunction`.

`mcf "..."` writes a generated macro function. MCFC evaluates each `$(...)` expression into command storage, then calls the macro with `function namespace:path with storage namespace:runtime <path>`.

## Context

`as(entity):` and `at(entity):` lower to `execute as ... run function ...` or `execute at ... run function ...`. Nested context is carried through generated function calls so references such as `@s` and relative positions keep the intended meaning.

## Async, Sleep, And Host Calls

`async:` creates a separate generated function and launches it without waiting. Captured locals are copied into storage/scoreboard slots before launch.

`sleep(...)`, `sleep_ticks(...)`, and host bridge calls split the current function at the suspension point. MCFC emits a continuation function and resumes it later with Minecraft `schedule function` or the `mcfd` response pump.

## Builders And NBT

Builder values are assembled in command storage. Methods such as `summon(entity_def)` and `setblock(block_def)` render those stored payloads into Minecraft commands and `data modify` operations.

When a builder is used where `nbt` is expected, MCFC emits the equivalent of reading the builder's `.as_nbt()` payload.

## Events And Commands

Vanilla-safe events lower to datapack detectors:

- `player_join` uses generated player tagging to detect first-seen players.
- `player_death` uses `deathCount` scoreboard objectives and seen counters.
- `command name:` uses a trigger objective and dispatches matching players.
- `task` declarations use generated counters or `schedule function`.

Agent-backed events lower to generated `agent/event/<name>.mcfunction` entrypoints. `mcfd-agent` writes the current payload into command storage; the wrapper copies that payload into the typed event parameter slot before calling the handler. If the handler calls `event.cancel()`, MCFC writes a cancellation decision into the generated agent decision storage.

