# `item_def`

Mutable item builder created with `item(...)`.

```mcfc
fn reward(player: player_ref) -> void:
    let sword = item("minecraft:diamond_sword")
    sword.name = "Quest Blade"
    player.give(sword)
```

