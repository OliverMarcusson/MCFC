# `give`

```mcfc
entity.give(item_id: string, count: int) -> void
entity.give(stack: item_def) -> void
```

Gives an item to the receiver.

```mcfc
fn reward(player: player_ref) -> void:
    player.give("minecraft:emerald", 3)

    let sword = item("minecraft:diamond_sword")
    sword.name = "Quest Blade"
    player.give(sword)
```

