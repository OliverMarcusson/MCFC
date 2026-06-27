# `string`

```mcfc
string(value: nbt) -> string
```

Converts an NBT value to a `string`.

```mcfc
fn read(player: player_ref) -> void:
    let name = string(player.nbt.CustomName)
    player.tellraw(name)
```

