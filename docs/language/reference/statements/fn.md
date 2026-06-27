# `fn`

Declares a function with typed parameters and an explicit return type.

```mcfc
fn greet(player: player_ref, message: string) -> void:
    player.tellraw(message)
```

`fn tick() -> void:` is special: it maps to the datapack tick function and runs every game tick.

