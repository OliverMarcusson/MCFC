# `as`

```mcfc
as(executor: entity_set|entity_ref, value: entity_set|entity_ref|block_ref)
```

Composes a selector or reference under a different executor context.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))
    let self_ref = single(as(player, selector("@s")))
    self_ref.tellraw("hello")
```

For statement blocks, see [`as:`](../statements/as).

