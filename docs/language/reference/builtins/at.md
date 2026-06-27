# `at`

```mcfc
at(origin: entity_ref, value: entity_set|entity_ref|block_ref)
```

Composes a selector or reference at another entity or position.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))
    let nearest = single(at(player, selector("@e[type=minecraft:pig,sort=nearest,limit=1]")))
    nearest.add_tag("nearest")
```

For statement blocks, see [`at:`](../statements/at).

