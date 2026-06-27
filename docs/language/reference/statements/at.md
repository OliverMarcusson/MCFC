# `at`

Runs a block at a different entity or block position.

```mcfc
fn sparkle() -> void:
    let player = single(selector("@p"))

    at(player):
        mc "particle minecraft:happy_villager ~ ~1 ~ 0.2 0.2 0.2 0 8"
```

Use the function-style `at(origin, value)` builtin when composing selector expressions.

