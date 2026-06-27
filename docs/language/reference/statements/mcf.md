# `mcf`

Emits a macro command with runtime `$(...)` placeholders.

```mcfc
fn reward(amount: int) -> void:
    mcf "xp add @a $(amount) levels"
    mcf "say next reward is $(amount + 1)"
```

Use [`mc`](./mc) for literal commands that do not need MCFC values.

