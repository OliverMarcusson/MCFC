# `bool`

```mcfc
bool(value: nbt) -> bool
```

Converts an NBT value to a `bool`.

```mcfc
fn read(pig: entity_ref) -> void:
    let glowing = bool(pig.nbt.Glowing)
    if glowing:
        pig.add_tag("glowing")
```

