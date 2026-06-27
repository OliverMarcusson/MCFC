# `block_type`

```mcfc
block_type(id: string) -> block_def
```

Creates a block builder.

```mcfc
fn place() -> void:
    let chest = block_type("minecraft:chest")
    chest.states.facing = "north"
    chest.name = "Loot"
    block("~ ~ ~").setblock(chest)
```

