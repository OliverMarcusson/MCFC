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

## Under The Hood

The condition is evaluated into a scoreboard boolean. MCFC emits guarded `execute if score ... run function ...` calls for the branch bodies, and generated control slots prevent later branch commands from running after a `return`, `break`, or suspended call.
