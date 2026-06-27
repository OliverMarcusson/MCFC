# `as`

Runs a block with a different executor.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))

    as(player):
        mc "say executor is @s"
```

Use the function-style `as(executor, value)` builtin when composing selector expressions.

