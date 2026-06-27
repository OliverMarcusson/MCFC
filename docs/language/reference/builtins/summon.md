# `summon`

```mcfc
summon(id: string) -> entity_ref
summon(id: string, data: nbt) -> entity_ref
summon(spec: entity_def) -> entity_ref
```

Summons an entity and returns a reference to it.

```mcfc
fn spawn() -> void:
    let pig = entity("minecraft:pig")
    pig.name = "Pet"

    let spawned = summon(pig)
    spawned.add_tag("pet")
```

