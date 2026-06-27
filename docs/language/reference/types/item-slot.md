# `item_slot`

Player inventory or hotbar slot surface.

```mcfc
fn inspect(player: player_ref) -> void:
    if player.inventory[3].exists:
        player.tellraw(player.inventory[3].id)
```

