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

