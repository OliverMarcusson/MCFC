# `block_def`

Mutable block builder created with `block_type(...)`.

```mcfc
fn place() -> void:
    let chest = block_type("minecraft:chest")
    chest.states.facing = "north"
    block("~ ~ ~").setblock(chest)
```

