# `fn`

Declares a function with typed parameters and an explicit return type.

```mcfc
fn greet(player: player_ref, message: string) -> void:
    player.tellraw(message)
```

`fn tick() -> void:` is special: it maps to the datapack tick function and runs every game tick.

## Under The Hood

Each function lowers to a generated `.mcfunction` body plus, when exported, a public wrapper under `data/<namespace>/function/`. The wrapper resets the generated control slot before calling the lowered body. `tick` is wired into the datapack tick entrypoint.
