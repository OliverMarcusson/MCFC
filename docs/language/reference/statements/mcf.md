# `mcf`

Emits a macro command with runtime `$(...)` placeholders.

```mcfc
fn reward(amount: int) -> void:
    mcf "xp add @a $(amount) levels"
    mcf "say next reward is $(amount + 1)"
```

Use [`mc`](./mc) for literal commands that do not need MCFC values.

## Under The Hood

MCFC emits a separate macro `.mcfunction` containing the final command with `$()` macro slots. It stores each placeholder value in command storage, then calls the macro with `function ... with storage ...`.
