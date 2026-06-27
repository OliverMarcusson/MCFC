# `async`

Launches a new execution path immediately and continues the caller without waiting.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))

    async:
        sleep_ticks(20)
        player.actionbar("later")

    player.actionbar("now")
```

Locals and parameters are snapshotted when the async block starts. `return` is not allowed inside `async`.

## Under The Hood

MCFC emits the async body as a separate generated function. Before launching it, captured scoreboard values and storage values are copied into async-local slots, then the generated function is called without blocking the parent path.
