# `at`

Runs a block at a different entity or block position.

```mcfc
fn sparkle() -> void:
    let player = single(selector("@p"))

    at(player):
        mc "particle minecraft:happy_villager ~ ~1 ~ 0.2 0.2 0.2 0 8"
```

Use the function-style `at(origin, value)` builtin when composing selector expressions.

## Under The Hood

The block body lowers to a generated function called through `execute at <selector-or-position> run function ...`. Relative coordinates inside raw commands and block references resolve under that position context.
