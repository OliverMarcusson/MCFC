# `block`

```mcfc
block(pos: string) -> block_ref
```

Creates a block reference from Minecraft coordinates.

```mcfc
fn tick() -> void:
    let here = block("~ ~ ~")
    here.setblock("minecraft:gold_block")
```

