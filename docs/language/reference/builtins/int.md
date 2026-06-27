# `int`

```mcfc
int(value: nbt) -> int
```

Converts an NBT value to an `int`.

```mcfc
fn read(player: player_ref) -> void:
    let health = int(player.nbt.Health)
    mcf "say health=$(health)"
```

