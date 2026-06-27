# `nbt`

NBT-like storage path or payload value.

```mcfc
fn read(player: player_ref) -> void:
    let health = int(player.nbt.Health)
    mcf "say health=$(health)"
```

