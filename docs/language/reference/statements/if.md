# `if` And `else`

Branches on a `bool` expression. `else` is optional.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))

    if player.has_tag("ready"):
        player.tellraw("Ready")
    else:
        player.tellraw("Waiting")
```

Each branch has its own block scope.

