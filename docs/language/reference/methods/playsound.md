# `playsound`

```mcfc
entity.playsound(sound: string, category: string) -> void
```

Plays a sound for the receiver.

```mcfc
fn chime(player: player_ref) -> void:
    player.playsound("minecraft:entity.player.levelup", "master")
```

