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

## Under The Hood

String-id summons emit a `summon` command directly. Builder summons read the `entity_def` id and NBT payload from command storage, generate the summon command, and track the resulting entity reference as a selector-aware value.
