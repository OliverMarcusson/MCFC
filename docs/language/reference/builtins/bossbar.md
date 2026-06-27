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

