# `stopsound`

```mcfc
entity.stopsound(category: string, sound: string) -> void
```

Stops a sound for the receiver.

```mcfc
fn silence(player: player_ref) -> void:
    player.stopsound("master", "minecraft:entity.player.levelup")
```

Alias: [`stop_sound`](./stop-sound).

