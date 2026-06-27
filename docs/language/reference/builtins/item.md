# `item`

```mcfc
item(id: string) -> item_def
```

Creates an item builder.

```mcfc
fn reward(player: player_ref) -> void:
    let sword = item("minecraft:diamond_sword")
    sword.name = "Quest Blade"
    sword.count = 1
    player.give(sword)
```

