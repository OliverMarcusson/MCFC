# Async And Runtime

`async:` launches a new execution path immediately and continues the caller without waiting.

```mcfc
fn main() -> void:
    async:
        sleep(5)
        debug("later")
    debug("now")
```

Rules:

- `async` is statement-only
- locals and parameters are snapshotted when the async block starts
- later parent mutations do not affect the async copy
- `return` is not allowed inside an async block
- `break` and `continue` keep their normal loop-only rules

## Sleep

`sleep(seconds)` pauses the current execution path and resumes it later with Minecraft `schedule function`.

`sleep_ticks(ticks)` does the same thing with Minecraft tick units.

Both are statement-only and cannot be used as nested expressions.

## Tick Functions

`fn tick() -> void:` compiles to the datapack tick entrypoint.

```mcfc
fn tick() -> void:
    for marker in selector("@e[type=minecraft:marker,tag=decay]"):
        marker.state.age = marker.state.age + 1
```

## Runtime Representation

The current backend maps values to Minecraft primitives:

- `int` and `bool`: scoreboards
- `string`, arrays, dictionaries, and bossbar handles: command storage
- entity and block references: selector or context-aware command forms
- async paths and sleeps: generated functions plus Minecraft schedules

Generated files are deterministic and use a reserved generated namespace layout.

See [How MCFC Lowers To mcfunction](./reference/lowering) for the full reference view of scoreboards, storage, generated functions, macro calls, and event dispatch.

## Optimization

By default, MCFC runs a conservative IR optimization pass. It folds pure literal expressions, removes simple no-op self-assignments, drops `while false:` bodies, and simplifies literal `if` branches when this cannot change later control-flow guarding.

Use `--no-optimize` to inspect unoptimized lowered output.
