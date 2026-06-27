# `let`

Creates a new local binding. The local type is inferred from the initializer.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))
    let amount = 5
    player.tellraw("amount=$(amount)")
```

Reusing an existing local or parameter name with `let` is rejected. Bindings created inside a block are not visible outside that block.

