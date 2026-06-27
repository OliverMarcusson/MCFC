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

## Under The Hood

`sleep_ticks` emits a generated continuation function and schedules it with Minecraft `schedule function ... <ticks>t`. Any following statements move into that continuation.
