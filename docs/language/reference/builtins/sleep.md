# `sleep`

```mcfc
sleep(seconds: int) -> void
```

Pauses the current execution path by seconds.

```mcfc
fn delayed() -> void:
    async:
        sleep(5)
        debug("five seconds later")
```

Inside `async`, the sleep pauses only that async branch.

## Under The Hood

`sleep` splits the current lowered function at the call site. MCFC emits a continuation function for the remaining statements and schedules it with Minecraft `schedule function` after converting seconds to ticks.
