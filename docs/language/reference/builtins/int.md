# `int`

```mcfc
int(value: nbt) -> int
int(value: float) -> int
```

Converts an NBT value or a [`float`](../types/float) to an `int`. Floats round down, so `int(-2.7)` is `-3`.

```mcfc
fn read(player: player_ref) -> void:
    let health = int(player.nbt.Health)
    mcf "say health=$(health)"
```

