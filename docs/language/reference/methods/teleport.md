# `teleport`

```mcfc
entity.teleport(destination: entity_ref|block_ref) -> void
```

Teleports the receiver to another entity or block position.

```mcfc
fn rescue() -> void:
    let player = single(selector("@p"))
    let pig = single(selector("@e[type=minecraft:pig,limit=1]"))
    pig.teleport(player)
```

