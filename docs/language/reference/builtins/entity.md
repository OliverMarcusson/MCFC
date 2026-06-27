# `entity`

```mcfc
entity(id: string) -> entity_def
```

Creates an entity builder.

```mcfc
fn spawn() -> void:
    let pig = entity("minecraft:pig")
    pig.name = "Pet"
    pig.no_ai = true
    summon(pig)
```

See [Builders](../builders) for `entity_def` fields.

