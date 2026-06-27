# `single`

```mcfc
single(value: entity_set) -> entity_ref
```

Narrows an entity set to a single entity reference.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))
    player.tellraw("hello")
```

Selectors passed to `single` should naturally resolve to one entity, such as `@p` or a selector with `limit=1`.

