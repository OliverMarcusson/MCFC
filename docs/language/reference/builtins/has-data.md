# `has_data`

```mcfc
has_data(path) -> bool
```

Tests whether a storage or NBT path has data.

```mcfc
fn inspect(player: player_ref) -> void:
    if has_data(player.nbt.CustomName):
        player.tellraw("has a custom name")
```

