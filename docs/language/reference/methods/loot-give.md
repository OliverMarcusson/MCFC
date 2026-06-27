# `loot_give`

```mcfc
entity.loot_give(table: string) -> void
```

Gives loot from a loot table to the receiver.

```mcfc
fn reward(player: player_ref) -> void:
    player.loot_give("minecraft:chests/simple_dungeon")
```

