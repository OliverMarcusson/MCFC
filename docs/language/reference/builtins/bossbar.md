# `bossbar`

```mcfc
bossbar(id: string, name: string|text_def) -> bossbar
```

Creates or references a bossbar handle.

```mcfc
fn tick() -> void:
    let bb = bossbar("mcfc:demo", "Demo")
    bb.max = 10
    bb.value = 5
    bb.players = selector("@a")
    bb.visible = true
```

## Under The Hood

The bossbar handle stores the bossbar id in command storage. Field writes lower to vanilla `bossbar set` commands, and `remove()` lowers to `bossbar remove`.
