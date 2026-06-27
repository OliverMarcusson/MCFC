# `mc`

Emits a literal Minecraft command.

```mcfc
fn load() -> void:
    mc "say loaded"
    mc "scoreboard objectives add health dummy"
```

`$(...)` is not interpolated in `mc`. For interpolation, use [`mcf`](./mcf).

## Under The Hood

The command string is copied directly into the generated `.mcfunction`. MCFC does not evaluate placeholders or allocate temporary storage for `mc`.
