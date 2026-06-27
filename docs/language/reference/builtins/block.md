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

## Under The Hood

Block references carry a position string plus context. Block methods render that position into vanilla commands such as `setblock`, `fill`, `loot`, `particle`, or `summon`.
