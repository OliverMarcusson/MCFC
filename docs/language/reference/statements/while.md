# `while`

Repeats a block while a `bool` expression remains true.

```mcfc
fn count() -> void:
    let i = 0
    while i < 3:
        mcf "say $(i)"
        i = i + 1
```

Use `break` to exit early and `continue` to skip to the next iteration.

## Under The Hood

`while` lowers to generated loop functions. MCFC reevaluates the condition each iteration, uses scoreboard slots for loop control, and schedules or calls continuation functions when the body contains `sleep` or another suspension point.
