# `break`, `continue`, And `return`

`break` exits the nearest loop. `continue` skips to the next loop iteration. `return` exits the current function.

```mcfc
fn first_ready() -> void:
    for player in selector("@a"):
        if not player.has_tag("ready"):
            continue

        player.tellraw("Ready")
        break
```

```mcfc
fn clamp(value: int) -> int:
    if value < 0:
        return 0

    return value
```

`return expr` must match the function return type. Use plain `return` in `void` functions.

