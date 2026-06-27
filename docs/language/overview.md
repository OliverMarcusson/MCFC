# Language Overview

MCFC uses indentation for block structure and requires explicit function signatures. It is designed around typed gameplay code that lowers to Minecraft commands, scoreboards, storage, functions, and schedules.

```mcfc
fn main() -> void:
    let player = single(selector("@p"))
    let bb = bossbar("mcfc:demo", "MCFC Bossbar")

    bb.value = 5
    bb.max = 10
    bb.visible = true
    bb.players = player

    async:
        sleep(5)
        bb.remove()
        player.position.setblock("minecraft:gold_block")
```

## Core Features

- functions with typed parameters and return types
- integer, boolean, string, array, dictionary, struct, entity, block, item, bossbar, NBT, and scoreboard-backed state values
- `if`, `match`, `while`, range `for`, and selector `for`
- `as(...)` and `at(...)` execution context composition
- raw Minecraft commands with `mc`
- macro commands with `mcf`
- non-blocking `async:` blocks with `sleep(...)` and `sleep_ticks(...)`
- special `tick()` functions that compile to the datapack tick entrypoint
- vanilla-safe `data`, `event`, `command`, and `task` declarations

## Comments And Blocks

`#` starts a line comment and may also appear after code on a line.

Tabs are rejected for indentation. Use spaces.

::: warning Legacy syntax
The old `end` block terminator is no longer supported.
:::
