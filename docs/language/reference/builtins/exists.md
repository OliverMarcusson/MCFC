# `exists`

```mcfc
exists(value: entity_ref) -> bool
```

Tests whether an entity reference exists.

```mcfc
fn tick() -> void:
    let pig = single(selector("@e[type=minecraft:pig,limit=1]"))
    if exists(pig):
        pig.add_tag("found")
```

