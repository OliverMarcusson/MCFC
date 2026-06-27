# `stop_sound`

```mcfc
entity.stop_sound(category: string, sound: string) -> void
```

Bukkit-style alias for [`stopsound`](./stopsound).

```mcfc
fn silence(player: player_ref) -> void:
    player.stop_sound("master", "minecraft:entity.player.levelup")
```

