# `mc`

Emits a literal Minecraft command.

```mcfc
fn load() -> void:
    mc "say loaded"
    mc "scoreboard objectives add health dummy"
```

`$(...)` is not interpolated in `mc`. For interpolation, use [`mcf`](./mcf).

