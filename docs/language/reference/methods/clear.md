# `clear`

```mcfc
entity.clear(item_id: string, count: int) -> void
```

Clears matching items from the receiver.

```mcfc
fn remove_tokens(player: player_ref) -> void:
    player.clear("minecraft:paper", 1)
```

