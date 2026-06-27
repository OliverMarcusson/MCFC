# `sleep_ticks`

```mcfc
sleep_ticks(ticks: int) -> void
```

Pauses the current execution path by Minecraft ticks.

```mcfc
fn delayed() -> void:
    async:
        sleep_ticks(20)
        debug("one second later")
```

