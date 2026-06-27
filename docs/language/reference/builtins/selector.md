# `selector`

```mcfc
selector(value: string) -> entity_set
```

Wraps a Minecraft selector or player name as an `entity_set`.

```mcfc
fn tick() -> void:
    let players = selector("@a")
    for player in players:
        player.add_tag("seen")
```

Use `single(selector(...))` when an API requires one entity.

## Under The Hood

Selectors are carried as selector strings plus execution context. Later calls decide whether to emit the selector directly or wrap the operation in `execute as`/`execute at`.
