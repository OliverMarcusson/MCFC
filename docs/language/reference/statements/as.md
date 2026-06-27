# `as`

Runs a block with a different executor.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))

    as(player):
        mc "say executor is @s"
```

Use the function-style `as(executor, value)` builtin when composing selector expressions.

## Under The Hood

The block body lowers to a generated function called through `execute as <selector> run function ...`. Nested calls inherit that executor context unless another context block changes it.
